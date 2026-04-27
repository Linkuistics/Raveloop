use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use tokio::process::Command;

use super::Agent;
use super::common::{
    STREAM_SNIPPET_BYTES, StreamLineOutcome, build_dispatch_plan_context, run_streaming_child,
    truncate_snippet,
};
use crate::config::load_tokens;
use crate::debug_log;
use crate::init::{embedded_entries_with_prefix, require_embedded};
use crate::format::{
    FormattedOutput, Intent, Span, Style, StyledLine, ToolCall, clean_tool_name,
    extract_tool_detail, format_result_text, format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::UISender;

// Dotall flag so `.` matches newlines in the body capture group.
static FRONTMATTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)^---\n(.*?)\n---\n(.*)$").expect("valid frontmatter regex")
});

// ── Stream parser ─────────────────────────────────────────────────────────────

fn parse_pi_stream_line(
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

    if event_type == "tool_execution_start" {
        let Some(name) = event.get("tool_name").and_then(|v| v.as_str()) else {
            return StreamLineOutcome::Ignored;
        };
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
        return StreamLineOutcome::Output(format_tool_call(&tool, phase, shown_highlights));
    }

    if event_type == "tool_execution_end" {
        if event.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            let line = StyledLine(vec![
                Span::plain("  "),
                Span::styled("✗  tool error", Style::intent(Intent::Removed)),
            ]);
            return StreamLineOutcome::Output(FormattedOutput {
                lines: vec![line],
                persist: true,
            });
        }
        return StreamLineOutcome::Ignored;
    }

    if event_type == "message_end" {
        if let Some(content) = event.get("content").and_then(|c| c.as_array()) {
            let text: String = content.iter()
                .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return StreamLineOutcome::Output(FormattedOutput {
                    lines: format_result_text(&text),
                    persist: true,
                });
            }
        }
    }

    StreamLineOutcome::Ignored
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

    /// Load a pi prompt from the embedded set and run it through the
    /// shared `substitute_tokens` pipeline. Going through the pipeline
    /// (instead of ad-hoc `str::replace`) means any unresolved
    /// `{{NAME}}` token fails loudly at load time — the
    /// `{{MEMORY_DIR}}` bug originally slipped past because this file
    /// did its own string replacement and never ran the guard regex.
    fn load_prompt_file(&self, name: &str, ctx: &PlanContext) -> Result<String> {
        let rel = format!("agents/pi/prompts/{name}");
        let content = require_embedded(&rel)?;
        crate::prompt::substitute_tokens(content, ctx, &HashMap::new())
            .with_context(|| format!("Failed to substitute tokens in embedded {rel}"))
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

        if debug_log::is_enabled() {
            debug_log::log(
                "pi spawn (interactive, work)",
                &format!(
                    "cwd: {}\nstdio: inherited (no transcript available)\n{}\nprompt:\n{}",
                    ctx.project_dir,
                    debug_log::format_argv("pi", &args),
                    indent_block(prompt),
                ),
            );
        }

        let status = std::process::Command::new("pi")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn pi")?;

        debug_log::log(
            "pi exit (interactive, work)",
            &format!("status: {:?}", status.code()),
        );

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

        if debug_log::is_enabled() {
            debug_log::log(
                &format!("pi spawn (headless, {})", phase.as_str()),
                &format!(
                    "cwd: {}\nagent_id: {}\n{}\nprompt:\n{}",
                    ctx.project_dir,
                    agent_id,
                    debug_log::format_argv("pi", &args),
                    indent_block(prompt),
                ),
            );
        }

        let child = Command::new("pi")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn pi")?;

        run_streaming_child(child, phase, agent_id, "pi", tx, parse_pi_stream_line).await
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

        // 4. Deploy subagent definitions to <project_dir>/.pi/agents/
        // straight from the embedded set. The runtime never reads
        // `<config>/agents/pi/subagents/` from disk — registration in
        // `EMBEDDED_FILES` is the only source of truth.
        let dest_dir = Path::new(&ctx.project_dir).join(".pi/agents");
        fs::create_dir_all(&dest_dir)
            .with_context(|| format!("Failed to create {}", dest_dir.display()))?;

        let prefix = "agents/pi/subagents/";
        for (rel_path, raw) in embedded_entries_with_prefix(prefix) {
            let filename = rel_path.trim_start_matches(prefix);
            if !filename.ends_with(".md") {
                continue;
            }

            let process = || -> Result<()> {
                let caps = FRONTMATTER_RE.captures(raw).with_context(|| {
                    format!("No YAML frontmatter found in embedded {rel_path}")
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

                let fm: SkillFrontmatter = serde_yaml::from_str(fm_str).with_context(|| {
                    format!("Failed to parse frontmatter in embedded {rel_path}")
                })?;

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

                let dest = dest_dir.join(filename);
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

/// Indent every line of `body` by four spaces so it renders as a
/// nested block under a debug-log entry header.
fn indent_block(body: &str) -> String {
    body.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
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
    fn parse_pi_tool_start() {
        let line = r#"{"type":"tool_execution_start","tool_name":"read","tool_input":{"file_path":"/foo.md"}}"#;
        let mut shown = HashSet::new();
        let f = expect_output(parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(!f.persist);
        assert!(flat(&f).contains("/foo.md"));
    }

    #[test]
    fn parse_pi_message_end() {
        let line = r#"{"type":"message_end","content":[{"type":"text","text":"[ADDED] done"}]}"#;
        let mut shown = HashSet::new();
        let f = expect_output(parse_pi_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(f.persist);
        assert!(flat(&f).contains("ADDED"));
    }

    #[test]
    fn parse_pi_tool_error() {
        let line = r#"{"type":"tool_execution_end","isError":true}"#;
        let mut shown = HashSet::new();
        let f = expect_output(parse_pi_stream_line(line, None, &mut shown));
        assert!(f.persist);
        assert!(flat(&f).contains("tool error"));
    }

    #[test]
    fn parse_pi_ignores_blank() {
        let mut shown = HashSet::new();
        assert!(matches!(
            parse_pi_stream_line("", None, &mut shown),
            StreamLineOutcome::Ignored
        ));
    }

    #[test]
    fn parse_pi_malformed_json_surfaces_snippet() {
        // Pi used to silently drop malformed stream-JSON lines via
        // `Option::None`; distinguishing Malformed lets the pump surface
        // a warning so format drift doesn't hide.
        let mut shown = HashSet::new();
        let outcome = parse_pi_stream_line("not json at all", None, &mut shown);
        let StreamLineOutcome::Malformed { snippet } = outcome else {
            panic!("expected Malformed");
        };
        assert_eq!(snippet, "not json at all");
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

    fn pi_agent_for_test() -> PiAgent {
        let config = AgentConfig {
            models: HashMap::new(),
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: None,
        };
        PiAgent::new(config, "/unused".to_string())
    }

    #[test]
    fn load_prompt_returns_embedded_and_substitutes_tokens() {
        // `load_prompt_file` must source from the embedded set (no
        // `<config>/agents/pi/prompts/` disk read). The shipped
        // `memory-prompt.md` references `{{PLAN}}`, which substitution
        // resolves to the plan_dir from the PlanContext.
        let agent = pi_agent_for_test();
        let out = agent.load_prompt_file("memory-prompt.md", &test_plan_ctx()).unwrap();
        assert!(
            out.contains("/plans/my-plan"),
            "{{PLAN}} should be substituted with plan_dir; got: {out}"
        );
        assert!(
            !out.contains("{{PLAN}}"),
            "no unresolved {{PLAN}} should remain after substitution"
        );
    }

    #[test]
    fn load_prompt_fails_when_path_not_registered() {
        // Drift guard: asking for a non-shipped pi prompt errors with
        // a deterministic message naming the unregistered path.
        let agent = pi_agent_for_test();
        let err = agent
            .load_prompt_file("does-not-exist.md", &test_plan_ctx())
            .expect_err("unregistered embedded path should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("agents/pi/prompts/does-not-exist.md"), "msg: {msg}");
    }

    #[test]
    fn shipped_pi_prompts_have_no_dangling_tokens() {
        // Drift guard: every embedded pi prompt must substitute cleanly
        // against a realistic PlanContext. A future `{{X}}` added to a
        // prompt without a matching token source will fail here rather
        // than at agent invocation time.
        let ctx = test_plan_ctx();
        let mut count = 0;
        for (rel, body) in embedded_entries_with_prefix("agents/pi/prompts/") {
            crate::prompt::substitute_tokens(body, &ctx, &HashMap::new())
                .unwrap_or_else(|e| panic!("{rel}: {e:#}"));
            count += 1;
        }
        assert!(count > 0, "expected at least one embedded pi prompt");
    }
}
