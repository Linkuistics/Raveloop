use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
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

// Dotall flag so `.` matches newlines in the body capture group.
static FRONTMATTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)^---\n(.*?)\n---\n(.*)$").expect("valid frontmatter regex")
});

// ── Stream parser ─────────────────────────────────────────────────────────────

fn parse_pi_stream_line(
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

    if event_type == "tool_execution_start" {
        let name = event.get("tool_name")?.as_str().unwrap_or("");
        let input = event.get("tool_input").cloned().unwrap_or(serde_json::Value::Null);

        let tool = match name {
            "read" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
                edit_context: None,
            },
            "write" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
                edit_context: extract_edit_context(None, input.get("content").and_then(|v| v.as_str())),
            },
            "edit" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
                edit_context: extract_edit_context(
                    input.get("old_string").and_then(|v| v.as_str()),
                    input.get("new_string").and_then(|v| v.as_str()),
                ),
            },
            "grep" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: Some(format!(
                    "\"{}\" in {}",
                    input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                    input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                )),
                edit_context: None,
            },
            "find" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: input.get("pattern").or(input.get("glob"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                edit_context: None,
            },
            "bash" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: input.get("command").and_then(|v| v.as_str())
                    .map(|s| s.chars().take(120).collect()),
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

    if event_type == "tool_execution_end" {
        if event.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            return Some(FormattedOutput {
                text: "  \x1b[31m✗  tool error\x1b[0m".to_string(),
                persist: true,
            });
        }
        return None;
    }

    if event_type == "message_end" {
        if let Some(content) = event.get("content").and_then(|c| c.as_array()) {
            let text: String = content.iter()
                .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Some(FormattedOutput {
                    text: format_result_text(&text),
                    persist: true,
                });
            }
        }
    }

    None
}

// ── Struct + helpers ──────────────────────────────────────────────────────────

pub struct PiAgent {
    config: AgentConfig,
    config_root: String,
}

impl PiAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn load_prompt_file(&self, name: &str, ctx: &PlanContext) -> Result<String> {
        let path = Path::new(&self.config_root).join("agents/pi/prompts").join(name);
        let mut content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        content = content.replace("{{PROJECT}}", &ctx.project_dir);
        content = content.replace("{{DEV_ROOT}}", &ctx.dev_root);
        content = content.replace("{{PLAN}}", &ctx.plan_dir);
        Ok(content)
    }

    fn build_headless_args(&self, prompt: &str, phase: LlmPhase, system_prompt: &str) -> Vec<String> {
        let mut args = vec![
            "--no-session".to_string(),
            "--append-system-prompt".to_string(),
            system_prompt.to_string(),
            "--provider".to_string(),
            self.config.provider.clone().unwrap_or_else(|| "anthropic".to_string()),
        ];
        if let Some(model) = self.config.models.get(phase.as_str()) {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }
        args.extend([
            "--mode".to_string(),
            "json".to_string(),
            "-p".to_string(),
            prompt.to_string(),
        ]);
        if let Some(thinking) = self.config.thinking.get(phase.as_str()) {
            if !thinking.is_empty() {
                args.extend(["--thinking".to_string(), thinking.clone()]);
            }
        }
        args
    }
}

// ── Agent impl ────────────────────────────────────────────────────────────────

#[async_trait]
impl Agent for PiAgent {
    async fn invoke_interactive(&self, prompt: &str, ctx: &PlanContext) -> Result<()> {
        let system_prompt = self.load_prompt_file("system-prompt.md", ctx)?;
        let memory_prompt = self.load_prompt_file("memory-prompt.md", ctx)?;
        let full = format!("{}\n\n{}\n\n{}", system_prompt, memory_prompt, prompt);

        let mut args = vec![
            "--no-session".to_string(),
            "--append-system-prompt".to_string(),
            full,
            "--provider".to_string(),
            self.config.provider.clone().unwrap_or_else(|| "anthropic".to_string()),
        ];

        if let Some(model) = self.config.models.get("work") {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if let Some(thinking) = self.config.thinking.get("work") {
            if !thinking.is_empty() {
                args.extend(["--thinking".to_string(), thinking.clone()]);
            }
        }

        let status = std::process::Command::new("pi")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn pi")?;

        if !status.success() {
            anyhow::bail!("pi exited with code {:?}", status.code());
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
        let system_prompt = self.load_prompt_file("system-prompt.md", ctx)?;
        let args = self.build_headless_args(prompt, phase, &system_prompt);

        let mut child = Command::new("pi")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn pi")?;

        let stdout = child.stdout.take().context("No stdout")?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut shown_highlights = HashSet::new();

        let mut read_err: Option<anyhow::Error> = None;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(formatted) = parse_pi_stream_line(&line, Some(phase), &mut shown_highlights) {
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
            anyhow::bail!("pi exited with code {:?}", status.code());
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
        load_tokens(Path::new(&self.config_root), "pi").unwrap_or_default()
    }

    async fn setup(&self, ctx: &PlanContext) -> Result<()> {
        // 1. Check pi is installed.
        let which = std::process::Command::new("which")
            .arg("pi")
            .output();
        match which {
            Ok(out) if out.status.success() => {}
            _ => anyhow::bail!(
                "pi is not installed. Install it from https://pi.ai or via your package manager."
            ),
        }

        // 2. Warn if ANTHROPIC_API_KEY is unset (non-fatal).
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            eprintln!("Warning: ANTHROPIC_API_KEY is not set — pi may fail to authenticate.");
        }

        // 3. Ensure the pi-subagent extension is installed.
        let settings_path = dirs::home_dir()
            .context("Cannot determine home directory")?
            .join(".pi/agent/settings.json");

        let extension_installed = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path)
                .with_context(|| format!("Failed to read {}", settings_path.display()))?;
            let val: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "Warning: {} contains invalid JSON ({e}); treating pi-subagent as not installed.",
                        settings_path.display()
                    );
                    serde_json::Value::Null
                }
            };
            val.get("packages")
                .and_then(|p| p.as_array())
                .map(|arr| {
                    arr.iter().any(|entry| {
                        entry.as_str().map(|s| s.contains("pi-subagent")).unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        } else {
            false
        };

        if !extension_installed {
            let status = std::process::Command::new("pi")
                .args(["install", "npm:@mjakl/pi-subagent"])
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("Failed to run `pi install npm:@mjakl/pi-subagent`")?;
            if !status.success() {
                anyhow::bail!(
                    "`pi install npm:@mjakl/pi-subagent` failed with code {:?}",
                    status.code()
                );
            }
        }

        // 4. Deploy skill files to <project_dir>/.pi/agents/.
        let skills_dir = Path::new(&self.config_root).join("skills");
        if !skills_dir.exists() {
            return Ok(());
        }

        let dest_dir = Path::new(&ctx.project_dir).join(".pi/agents");
        fs::create_dir_all(&dest_dir)
            .with_context(|| format!("Failed to create {}", dest_dir.display()))?;

        let entries = fs::read_dir(&skills_dir)
            .with_context(|| format!("Failed to read skills dir {}", skills_dir.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Warning: failed to read skills dir entry: {e}");
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let process = || -> Result<()> {
                let raw = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;

                let caps = FRONTMATTER_RE.captures(&raw).with_context(|| {
                    format!("No YAML frontmatter found in {}", path.display())
                })?;
                let fm_str = caps.get(1).unwrap().as_str();
                let body = caps.get(2).unwrap().as_str();

                #[derive(Deserialize)]
                struct SkillFrontmatter {
                    name: String,
                    description: String,
                    tools: Option<Vec<String>>,
                    model: Option<String>,
                    thinking: Option<String>,
                }

                let fm: SkillFrontmatter = serde_yaml::from_str(fm_str)
                    .with_context(|| format!("Failed to parse frontmatter in {}", path.display()))?;

                let mut out = format!("---\nname: {}\ndescription: {}\n", fm.name, fm.description);
                if let Some(tools) = &fm.tools {
                    out.push_str(&format!("tools: {}\n", tools.join(", ")));
                }
                if let Some(model) = &fm.model {
                    out.push_str(&format!("model: {}\n", model));
                }
                if let Some(thinking) = &fm.thinking {
                    out.push_str(&format!("thinking: {}\n", thinking));
                }
                out.push_str("---\n\n");
                out.push_str(body);

                let dest = dest_dir.join(&filename);
                fs::write(&dest, &out)
                    .with_context(|| format!("Failed to write {}", dest.display()))?;
                Ok(())
            };

            if let Err(e) = process() {
                eprintln!("Warning: failed to process skill {filename}: {e}");
            }
        }

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pi_tool_start() {
        let line = r#"{"type":"tool_execution_start","tool_name":"read","tool_input":{"file_path":"/foo.md"}}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(!f.persist);
        assert!(f.text.contains("/foo.md"));
    }

    #[test]
    fn parse_pi_message_end() {
        let line = r#"{"type":"message_end","content":[{"type":"text","text":"[ADDED] done"}]}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(f.persist);
        assert!(f.text.contains("ADDED"));
    }

    #[test]
    fn parse_pi_tool_error() {
        let line = r#"{"type":"tool_execution_end","isError":true}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, None, &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(f.persist);
        assert!(f.text.contains("tool error"));
    }

    #[test]
    fn parse_pi_ignores_blank() {
        let mut shown = HashSet::new();
        assert!(parse_pi_stream_line("", None, &mut shown).is_none());
    }
}
