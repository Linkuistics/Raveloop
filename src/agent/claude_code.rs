use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::Agent;
use crate::config::load_tokens;
use crate::format::{
    FormattedOutput, Intent, Span, Style, StyledLine, ToolCall, clean_tool_name,
    extract_tool_detail, format_result_text, format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{UIMessage, UISender};

/// Maximum bytes of a malformed stream line retained in the per-line
/// warning. The full line is dropped; the snippet is enough to diagnose
/// format drift without flooding scrollback when claude produces many
/// bad lines in a row.
const STREAM_SNIPPET_BYTES: usize = 200;

/// Rolling cap on the stderr tail buffer. When a run exceeds this, the
/// oldest bytes are discarded and a one-shot warning is sent so the user
/// knows the error message they're about to see (on failure) is truncated.
const STDERR_BUFFER_CAP: usize = 4096;

/// Outcome of trying to interpret one stream-JSON line from claude.
///
/// Distinguishes "valid JSON but nothing to display" (Ignored) from
/// "couldn't parse" (Malformed) so the caller can warn on the latter.
/// The old `Option<FormattedOutput>` collapsed both into `None`, which
/// is how claude stream-format drift becomes invisible in the TUI.
enum StreamLineOutcome {
    Output(FormattedOutput),
    Ignored,
    Malformed { snippet: String },
}

/// Truncates on a UTF-8 char boundary so a multibyte code point at the
/// cut point never panics. Appends `…` when the input was trimmed.
fn truncate_snippet(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &raw[..cut])
}

/// Builds a yellow `⚠  …` warning line for the TUI scrollback. Matches
/// the existing warning visual used by `warn_if_project_tree_dirty` in
/// `phase_loop`, but renders through `Persist` because it's emitted from
/// an agent rather than the phase-loop thread.
fn warning_line(body: impl Into<String>) -> StyledLine {
    StyledLine(vec![Span::styled(
        format!("  ⚠  {}", body.into()),
        Style::bold_intent(Intent::Changed),
    )])
}

pub struct ClaudeCodeAgent {
    config: AgentConfig,
    config_root: String,
}

impl ClaudeCodeAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn is_dangerous(&self, phase: &str) -> bool {
        self.config.params.get(phase)
            .and_then(|p| p.get("dangerous"))
            .and_then(|v| v.as_bool())
            == Some(true)
    }

    fn build_headless_args(&self, prompt: &str, phase: LlmPhase, plan_dir: &str) -> Vec<String> {
        let mut args = vec![
            "--strict-mcp-config".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--add-dir".to_string(),
            plan_dir.to_string(),
        ];

        if let Some(model) = self.config.models.get(phase.as_str()) {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if self.is_dangerous(phase.as_str()) {
            args.push("--dangerously-skip-permissions".to_string());
        }

        args
    }
}

fn parse_stream_line(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> StreamLineOutcome {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamLineOutcome::Ignored;
    }

    let event: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            return StreamLineOutcome::Malformed {
                snippet: truncate_snippet(trimmed, STREAM_SNIPPET_BYTES),
            };
        }
    };

    let Some(event_type) = event.get("type").and_then(|t| t.as_str()) else {
        return StreamLineOutcome::Ignored;
    };

    if event_type == "assistant" {
        if let Some(content) = event.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                    continue;
                }
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);

                let tool = match name {
                    "Read" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                    },
                    "Write" | "Edit" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                    },
                    "Grep" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: Some(format!(
                            "\"{}\" in {}",
                            input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                            input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                        )),
                    },
                    "Glob" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("pattern").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    },
                    "Bash" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("command").and_then(|v| v.as_str()).map(|s| s.chars().take(120).collect()),
                    },
                    _ => ToolCall {
                        name: clean_tool_name(name),
                        path: None,
                        detail: Some(extract_tool_detail(&input)),
                    },
                };

                return StreamLineOutcome::Output(format_tool_call(&tool, phase, shown_highlights));
            }
        }
        return StreamLineOutcome::Ignored;
    }

    if event_type == "result" {
        if let Some(result_text) = event.get("result").and_then(|r| r.as_str()) {
            return StreamLineOutcome::Output(FormattedOutput {
                lines: format_result_text(result_text),
                persist: true,
            });
        }
    }

    StreamLineOutcome::Ignored
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()> {
        let mut args = vec![
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--add-dir".to_string(),
            ctx.plan_dir.clone(),
        ];

        if let Some(model) = self.config.models.get("work") {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if self.is_dangerous("work") {
            args.push("--dangerously-skip-permissions".to_string());
        }

        args.push(prompt.to_string());

        let status = std::process::Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn claude")?;

        if !status.success() {
            anyhow::bail!("claude exited with code {:?}", status.code());
        }
        Ok(())
    }

    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let args = self.build_headless_args(prompt, phase, &ctx.plan_dir);

        let mut child = Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn claude")?;

        let stdout = child.stdout.take().context("No stdout")?;
        let stderr = child.stderr.take().context("No stderr")?;

        // Drain stderr concurrently so it never blocks the child;
        // retain the last STDERR_BUFFER_CAP bytes so failures can be
        // surfaced in the error. On the first overflow, emit a one-shot
        // Persist warning so the user knows any error tail they later
        // see is the tail, not the head.
        let overflow_tx = tx.clone();
        let overflow_agent_id = agent_id.to_string();
        let stderr_task = tokio::spawn(async move {
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
                        let _ = overflow_tx.send(UIMessage::Persist {
                            agent_id: overflow_agent_id.clone(),
                            lines: vec![warning_line(format!(
                                "claude stderr exceeded {STDERR_BUFFER_CAP}-byte buffer — earlier lines dropped"
                            ))],
                        });
                    }
                }
            }
            buf
        });

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut shown_highlights = HashSet::new();

        let mut read_err: Option<anyhow::Error> = None;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    match parse_stream_line(&line, Some(phase), &mut shown_highlights) {
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
                                    "claude stream-JSON parse failed — dropping line: {snippet}"
                                ))],
                            });
                        }
                        StreamLineOutcome::Ignored => {}
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    read_err = Some(e.into());
                    break;
                }
            }
        }

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
                anyhow::bail!("claude exited with code {:?}", status.code());
            }
            anyhow::bail!(
                "claude exited with code {:?}\n--- stderr ---\n{trimmed}",
                status.code()
            );
        }
        Ok(())
    }

    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let project_dir = crate::git::find_project_root(Path::new(target_plan))?;

        let ctx = PlanContext {
            plan_dir: target_plan.to_string(),
            project_dir,
            dev_root: Path::new(target_plan)
                .parent().and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            related_plans: String::new(),
            config_root: self.config_root.clone(),
        };

        self.invoke_headless(prompt, &ctx, LlmPhase::Triage, agent_id, tx).await
    }

    fn tokens(&self) -> HashMap<String, String> {
        load_tokens(Path::new(&self.config_root), "claude-code")
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(formatted: &FormattedOutput) -> String {
        formatted.lines.iter()
            .map(|l| l.0.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn expect_output(outcome: StreamLineOutcome) -> FormattedOutput {
        match outcome {
            StreamLineOutcome::Output(f) => f,
            StreamLineOutcome::Ignored => panic!("expected Output, got Ignored"),
            StreamLineOutcome::Malformed { snippet } => {
                panic!("expected Output, got Malformed({snippet})")
            }
        }
    }

    #[test]
    fn parse_tool_use_read() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/foo/bar.md"}}]}}"#;
        let mut shown = HashSet::new();
        let formatted = expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(!formatted.persist);
        let text = flat(&formatted);
        assert!(text.contains("Read"));
        assert!(text.contains("/foo/bar.md"));
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","result":"[ADDED] New entry — description"}"#;
        let mut shown = HashSet::new();
        let formatted = expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(formatted.persist);
        assert!(flat(&formatted).contains("ADDED"));
    }

    #[test]
    fn parse_highlight_write_memory() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/plan/memory.md","content":"stuff"}}]}}"#;
        let mut shown = HashSet::new();
        assert!(expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown)).persist);
    }

    #[test]
    fn parse_ignores_empty_lines() {
        let mut shown = HashSet::new();
        assert!(matches!(
            parse_stream_line("", None, &mut shown),
            StreamLineOutcome::Ignored
        ));
        assert!(matches!(
            parse_stream_line("   ", None, &mut shown),
            StreamLineOutcome::Ignored
        ));
    }

    #[test]
    fn parse_unhandled_event_type_is_ignored() {
        // Valid JSON but nothing we display. Must NOT be classified as Malformed
        // — otherwise every system event would trigger a warning.
        let mut shown = HashSet::new();
        assert!(matches!(
            parse_stream_line(r#"{"type":"system","subtype":"init"}"#, None, &mut shown),
            StreamLineOutcome::Ignored
        ));
    }

    #[test]
    fn parse_malformed_json_surfaces_snippet() {
        // This is the scenario that used to silently disappear. The caller
        // now gets a snippet of the bad line so it can warn the user.
        let mut shown = HashSet::new();
        let outcome = parse_stream_line("this is not json", None, &mut shown);
        let StreamLineOutcome::Malformed { snippet } = outcome else {
            panic!("expected Malformed");
        };
        assert_eq!(snippet, "this is not json");
    }

    #[test]
    fn malformed_snippet_is_bounded_and_utf8_safe() {
        // Generate a >STREAM_SNIPPET_BYTES string containing multibyte chars
        // at the cut point to verify we never slice mid-codepoint.
        let mut s = String::new();
        for _ in 0..50 {
            s.push_str("café "); // 5 bytes each (é is 2 bytes)
        }
        assert!(s.len() > STREAM_SNIPPET_BYTES);
        let truncated = truncate_snippet(&s, STREAM_SNIPPET_BYTES);
        assert!(truncated.ends_with('…'));
        assert!(truncated.len() <= STREAM_SNIPPET_BYTES + '…'.len_utf8());
    }

    #[test]
    fn truncate_snippet_passes_short_inputs_unchanged() {
        assert_eq!(truncate_snippet("short", STREAM_SNIPPET_BYTES), "short");
        assert_eq!(truncate_snippet("", STREAM_SNIPPET_BYTES), "");
    }
}
