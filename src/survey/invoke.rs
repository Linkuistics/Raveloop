// src/survey/invoke.rs
//
// Spawn + read the `claude` CLI for the survey, and the end-to-end
// orchestrator that ties plan loading, composition, invocation,
// parsing, hash injection, and YAML emission together. Markdown
// rendering is now a separate concern delegated to the
// `ravel-lite survey-format` subcommand.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};
use crate::types::AgentConfig;

use super::compose::{load_survey_prompt, render_survey_input};
use super::discover::load_plan;
use super::render::render_survey_output;
use super::schema::{emit_survey_yaml, inject_input_hashes, parse_survey_response, plan_key};

/// Fallback model when neither `--model` nor `models.survey` is
/// configured. A cheap, fast model is appropriate: survey is a
/// summarisation task over plain-text inputs.
pub const DEFAULT_SURVEY_MODEL: &str = "claude-haiku-4-5";

/// Default ceiling on how long the `claude` subprocess may run before
/// survey gives up. Survey is advertised as single-shot and read-only;
/// a hang with no feedback is a worse failure mode than a loud error
/// that the user can retry. Five minutes is generous for any model
/// summarising plain-text inputs and short enough to surface problems
/// before the user walks away.
pub const DEFAULT_SURVEY_TIMEOUT_SECS: u64 = 300;

fn resolve_timeout(flag_override: Option<u64>) -> Duration {
    Duration::from_secs(flag_override.unwrap_or(DEFAULT_SURVEY_TIMEOUT_SECS))
}

/// Resolve which model to use for the survey call. Precedence:
///   1. explicit `--model` flag on the CLI
///   2. `models.survey` in the agent's config
///   3. `DEFAULT_SURVEY_MODEL` constant
fn resolve_model(agent_config: &AgentConfig, flag_override: Option<String>) -> String {
    flag_override
        .or_else(|| agent_config.models.get("survey").cloned())
        .unwrap_or_else(|| DEFAULT_SURVEY_MODEL.to_string())
}

/// End-to-end survey runner. Loads each named plan directory,
/// composes the prompt, invokes the `claude` CLI headlessly, parses
/// the YAML response, injects Rust-computed `input_hash` values into
/// each row, and writes canonical YAML to stdout.
///
/// The plan-root walk is gone: each positional argument on the CLI
/// names exactly one plan directory (a directory containing
/// `phase.md`). Routing responsibility stays in the caller.
pub async fn run_survey(
    config_root: &Path,
    plan_dirs: &[PathBuf],
    model_override: Option<String>,
    timeout_override_secs: Option<u64>,
) -> Result<()> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "survey currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = resolve_model(&agent_config, model_override);

    if plan_dirs.is_empty() {
        anyhow::bail!("No plan directories supplied.");
    }
    let mut all_plans = Vec::with_capacity(plan_dirs.len());
    for plan_dir in plan_dirs {
        let snapshot = load_plan(plan_dir)
            .with_context(|| format!("Failed to load plan at {}", plan_dir.display()))?;
        all_plans.push(snapshot);
    }
    all_plans.sort_by(|a, b| (&a.project, &a.plan).cmp(&(&b.project, &b.plan)));

    let survey_prompt = load_survey_prompt(config_root)?;
    let plan_input = render_survey_input(&all_plans);
    let full_prompt = format!("{survey_prompt}\n\n---\n{plan_input}");

    eprintln!(
        "Surveying {} plan(s) using model {}...",
        all_plans.len(),
        model
    );

    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(&full_prompt)
        .arg("--model")
        .arg(&model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn 'claude' CLI. Ensure it is installed and on PATH.")?;

    let mut stdout = child
        .stdout
        .take()
        .context("claude CLI stdout pipe was unexpectedly unavailable")?;
    let mut output = String::new();
    let timeout = resolve_timeout(timeout_override_secs);
    let start = Instant::now();
    let read_result = tokio::time::timeout(timeout, stdout.read_to_string(&mut output)).await;

    match read_result {
        Ok(Ok(_)) => {}
        Ok(Err(io_err)) => {
            let _ = child.kill().await;
            return Err(io_err).context("failed reading stdout from claude");
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            anyhow::bail!(
                "claude CLI did not produce a result within {}s timeout (elapsed {}s).\n\
                 Captured {} bytes of stdout before timing out:\n{}\n\n\
                 Try one of:\n  \
                 * re-run the command (transient hangs sometimes clear)\n  \
                 * swap the model with --model <other>\n  \
                 * check network / API reachability\n  \
                 * extend the limit with --timeout-secs <N>",
                timeout.as_secs(),
                start.elapsed().as_secs(),
                output.len(),
                output,
            );
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude CLI exited with status {status}");
    }

    let mut response = parse_survey_response(&output)?;
    let hashes: HashMap<String, String> = all_plans
        .iter()
        .map(|p| (plan_key(&p.project, &p.plan), p.input_hash.clone()))
        .collect();
    inject_input_hashes(&mut response, &hashes)?;
    print!("{}", emit_survey_yaml(&response)?);
    Ok(())
}

/// Render a saved YAML survey file as human-readable markdown on
/// stdout. Parsing goes through the same schema + `parse_survey_response`
/// path as `run_survey`, then delegates to `render_survey_output` —
/// separates presentation from the (possibly expensive) LLM call and
/// lets the user re-render a stored survey cheaply.
pub fn run_survey_format(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read survey file at {}", path.display()))?;
    let response = parse_survey_response(&content)?;
    print!("{}", render_survey_output(&response));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_agent_config(models: &[(&str, &str)]) -> AgentConfig {
        let mut m = HashMap::new();
        for (k, v) in models {
            m.insert(k.to_string(), v.to_string());
        }
        AgentConfig {
            models: m,
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: None,
        }
    }

    #[test]
    fn resolve_model_prefers_cli_flag() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, Some("flag-model".into()));
        assert_eq!(resolved, "flag-model");
    }

    #[test]
    fn resolve_model_falls_back_to_agent_config_survey_key() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, "configured-model");
    }

    #[test]
    fn resolve_model_uses_default_when_nothing_configured() {
        let cfg = empty_agent_config(&[]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, DEFAULT_SURVEY_MODEL);
    }

    #[test]
    fn resolve_timeout_uses_default_when_no_override() {
        assert_eq!(resolve_timeout(None), Duration::from_secs(DEFAULT_SURVEY_TIMEOUT_SECS));
    }

    #[test]
    fn resolve_timeout_honours_override() {
        assert_eq!(resolve_timeout(Some(42)), Duration::from_secs(42));
    }

    #[test]
    fn run_survey_format_renders_markdown_from_yaml_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("survey.yaml");
        std::fs::write(
            &path,
            "plans:\n  - project: P\n    plan: x\n    phase: work\n    unblocked: 1\n\
             \n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n",
        )
        .unwrap();
        // run_survey_format writes to stdout, so this test only checks
        // the happy path doesn't error. Content-level golden rendering
        // is covered by render_survey_output's own tests.
        run_survey_format(&path).unwrap();
    }

    #[test]
    fn run_survey_format_errors_on_missing_file() {
        let missing = std::path::PathBuf::from("/definitely/not/a/survey/file.yaml");
        let err = run_survey_format(&missing).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to read survey file"));
    }

    #[test]
    fn run_survey_format_errors_on_malformed_yaml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.yaml");
        std::fs::write(&path, "not: valid: yaml: at: all:\n  - [").unwrap();
        let err = run_survey_format(&path).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to parse survey response"));
    }
}
