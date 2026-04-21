use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;

use super::Agent;
use super::common::{
    STREAM_SNIPPET_BYTES, StreamLineOutcome, build_dispatch_plan_context, run_streaming_child,
    truncate_snippet,
};
use crate::config::load_tokens;
use crate::format::{
    FormattedOutput, ToolCall, clean_tool_name, extract_tool_detail, format_result_text,
    format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::UISender;

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
        // `--output-format stream-json` only applies with `-p`/`--print`
        // (per `claude --help`). In interactive mode it puts claude into
        // a hybrid state where the TUI silently fails to render. Leave
        // interactive output to claude's default TUI.
        let mut args = vec![
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

        // WORKAROUND (2026-04-21): claude's interactive TUI silently
        // fails to render when claude is spawned by ravel-lite, even
        // though the EXACT same argv, cwd, env, prompt, and binary
        // path render normally when invoked from a bash shell. Adding
        // `--debug-file <path>` (which implicitly turns on `--debug`
        // mode in claude — see `claude --help`) reliably masks the
        // bug. We do not understand why; investigating took hours and
        // ruled out: termios state, isatty on stdin/stdout/stderr,
        // process-group leadership, foreground tty ownership, signal
        // mask, signal handlers, O_NONBLOCK, args content, env vars,
        // claude version (2.1.113 / 2.1.114 / 2.1.116), prompt size,
        // the cmux wrapper, and earlier ravel-lite versions (commit
        // 91ad991 from 2026-04-19 also reproduces the bug).
        //
        // The same shell command works:
        //     cd <project> && claude --add-dir <plan> "<prompt>"
        // but ravel-lite's std::process::Command spawn of the same
        // does not. Difference must be in inherited process state we
        // could not isolate without dtrace-level instrumentation.
        //
        // TRY REMOVING THIS in the future when claude is updated past
        // 2.1.116 — delete the next two `args.push` lines and verify
        // the Work-phase TUI still renders. If it does, the upstream
        // issue is fixed and this workaround is no longer needed.
        args.push("--debug-file".to_string());
        args.push("/tmp/claude-debug.log".to_string());

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

        let child = Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn claude")?;

        run_streaming_child(child, phase, agent_id, "claude", tx, parse_stream_line).await
    }

    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let ctx = build_dispatch_plan_context(target_plan, self.config_root.clone())?;
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
}
