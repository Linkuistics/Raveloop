// src/ui.rs
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal,
};
use indexmap::IndexMap;
use ratatui::{
    backend::CrosstermBackend,
    style::{Color, Modifier, Style as RatatuiStyle},
    text::{Line, Span as RatatuiSpan},
    widgets::{Paragraph, Widget},
    Frame, Terminal, TerminalOptions, Viewport,
};
use tokio::sync::{mpsc, oneshot};

use crate::format::{Intent, Span, Style, StyledLine};

// ── Semantic → ratatui mapping ────────────────────────────────────────────────
// This is the ONLY place ratatui sees format-layer styling. Keep it small and
// complete; don't scatter mapping logic elsewhere.

fn intent_color(intent: Intent) -> Color {
    match intent {
        Intent::Added => Color::Green,
        Intent::Removed => Color::Red,
        Intent::Changed => Color::Yellow,
        Intent::Meta => Color::Cyan,
    }
}

fn to_ratatui_style(style: Style) -> RatatuiStyle {
    let mut out = RatatuiStyle::default();
    if let Some(intent) = style.intent {
        out = out.fg(intent_color(intent));
    }
    if style.dim { out = out.add_modifier(Modifier::DIM); }
    if style.bold { out = out.add_modifier(Modifier::BOLD); }
    out
}

fn to_ratatui_span(span: &Span) -> RatatuiSpan<'static> {
    RatatuiSpan::styled(span.text.clone(), to_ratatui_style(span.style))
}

fn to_ratatui_line(line: &StyledLine) -> Line<'static> {
    Line::from(line.0.iter().map(to_ratatui_span).collect::<Vec<_>>())
}

/// All messages to the TUI flow through this enum.
pub enum UIMessage {
    // From agents
    Progress { agent_id: String, line: StyledLine },
    Persist { agent_id: String, lines: Vec<StyledLine> },
    AgentDone { agent_id: String },

    // From the phase loop — plain text; gets wrapped in StyledLine on ingress.
    Log(String),
    RegisterAgent { agent_id: String },
    Confirm { message: String, reply: oneshot::Sender<bool> },
    // `ack` closes to synchronously notify the caller that the TUI has
    // finished clearing the viewport and disabling raw mode. Required
    // because `invoke_interactive` spawns a child that inherits stdin —
    // if raw mode is still on, the child's ctrl-C is swallowed and its
    // TUI setup races with ratatui.
    Suspend { ack: oneshot::Sender<()> },
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

    pub fn register_agent(&self, agent_id: &str) {
        let _ = self.tx.send(UIMessage::RegisterAgent {
            agent_id: agent_id.to_string(),
        });
    }

    pub async fn confirm(&self, message: &str) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(UIMessage::Confirm {
            message: message.to_string(),
            reply: reply_tx,
        });
        reply_rx.await.unwrap_or(false)
    }

    pub async fn suspend(&self) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.tx.send(UIMessage::Suspend { ack: ack_tx });
        let _ = ack_rx.await;
    }

    pub fn resume(&self) {
        let _ = self.tx.send(UIMessage::Resume);
    }

    pub fn quit(&self) {
        let _ = self.tx.send(UIMessage::Quit);
    }
}

/// State for the inline live area.
///
/// Log/Persist lines are inserted into native scrollback via
/// `Terminal::insert_before` and are NOT buffered here — that gives users
/// real terminal scrollback. Progress and confirm live only in the viewport
/// and never touch scrollback, making them truly transient.
pub struct AppState {
    pub progress_groups: IndexMap<String, AgentProgress>,
    pub confirm_prompt: Option<ConfirmState>,
}

pub struct AgentProgress {
    pub progress: Option<StyledLine>,
    pub progress_at: Option<Instant>,
}

pub struct ConfirmState {
    pub message: String,
    pub reply: Option<oneshot::Sender<bool>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            progress_groups: IndexMap::new(),
            confirm_prompt: None,
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_agent_progress(&mut self, agent_id: &str) {
        if let Some(group) = self.progress_groups.get_mut(agent_id) {
            group.progress = None;
            group.progress_at = None;
        }
    }

    pub fn clear_all_progress(&mut self) {
        for group in self.progress_groups.values_mut() {
            group.progress = None;
            group.progress_at = None;
        }
    }

    /// The most-recently-updated agent with a progress line, if any.
    /// Used to pick which single agent to show in our 2-row viewport.
    fn active_progress(&self) -> Option<(&str, &AgentProgress)> {
        self.progress_groups
            .iter()
            .filter(|(_, g)| g.progress.is_some())
            .max_by_key(|(_, g)| g.progress_at)
            .map(|(id, g)| (id.as_str(), g))
    }

    /// Update state from non-scrollback messages. Log/Persist and lifecycle
    /// messages (Suspend/Resume/Quit) are handled by the runtime loop.
    pub fn handle_message(&mut self, msg: UIMessage) {
        match msg {
            UIMessage::RegisterAgent { agent_id } => {
                self.progress_groups.insert(agent_id, AgentProgress {
                    progress: None,
                    progress_at: None,
                });
            }
            UIMessage::Progress { agent_id, line } => {
                if let Some(group) = self.progress_groups.get_mut(&agent_id) {
                    group.progress = Some(line);
                    group.progress_at = Some(Instant::now());
                }
            }
            UIMessage::AgentDone { agent_id } => {
                self.progress_groups.shift_remove(&agent_id);
            }
            UIMessage::Confirm { message, reply } => {
                self.confirm_prompt = Some(ConfirmState {
                    message,
                    reply: Some(reply),
                });
            }
            _ => {}
        }
    }
}

/// Inline viewport height. One row — the live area answers a single
/// question: "what's running right now?" (tool call) or "what am I waiting
/// on?" (confirm). Phase headers, subagent identities, and persisted
/// summaries are already in scrollback; the viewport does not repeat them.
const VIEWPORT_HEIGHT: u16 = 1;

fn insert_plain(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    line: String,
) -> io::Result<()> {
    terminal.insert_before(1, |buf| {
        Paragraph::new(Line::raw(line)).render(buf.area, buf);
    })
}

fn insert_styled(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    line: &StyledLine,
) -> io::Result<()> {
    let rline = to_ratatui_line(line);
    terminal.insert_before(1, |buf| {
        Paragraph::new(rline).render(buf.area, buf);
    })
}

pub async fn run_tui(mut rx: mpsc::UnboundedReceiver<UIMessage>) -> Result<(), anyhow::Error> {
    terminal::enable_raw_mode()?;
    let stderr = io::stderr();
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions { viewport: Viewport::Inline(VIEWPORT_HEIGHT) },
    )?;

    let mut state = AppState::new();
    let mut suspended = false;

    loop {
        if !suspended {
            terminal.draw(|f| draw_live(f, &state))?;
        }

        if suspended {
            // While a child process (invoke_interactive) owns the terminal,
            // we must not touch stdin/stdout — crossterm's `event::poll`
            // races with the child for tty reads and can blank its TUI.
            // Wait for messages only. The only events that matter here are
            // Resume / Quit / queued Log writes (log writes are buffered
            // into ratatui's `insert_before` area; they'll paint once we
            // resume — writing now would corrupt the child's output).
            match rx.recv().await {
                Some(UIMessage::Quit) | None => break,
                Some(UIMessage::Resume) => {
                    let _ = writeln!(io::stderr());
                    let _ = io::stderr().flush();
                    terminal::enable_raw_mode()?;
                    let backend = CrosstermBackend::new(io::stderr());
                    terminal = Terminal::with_options(
                        backend,
                        TerminalOptions { viewport: Viewport::Inline(VIEWPORT_HEIGHT) },
                    )?;
                    suspended = false;
                }
                Some(msg) => state.handle_message(msg),
            }
            continue;
        }

        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(UIMessage::Quit) | None => break,
                    Some(UIMessage::Suspend { ack }) => {
                        // Hand the terminal over to the child process cleanly.
                        // `terminal.clear()` only clears ratatui's inline
                        // viewport, not any residual terminal state. Emit
                        // explicit resets so the child sees a clean tty:
                        //   - Exit alt screen (no-op if not entered)
                        //   - Disable bracketed paste
                        //   - Disable mouse capture (any mode)
                        //   - Reset SGR attributes
                        //   - Show cursor
                        use std::io::Write as _;
                        let mut err = io::stderr();
                        let _ = write!(err, "\x1b[?1049l\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[0m\x1b[?25h");
                        let _ = err.flush();
                        terminal.clear()?;
                        terminal::disable_raw_mode()?;
                        suspended = true;
                        let _ = ack.send(());
                    }
                    Some(UIMessage::Resume) => {
                        // The interactive child may end without a trailing
                        // newline, leaving the cursor mid-line over its last
                        // output. The old Terminal's cached `viewport_area`
                        // still points at the row the inline viewport
                        // occupied before suspend; in inline mode that row
                        // now overlaps the child's final visible line, so
                        // calling `terminal.clear()` (which jumps the cursor
                        // to `viewport_area.top()` and clears below) wipes
                        // the bottom of the child's output. Fix in two steps:
                        //   1. Emit a newline + flush so the cursor moves
                        //      onto a fresh row below the child's last byte.
                        //   2. Reconstruct the Terminal so its constructor
                        //      re-queries the cursor and places a fresh
                        //      viewport on the new row. This avoids any
                        //      clear() against the stale viewport_area.
                        let _ = writeln!(io::stderr());
                        let _ = io::stderr().flush();
                        terminal::enable_raw_mode()?;
                        let backend = CrosstermBackend::new(io::stderr());
                        terminal = Terminal::with_options(
                            backend,
                            TerminalOptions { viewport: Viewport::Inline(VIEWPORT_HEIGHT) },
                        )?;
                        suspended = false;
                    }
                    Some(UIMessage::Log(text)) => {
                        // A phase-level log invalidates every agent's
                        // in-flight tool status.
                        state.clear_all_progress();
                        for line in text.split('\n') {
                            insert_plain(&mut terminal, line.to_string())?;
                        }
                    }
                    Some(UIMessage::Persist { agent_id, lines }) => {
                        for line in &lines {
                            insert_styled(&mut terminal, line)?;
                        }
                        state.clear_agent_progress(&agent_id);
                    }
                    Some(msg) => state.handle_message(msg),
                }
            }
            // tokio::time::sleep (not spawn_blocking(event::poll)) because
            // the latter holds an OS thread inside crossterm's stdin epoll
            // until the timeout fires, even after select! drops its
            // JoinHandle. After Suspend the blocking thread keeps running
            // for up to 50ms, races the just-spawned child for the tty
            // (termios is global to the device, so once the child enables
            // raw mode our blocked event::poll wakes and reads bytes the
            // child needed). sleep() is properly cancellable: dropping it
            // cancels the timer-wheel registration, no thread, no race.
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if !suspended {
                    if let Ok(true) = event::poll(Duration::from_millis(0)) {
                        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
                            // Ctrl+C / Ctrl+D — raw mode swallows SIGINT, so we
                            // catch the keystroke ourselves. Restore the
                            // terminal and exit 130 (user-cancelled).
                            let is_cancel = modifiers.contains(KeyModifiers::CONTROL)
                                && matches!(code, KeyCode::Char('c') | KeyCode::Char('d'));
                            if is_cancel {
                                if let Some(mut confirm) = state.confirm_prompt.take() {
                                    if let Some(reply) = confirm.reply.take() {
                                        let _ = reply.send(false);
                                    }
                                }
                                let _ = terminal.clear();
                                let _ = terminal::disable_raw_mode();
                                eprintln!("\n  ✗  Cancelled.");
                                std::process::exit(130);
                            }

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

    terminal.clear()?;
    terminal::disable_raw_mode()?;
    Ok(())
}

/// Draws the 1-row inline viewport: confirm prompt if active, else the
/// most-recently-updated agent's tool-call progress line. Blank when idle.
fn draw_live(f: &mut Frame, state: &AppState) {
    let area = f.area();

    if let Some(ref confirm) = state.confirm_prompt {
        let line = Line::styled(
            format!("  ▶  {} [Y/n] ", confirm.message),
            RatatuiStyle::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        );
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    if let Some((_id, group)) = state.active_progress() {
        if let Some(ref progress) = group.progress {
            let mut spans: Vec<RatatuiSpan<'static>> = vec![
                RatatuiSpan::styled(
                    "      ".to_string(),
                    RatatuiStyle::default().add_modifier(Modifier::DIM),
                ),
            ];
            spans.extend(progress.0.iter().map(to_ratatui_span));
            if let Some(t) = group.progress_at {
                spans.push(RatatuiSpan::styled(
                    format!(" (+{:.1}s)", t.elapsed().as_secs_f32()),
                    RatatuiStyle::default().add_modifier(Modifier::DIM),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
        }
    }
    // Otherwise: nothing to render — the 1 row stays blank.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(line: &StyledLine) -> String {
        line.0.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn state_handles_agent_lifecycle() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "sub-B".to_string(),
        });
        assert!(state.progress_groups.contains_key("sub-B"));

        state.handle_message(UIMessage::Progress {
            agent_id: "sub-B".to_string(),
            line: StyledLine::plain("Read memory.yaml"),
        });
        let progress = state.progress_groups.get("sub-B").unwrap().progress.as_ref().unwrap();
        assert_eq!(flat(progress), "Read memory.yaml");

        state.handle_message(UIMessage::AgentDone {
            agent_id: "sub-B".to_string(),
        });
        assert!(!state.progress_groups.contains_key("sub-B"));
    }

    #[test]
    fn active_progress_picks_most_recent() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "a".to_string(),
        });
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "b".to_string(),
        });
        state.handle_message(UIMessage::Progress {
            agent_id: "a".to_string(),
            line: StyledLine::plain("Read /a"),
        });
        std::thread::sleep(std::time::Duration::from_millis(5));
        state.handle_message(UIMessage::Progress {
            agent_id: "b".to_string(),
            line: StyledLine::plain("Read /b"),
        });
        let (id, _) = state.active_progress().expect("some progress");
        assert_eq!(id, "b", "most-recently-updated agent wins");
    }

    #[test]
    fn clear_progress_helpers() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "a".to_string(),
        });
        state.handle_message(UIMessage::Progress {
            agent_id: "a".to_string(),
            line: StyledLine::plain("Read foo"),
        });
        assert!(state.progress_groups["a"].progress.is_some());

        state.clear_agent_progress("a");
        assert!(state.progress_groups["a"].progress.is_none());
        assert!(state.progress_groups["a"].progress_at.is_none());

        state.handle_message(UIMessage::Progress {
            agent_id: "a".to_string(),
            line: StyledLine::plain("Read bar"),
        });
        state.clear_all_progress();
        assert!(state.progress_groups["a"].progress.is_none());
    }

    #[tokio::test]
    async fn ui_confirm_roundtrip() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ui = UI::new(tx);

        let confirm_handle = tokio::spawn(async move {
            ui.confirm("Proceed?").await
        });

        if let Some(UIMessage::Confirm { reply, .. }) = rx.recv().await {
            reply.send(true).unwrap();
        }

        assert!(confirm_handle.await.unwrap());
    }
}
