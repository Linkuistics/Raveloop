//! Shared post-spawn plumbing for streaming CLI agents (claude, pi).
//!
//! Argv construction and the agent-specific stream-JSON parsers stay in
//! the concrete `claude_code.rs` / `pi.rs` — their schemas differ. What
//! is identical between the two, and therefore lives here:
//!
//! * `StreamLineOutcome` (ignored / malformed / output) and its snippet
//!   helper, so format drift never silently disappears;
//! * the rolling stderr drain with a fixed byte cap;
//! * the stdout pump that fans parser outputs to `UIMessage::Progress`
//!   / `Persist` and surfaces parse failures as Persist warnings;
//! * the post-spawn dance (pump, wait, drain, `AgentDone`, exit-error
//!   shape), so a fix on one side cannot silently drift on the other;
//! * dispatch-subagent `PlanContext` construction.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::task::JoinHandle;

use crate::format::{FormattedOutput, Intent, Span, Style, StyledLine};
use crate::types::{LlmPhase, PlanContext};
use crate::ui::{UIMessage, UISender};

/// Rolling cap on stderr retained for surfacing in error messages.
/// When a run's stderr exceeds this, oldest bytes are discarded and a
/// one-shot Persist warning tells the user the tail they'll see on
/// failure is truncated.
pub const STDERR_BUFFER_CAP: usize = 4096;

/// Max bytes of a malformed stream line kept in the per-line warning.
pub const STREAM_SNIPPET_BYTES: usize = 200;

/// Outcome of interpreting one stream-JSON line from an agent.
///
/// `Ignored` means valid JSON but nothing to display (e.g. a system
/// event). `Malformed` means the line itself couldn't be parsed; the
/// pump warns with the snippet. Collapsing these with `Option` hides
/// format drift — the exact class of bug this enum guards against.
pub enum StreamLineOutcome {
    Output(FormattedOutput),
    Ignored,
    Malformed { snippet: String },
}

/// Per-agent stream-JSON parser signature. Takes a line, the current
/// phase (for tool-call highlighting), and a mutable set tracking
/// which highlights have already been emitted in this run.
pub type ParseLineFn = fn(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> StreamLineOutcome;

/// Truncate `raw` to at most `max_bytes` on a UTF-8 char boundary,
/// appending `…` when trimmed. Never slices mid-codepoint.
pub fn truncate_snippet(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &raw[..cut])
}

/// Yellow `⚠  …` TUI warning line. Matches the visual used by
/// `warn_if_project_tree_dirty` in `phase_loop`, rendered through
/// `Persist` because it originates from an agent.
pub fn warning_line(body: impl Into<String>) -> StyledLine {
    StyledLine(vec![Span::styled(
        format!("  ⚠  {}", body.into()),
        Style::bold_intent(Intent::Changed),
    )])
}

/// Spawn a task that drains the child's stderr into a rolling
/// STDERR_BUFFER_CAP buffer. Returns the captured tail on completion.
/// On the first overflow, emits a one-shot Persist warning naming the
/// agent so the user knows a later error tail is truncated.
pub fn spawn_stderr_drain(
    stderr: ChildStderr,
    tx: UISender,
    agent_id: String,
    agent_name: &'static str,
) -> JoinHandle<String> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut buf = String::new();
        let mut overflow_warned = false;
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
            if buf.len() > STDERR_BUFFER_CAP {
                let cut = buf.len() - STDERR_BUFFER_CAP;
                buf.drain(..cut);
                if !overflow_warned {
                    overflow_warned = true;
                    let _ = tx.send(UIMessage::Persist {
                        agent_id: agent_id.clone(),
                        lines: vec![warning_line(format!(
                            "{agent_name} stderr exceeded {STDERR_BUFFER_CAP}-byte buffer — earlier lines dropped"
                        ))],
                    });
                }
            }
        }
        buf
    })
}

/// Read lines from `stdout`, dispatch each parsed outcome to `tx`, and
/// return any I/O error encountered (so the caller can still wait on
/// the child and drain stderr before propagating). Malformed lines
/// surface as Persist warnings naming the agent.
pub async fn pump_stdout_to_ui(
    stdout: ChildStdout,
    phase: LlmPhase,
    agent_id: &str,
    agent_name: &'static str,
    tx: &UISender,
    parse_line: ParseLineFn,
) -> Option<anyhow::Error> {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut shown_highlights = HashSet::new();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match parse_line(&line, Some(phase), &mut shown_highlights) {
                StreamLineOutcome::Output(formatted) => {
                    if formatted.is_empty() {
                        continue;
                    }
                    if formatted.persist {
                        let _ = tx.send(UIMessage::Persist {
                            agent_id: agent_id.to_string(),
                            lines: formatted.lines,
                        });
                    } else if let Some(line) = formatted.lines.into_iter().next() {
                        let _ = tx.send(UIMessage::Progress {
                            agent_id: agent_id.to_string(),
                            line,
                        });
                    }
                }
                StreamLineOutcome::Malformed { snippet } => {
                    let _ = tx.send(UIMessage::Persist {
                        agent_id: agent_id.to_string(),
                        lines: vec![warning_line(format!(
                            "{agent_name} stream-JSON parse failed — dropping line: {snippet}"
                        ))],
                    });
                }
                StreamLineOutcome::Ignored => {}
            },
            Ok(None) => return None,
            Err(e) => return Some(e.into()),
        }
    }
}

/// Drive a just-spawned agent child to completion: pump stdout, wait
/// on the process, surface the stderr tail on non-zero exit. Sends
/// `AgentDone` unconditionally before returning.
pub async fn run_streaming_child(
    mut child: Child,
    phase: LlmPhase,
    agent_id: &str,
    agent_name: &'static str,
    tx: UISender,
    parse_line: ParseLineFn,
) -> Result<()> {
    let stdout = child.stdout.take().context("No stdout")?;
    let stderr = child.stderr.take().context("No stderr")?;

    let stderr_task = spawn_stderr_drain(stderr, tx.clone(), agent_id.to_string(), agent_name);
    let read_err = pump_stdout_to_ui(stdout, phase, agent_id, agent_name, &tx, parse_line).await;
    let status = child.wait().await?;
    let stderr_tail = stderr_task.await.unwrap_or_default();

    let _ = tx.send(UIMessage::AgentDone {
        agent_id: agent_id.to_string(),
    });

    if let Some(e) = read_err {
        return Err(e);
    }
    if !status.success() {
        let trimmed = stderr_tail.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{agent_name} exited with code {:?}", status.code());
        }
        anyhow::bail!(
            "{agent_name} exited with code {:?}\n--- stderr ---\n{trimmed}",
            status.code()
        );
    }
    Ok(())
}

/// Build a `PlanContext` for a subagent dispatched to `target_plan`.
/// Resolves `project_dir` via `git::project_root_for_plan` and derives
/// `dev_root` as the grandparent of the plan directory.
pub fn build_dispatch_plan_context(
    target_plan: &str,
    config_root: String,
) -> Result<PlanContext> {
    let project_dir = crate::git::project_root_for_plan(Path::new(target_plan))?;
    Ok(PlanContext {
        plan_dir: target_plan.to_string(),
        project_dir,
        dev_root: Path::new(target_plan)
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        related_plans: String::new(),
        config_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_snippet_passes_short_unchanged() {
        assert_eq!(truncate_snippet("short", STREAM_SNIPPET_BYTES), "short");
        assert_eq!(truncate_snippet("", STREAM_SNIPPET_BYTES), "");
    }

    #[test]
    fn truncate_snippet_is_utf8_safe_at_cut_point() {
        // >STREAM_SNIPPET_BYTES string with multibyte chars straddling the
        // cut point; must never slice mid-codepoint.
        let mut s = String::new();
        for _ in 0..50 {
            s.push_str("café ");
        }
        assert!(s.len() > STREAM_SNIPPET_BYTES);
        let truncated = truncate_snippet(&s, STREAM_SNIPPET_BYTES);
        assert!(truncated.ends_with('…'));
        assert!(truncated.len() <= STREAM_SNIPPET_BYTES + '…'.len_utf8());
    }
}
