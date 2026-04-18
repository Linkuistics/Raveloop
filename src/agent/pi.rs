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
    FormattedOutput, Intent, Span, Style, StyledLine, ToolCall, clean_tool_name,
    extract_tool_detail, format_result_text, format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{UIMessage, UISender};

// Dotall flag so `.` matches newlines in the body capture group.
static FRONTMATTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)^---\n(.*?)\n---\n(.*)$").expect("valid frontmatter regex")
});

/// Rolling cap on the stderr tail buffer. When a run exceeds this, the
/// oldest bytes are discarded and a one-shot warning is sent so the user
/// knows the error message they're about to see (on failure) is truncated.
/// Duplicated from `claude_code.rs` for now; the refactor task that
/// extracts shared spawn/stream machinery will unify them.
const STDERR_BUFFER_CAP: usize = 4096;

/// Builds a yellow `⚠  …` warning line for the TUI scrollback. Duplicated
/// from `claude_code.rs`; see the constant above for the extraction plan.
fn warning_line(body: impl Into<String>) -> StyledLine {
    StyledLine(vec![Span::styled(
        format!("  ⚠  {}", body.into()),
        Style::bold_intent(Intent::Changed),
    )])
}

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
            },
            "write" | "edit" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
            },
            "grep" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: Some(format!(
                    "\"{}\" in {}",
                    input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                    input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                )),
            },
            "find" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: input.get("pattern").or(input.get("glob"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
            },
            "bash" => ToolCall {
                name: name.to_string(),
                path: None,
                detail: input.get("command").and_then(|v| v.as_str())
                    .map(|s| s.chars().take(120).collect()),
            },
            _ => ToolCall {
                name: clean_tool_name(name),
                path: None,
                detail: Some(extract_tool_detail(&input)),
            },
        };
        return Some(format_tool_call(&tool, phase, shown_highlights));
    }

    if event_type == "tool_execution_end" {
        if event.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            let line = crate::format::StyledLine(vec![
                crate::format::Span::plain("  "),
                crate::format::Span::styled(
                    "✗  tool error",
                    crate::format::Style::intent(crate::format::Intent::Removed),
                ),
            ]);
            return Some(FormattedOutput {
                lines: vec![line],
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
                    lines: format_result_text(&text),
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

    /// Load a pi prompt file and run it through the shared
    /// `substitute_tokens` pipeline. Going through the pipeline (instead
    /// of ad-hoc `str::replace`) means any unresolved `{{NAME}}` token in
    /// the prompt fails loudly at load time — the `{{MEMORY_DIR}}` bug
    /// originally slipped past because this file did its own string
    /// replacement and never ran the guard regex.
    fn load_prompt_file(&self, name: &str, ctx: &PlanContext) -> Result<String> {
        let path = Path::new(&self.config_root).join("agents/pi/prompts").join(name);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        crate::prompt::substitute_tokens(&content, ctx, &HashMap::new())
            .with_context(|| format!("Failed to substitute tokens in {}", path.display()))
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
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn pi")?;

        let stdout = child.stdout.take().context("No stdout")?;
        let stderr = child.stderr.take().context("No stderr")?;

        // Drain stderr concurrently so it never blocks the child; retain
        // the last STDERR_BUFFER_CAP bytes so failures can be surfaced in
        // the error. On the first overflow, emit a one-shot Persist
        // warning so the user knows any error tail they later see is
        // the tail, not the head. Previously pi used `Stdio::inherit()`,
        // which let raw stderr bleed into the terminal underneath the
        // TUI and get overwritten by the next repaint — invisibly
        // losing the very output the user needed to debug a failure.
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
                                "pi stderr exceeded {STDERR_BUFFER_CAP}-byte buffer — earlier lines dropped"
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
                    if let Some(formatted) = parse_pi_stream_line(&line, Some(phase), &mut shown_highlights) {
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
                anyhow::bail!("pi exited with code {:?}", status.code());
            }
            anyhow::bail!(
                "pi exited with code {:?}\n--- stderr ---\n{trimmed}",
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

        // 4. Deploy subagent definitions to <project_dir>/.pi/agents/.
        let subagents_src = Path::new(&self.config_root).join("agents/pi/subagents");
        if !subagents_src.exists() {
            return Ok(());
        }

        let dest_dir = Path::new(&ctx.project_dir).join(".pi/agents");
        fs::create_dir_all(&dest_dir)
            .with_context(|| format!("Failed to create {}", dest_dir.display()))?;

        let entries = fs::read_dir(&subagents_src)
            .with_context(|| format!("Failed to read subagents dir {}", subagents_src.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Warning: failed to read subagents dir entry: {e}");
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
                eprintln!("Warning: failed to process subagent {filename}: {e}");
            }
        }

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(f: &FormattedOutput) -> String {
        f.lines.iter()
            .map(|l| l.0.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parse_pi_tool_start() {
        let line = r#"{"type":"tool_execution_start","tool_name":"read","tool_input":{"file_path":"/foo.md"}}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(!f.persist);
        assert!(flat(&f).contains("/foo.md"));
    }

    #[test]
    fn parse_pi_message_end() {
        let line = r#"{"type":"message_end","content":[{"type":"text","text":"[ADDED] done"}]}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(f.persist);
        assert!(flat(&f).contains("ADDED"));
    }

    #[test]
    fn parse_pi_tool_error() {
        let line = r#"{"type":"tool_execution_end","isError":true}"#;
        let mut shown = HashSet::new();
        let r = parse_pi_stream_line(line, None, &mut shown);
        assert!(r.is_some());
        let f = r.unwrap();
        assert!(f.persist);
        assert!(flat(&f).contains("tool error"));
    }

    #[test]
    fn parse_pi_ignores_blank() {
        let mut shown = HashSet::new();
        assert!(parse_pi_stream_line("", None, &mut shown).is_none());
    }

    fn agent_with_prompts(prompts: &[(&str, &str)]) -> (tempfile::TempDir, PiAgent) {
        let dir = tempfile::TempDir::new().unwrap();
        let prompts_dir = dir.path().join("agents/pi/prompts");
        fs::create_dir_all(&prompts_dir).unwrap();
        for (name, body) in prompts {
            fs::write(prompts_dir.join(name), body).unwrap();
        }
        let config = AgentConfig {
            models: HashMap::new(),
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: None,
        };
        let agent = PiAgent::new(config, dir.path().to_string_lossy().to_string());
        (dir, agent)
    }

    fn test_plan_ctx() -> PlanContext {
        PlanContext {
            plan_dir: "/plans/my-plan".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: String::new(),
            config_root: "/config".to_string(),
        }
    }

    #[test]
    fn load_prompt_substitutes_plan_token() {
        let (_dir, agent) = agent_with_prompts(&[(
            "memory-prompt.md",
            "Memory lives at {{PLAN}}/auto-memory.",
        )]);
        let out = agent.load_prompt_file("memory-prompt.md", &test_plan_ctx()).unwrap();
        assert_eq!(out, "Memory lives at /plans/my-plan/auto-memory.");
    }

    #[test]
    fn load_prompt_fails_on_unresolved_token() {
        // Regression guard: a new `{{X}}` slipping into a pi prompt must
        // fail at load time. Before routing through `substitute_tokens`,
        // the literal token reached the LLM unchanged — that is how the
        // `{{MEMORY_DIR}}` bug went undetected.
        let (_dir, agent) = agent_with_prompts(&[(
            "memory-prompt.md",
            "dangling {{MEMORY_DIR}} token",
        )]);
        let err = agent
            .load_prompt_file("memory-prompt.md", &test_plan_ctx())
            .expect_err("unresolved token should fail to load");
        let msg = format!("{err:#}");
        assert!(msg.contains("{{MEMORY_DIR}}"), "error should name the token: {msg}");
    }

    #[test]
    fn shipped_pi_prompts_have_no_dangling_tokens() {
        // Drift guard: every on-disk defaults/agents/pi/prompts/*.md must
        // substitute cleanly against a realistic PlanContext. A future
        // `{{X}}` added to a prompt without a matching token source will
        // fail here rather than at agent invocation time.
        let prompts_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("defaults")
            .join("agents")
            .join("pi")
            .join("prompts");
        let entries: Vec<_> = fs::read_dir(&prompts_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .collect();
        assert!(!entries.is_empty(), "expected at least one pi prompt on disk");

        let ctx = test_plan_ctx();
        for entry in entries {
            let path = entry.path();
            let body = fs::read_to_string(&path).unwrap();
            crate::prompt::substitute_tokens(&body, &ctx, &HashMap::new())
                .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        }
    }
}
