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

        args.push(prompt.to_string());

        // Temporary diagnostic — dump args + env + tty + termios state to
        // /tmp/ravel-debug.log so we can see what's different between shell
        // spawn (works) and ravel-lite spawn (claude TUI doesn't render).
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/ravel-debug.log") {
            use std::io::Write;
            let _ = writeln!(f, "---- invoke_interactive @ {:?} ----", std::time::SystemTime::now());
            let _ = writeln!(f, "cwd={}", ctx.project_dir);
            let _ = writeln!(f, "args[0..{}]:", args.len());
            for (i, a) in args.iter().enumerate() {
                let disp = if a.len() > 120 { format!("{}... [truncated, len={}]", &a[..120], a.len()) } else { a.clone() };
                let _ = writeln!(f, "  [{i}] {disp}");
            }
            let _ = writeln!(f, "--- env ---");
            let mut env_pairs: Vec<(String, String)> = std::env::vars().collect();
            env_pairs.sort();
            for (k, v) in &env_pairs {
                let vd = if v.len() > 200 { format!("{}...[len={}]", &v[..200], v.len()) } else { v.clone() };
                let _ = writeln!(f, "  {k}={vd}");
            }
            let _ = writeln!(f, "--- tty ---");
            // Unsafe for isatty on raw fds
            unsafe {
                let _ = writeln!(f, "  isatty(stdin)={}", libc::isatty(0) != 0);
                let _ = writeln!(f, "  isatty(stdout)={}", libc::isatty(1) != 0);
                let _ = writeln!(f, "  isatty(stderr)={}", libc::isatty(2) != 0);
                let _ = writeln!(f, "  getpgrp()={}", libc::getpgrp());
                let _ = writeln!(f, "  tcgetpgrp(stdin)={}", libc::tcgetpgrp(0));
            }
            // termios snapshot — verifies disable_raw_mode actually restored
            // cooked-mode bits before the child inherits this tty. If
            // ICANON/ECHO/ISIG are off here the child gets a half-raw tty
            // and its TUI silently fails (and ctrl-C goes weird).
            let _ = writeln!(f, "--- termios(stdin) ---");
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                let rc = libc::tcgetattr(0, &mut t);
                let _ = writeln!(f, "  tcgetattr rc={rc}");
                let _ = writeln!(f, "  c_iflag=0x{:x} c_oflag=0x{:x} c_cflag=0x{:x} c_lflag=0x{:x}",
                    t.c_iflag, t.c_oflag, t.c_cflag, t.c_lflag);
                let on = |bit: libc::tcflag_t, mask: libc::tcflag_t| (bit & mask) != 0;
                let _ = writeln!(f, "  lflag: ICANON={} ECHO={} ECHOE={} ECHOK={} ISIG={} IEXTEN={}",
                    on(t.c_lflag, libc::ICANON),
                    on(t.c_lflag, libc::ECHO),
                    on(t.c_lflag, libc::ECHOE),
                    on(t.c_lflag, libc::ECHOK),
                    on(t.c_lflag, libc::ISIG),
                    on(t.c_lflag, libc::IEXTEN),
                );
                let _ = writeln!(f, "  iflag: ICRNL={} IXON={} BRKINT={} ISTRIP={}",
                    on(t.c_iflag, libc::ICRNL),
                    on(t.c_iflag, libc::IXON),
                    on(t.c_iflag, libc::BRKINT),
                    on(t.c_iflag, libc::ISTRIP),
                );
                let _ = writeln!(f, "  oflag: OPOST={} ONLCR={}",
                    on(t.c_oflag, libc::OPOST),
                    on(t.c_oflag, libc::ONLCR),
                );
            }
            let _ = writeln!(f, "spawning claude now...");
        }

        let status = std::process::Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn claude")?;

        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/ravel-debug.log") {
            use std::io::Write;
            let _ = writeln!(f, "claude exited: {:?}", status);
        }

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
