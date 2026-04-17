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
    FormattedOutput, ToolCall, clean_tool_name, extract_edit_context,
    extract_tool_detail, format_result_text, format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{UIMessage, UISender};

pub struct ClaudeCodeAgent {
    config: AgentConfig,
    config_root: String,
}

impl ClaudeCodeAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn build_headless_args(&self, prompt: &str, phase: LlmPhase) -> Vec<String> {
        let mut args = vec![
            "--strict-mcp-config".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];

        if let Some(model) = self.config.models.get(phase.as_str()) {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if let Some(params) = self.config.params.get(phase.as_str()) {
            if params.get("dangerous").and_then(|v| v.as_bool()) == Some(true) {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }

        args
    }
}

fn parse_stream_line(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> Option<FormattedOutput> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let event: serde_json::Value = serde_json::from_str(line).ok()?;

    let event_type = event.get("type")?.as_str()?;

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
                        edit_context: None,
                    },
                    "Write" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                        edit_context: extract_edit_context(None, input.get("content").and_then(|v| v.as_str())),
                    },
                    "Edit" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                        edit_context: extract_edit_context(
                            input.get("old_string").and_then(|v| v.as_str()),
                            input.get("new_string").and_then(|v| v.as_str()),
                        ),
                    },
                    "Grep" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: Some(format!(
                            "\"{}\" in {}",
                            input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                            input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                        )),
                        edit_context: None,
                    },
                    "Glob" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("pattern").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        edit_context: None,
                    },
                    "Bash" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("command").and_then(|v| v.as_str()).map(|s| s.chars().take(120).collect()),
                        edit_context: None,
                    },
                    _ => ToolCall {
                        name: clean_tool_name(name),
                        path: None,
                        detail: Some(extract_tool_detail(&input)),
                        edit_context: None,
                    },
                };

                return Some(format_tool_call(&tool, phase, shown_highlights));
            }
        }
        return None;
    }

    if event_type == "result" {
        if let Some(result_text) = event.get("result").and_then(|r| r.as_str()) {
            return Some(FormattedOutput {
                text: format_result_text(result_text),
                persist: true,
            });
        }
    }

    None
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()> {
        let mut args = vec!["--output-format".to_string(), "stream-json".to_string()];

        if let Some(model) = self.config.models.get("work") {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if let Some(params) = self.config.params.get("work") {
            if params.get("dangerous").and_then(|v| v.as_bool()) == Some(true) {
                args.push("--dangerously-skip-permissions".to_string());
            }
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
        let args = self.build_headless_args(prompt, phase);

        let mut child = Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn claude")?;

        let stdout = child.stdout.take().context("No stdout")?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut shown_highlights = HashSet::new();

        let mut read_err: Option<anyhow::Error> = None;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(formatted) = parse_stream_line(&line, Some(phase), &mut shown_highlights) {
                        if formatted.text.is_empty() {
                            continue;
                        }
                        if formatted.persist {
                            let _ = tx.send(UIMessage::Persist {
                                agent_id: agent_id.to_string(),
                                text: formatted.text,
                            });
                        } else {
                            let _ = tx.send(UIMessage::Progress {
                                agent_id: agent_id.to_string(),
                                text: formatted.text,
                            });
                        }
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
        let _ = tx.send(UIMessage::AgentDone {
            agent_id: agent_id.to_string(),
        });

        if let Some(e) = read_err {
            return Err(e);
        }

        if !status.success() {
            anyhow::bail!("claude exited with code {:?}", status.code());
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

    #[test]
    fn parse_tool_use_read() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/foo/bar.md"}}]}}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(!formatted.persist);
        assert!(formatted.text.contains("Read"));
        assert!(formatted.text.contains("/foo/bar.md"));
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","result":"[ADDED] New entry — description"}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(formatted.persist);
        assert!(formatted.text.contains("ADDED"));
    }

    #[test]
    fn parse_highlight_write_memory() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/plan/memory.md","content":"stuff"}}]}}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        assert!(result.unwrap().persist);
    }

    #[test]
    fn parse_ignores_empty_lines() {
        let mut shown = HashSet::new();
        assert!(parse_stream_line("", None, &mut shown).is_none());
        assert!(parse_stream_line("   ", None, &mut shown).is_none());
    }
}
