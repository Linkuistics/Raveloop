// src/ui.rs
use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use indexmap::IndexMap;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use tokio::sync::{mpsc, oneshot};

use crate::types::StatusInfo;

/// All messages to the TUI flow through this enum.
pub enum UIMessage {
    // From agents
    Progress { agent_id: String, text: String },
    Persist { agent_id: String, text: String },
    AgentDone { agent_id: String },

    // From the phase loop
    Log(String),
    RegisterAgent { agent_id: String, header: String },
    SetStatus(StatusInfo),
    Confirm { message: String, reply: oneshot::Sender<bool> },
    Suspend,
    Resume,
    Quit,
}

pub type UISender = mpsc::UnboundedSender<UIMessage>;

/// Handle for sending messages to the TUI.
/// Cloneable — the phase loop and agents all hold copies.
#[derive(Clone)]
pub struct UI {
    tx: UISender,
}

impl UI {
    pub fn new(tx: UISender) -> Self {
        Self { tx }
    }

    pub fn sender(&self) -> UISender {
        self.tx.clone()
    }

    pub fn log(&self, text: &str) {
        let _ = self.tx.send(UIMessage::Log(text.to_string()));
    }

    pub fn register_agent(&self, agent_id: &str, header: &str) {
        let _ = self.tx.send(UIMessage::RegisterAgent {
            agent_id: agent_id.to_string(),
            header: header.to_string(),
        });
    }

    pub fn clear_agent(&self, agent_id: &str) {
        let _ = self.tx.send(UIMessage::AgentDone {
            agent_id: agent_id.to_string(),
        });
    }

    pub fn set_status(&self, status: StatusInfo) {
        let _ = self.tx.send(UIMessage::SetStatus(status));
    }

    pub async fn confirm(&self, message: &str) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(UIMessage::Confirm {
            message: message.to_string(),
            reply: reply_tx,
        });
        reply_rx.await.unwrap_or(false)
    }

    pub fn suspend(&self) {
        let _ = self.tx.send(UIMessage::Suspend);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(UIMessage::Resume);
    }

    pub fn quit(&self) {
        let _ = self.tx.send(UIMessage::Quit);
    }
}

/// State for the TUI — used by the renderer (Task 12).
pub struct AppState {
    pub log_lines: Vec<String>,
    pub progress_groups: IndexMap<String, AgentProgress>,
    pub status: Option<StatusInfo>,
    pub confirm_prompt: Option<ConfirmState>,
}

pub struct AgentProgress {
    pub header: String,
    pub progress: Option<String>,
}

pub struct ConfirmState {
    pub message: String,
    pub reply: Option<oneshot::Sender<bool>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            log_lines: Vec::new(),
            progress_groups: IndexMap::new(),
            status: None,
            confirm_prompt: None,
        }
    }

    /// Process a UIMessage, updating state accordingly.
    pub fn handle_message(&mut self, msg: UIMessage) {
        match msg {
            UIMessage::Log(text) => {
                // Split multi-line log entries into individual lines
                for line in text.lines() {
                    self.log_lines.push(line.to_string());
                }
            }
            UIMessage::RegisterAgent { agent_id, header } => {
                self.progress_groups.insert(agent_id, AgentProgress {
                    header,
                    progress: None,
                });
            }
            UIMessage::Progress { agent_id, text } => {
                if let Some(group) = self.progress_groups.get_mut(&agent_id) {
                    group.progress = Some(text);
                }
            }
            UIMessage::Persist { agent_id: _, text } => {
                for line in text.lines() {
                    self.log_lines.push(line.to_string());
                }
            }
            UIMessage::AgentDone { agent_id } => {
                self.progress_groups.shift_remove(&agent_id);
            }
            UIMessage::SetStatus(status) => {
                self.status = Some(status);
            }
            UIMessage::Confirm { message, reply } => {
                self.confirm_prompt = Some(ConfirmState {
                    message,
                    reply: Some(reply),
                });
            }
            UIMessage::Suspend | UIMessage::Resume | UIMessage::Quit => {
                // Handled by the TUI event loop, not state
            }
        }
    }
}

pub async fn run_tui(mut rx: mpsc::UnboundedReceiver<UIMessage>) -> Result<(), anyhow::Error> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stderr();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new();
    let mut suspended = false;

    loop {
        if !suspended {
            terminal.draw(|f| draw_ui(f, &state))?;
        }

        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(UIMessage::Quit) | None => break,
                    Some(UIMessage::Suspend) => {
                        suspended = true;
                        terminal::disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    }
                    Some(UIMessage::Resume) => {
                        terminal::enable_raw_mode()?;
                        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                        terminal.clear()?;
                        suspended = false;
                    }
                    Some(msg) => {
                        state.handle_message(msg);
                    }
                }
            }
            _ = tokio::task::spawn_blocking(|| {
                event::poll(Duration::from_millis(50)).ok();
            }) => {
                if !suspended {
                    if let Ok(true) = event::poll(Duration::from_millis(0)) {
                        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
                            if let Some(ref mut confirm) = state.confirm_prompt {
                                let answer = match code {
                                    KeyCode::Char('n') | KeyCode::Char('N') => false,
                                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => true,
                                    _ => continue,
                                };
                                if let Some(reply) = confirm.reply.take() {
                                    let _ = reply.send(answer);
                                }
                                state.confirm_prompt = None;
                            }
                        }
                    }
                }
            }
        }
    }

    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn draw_ui(f: &mut Frame, state: &AppState) {
    let area = f.area();

    let progress_height = if state.progress_groups.is_empty() {
        0
    } else {
        state.progress_groups.len() as u16 * 2
    };

    let confirm_height = if state.confirm_prompt.is_some() { 2 } else { 0 };
    let live_height = progress_height + confirm_height;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(live_height),
            Constraint::Length(1),
        ])
        .split(area);

    let log_text: Vec<Line> = state.log_lines.iter()
        .map(|line| Line::raw(line.as_str()))
        .collect();
    let log_paragraph = Paragraph::new(log_text)
        .wrap(Wrap { trim: false })
        .scroll((
            state.log_lines.len().saturating_sub(chunks[0].height as usize) as u16,
            0,
        ));
    f.render_widget(log_paragraph, chunks[0]);

    if !state.progress_groups.is_empty() || state.confirm_prompt.is_some() {
        let mut progress_lines: Vec<Line> = Vec::new();

        for (_id, group) in &state.progress_groups {
            progress_lines.push(Line::raw(&group.header));
            if let Some(ref progress) = group.progress {
                progress_lines.push(Line::styled(
                    format!("      {progress}"),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            } else {
                progress_lines.push(Line::raw(""));
            }
        }

        if let Some(ref confirm) = state.confirm_prompt {
            progress_lines.push(Line::raw(""));
            progress_lines.push(Line::styled(
                format!("  ▶  {} [Y/n] ", confirm.message),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        let progress_paragraph = Paragraph::new(progress_lines);
        f.render_widget(progress_paragraph, chunks[1]);
    }

    let status_text = if let Some(ref status) = state.status {
        format!(
            " {} · {} · {} · {}{}",
            status.project,
            status.plan,
            status.phase,
            status.agent,
            status.cycle.map(|c| format!(" · cycle {c}")).unwrap_or_default()
        )
    } else {
        " raveloop".to_string()
    };

    let status_bar = Paragraph::new(Line::styled(
        status_text,
        Style::default().add_modifier(Modifier::DIM),
    ))
    .block(Block::default().borders(Borders::TOP));
    f.render_widget(status_bar, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_handles_log() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::Log("hello\nworld".to_string()));
        assert_eq!(state.log_lines, vec!["hello", "world"]);
    }

    #[test]
    fn state_handles_agent_lifecycle() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "sub-B".to_string(),
            header: "→ child: sub-B".to_string(),
        });
        assert!(state.progress_groups.contains_key("sub-B"));

        state.handle_message(UIMessage::Progress {
            agent_id: "sub-B".to_string(),
            text: "Read memory.md".to_string(),
        });
        assert_eq!(
            state.progress_groups.get("sub-B").unwrap().progress.as_deref(),
            Some("Read memory.md")
        );

        state.handle_message(UIMessage::AgentDone {
            agent_id: "sub-B".to_string(),
        });
        assert!(!state.progress_groups.contains_key("sub-B"));
    }

    #[test]
    fn state_handles_persist() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::Persist {
            agent_id: "main".to_string(),
            text: "★ Updating memory".to_string(),
        });
        assert_eq!(state.log_lines, vec!["★ Updating memory"]);
    }

    #[tokio::test]
    async fn ui_confirm_roundtrip() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ui = UI::new(tx);

        let confirm_handle = tokio::spawn(async move {
            ui.confirm("Proceed?").await
        });

        // Receive the confirm message and reply
        if let Some(UIMessage::Confirm { reply, .. }) = rx.recv().await {
            reply.send(true).unwrap();
        }

        assert!(confirm_handle.await.unwrap());
    }
}
