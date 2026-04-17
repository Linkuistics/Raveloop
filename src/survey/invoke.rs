// src/survey/invoke.rs
//
// Spawn + read the `claude` CLI for the survey, and the end-to-end
// orchestrator that ties discovery, composition, invocation, parsing,
// and rendering together.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};
use crate::types::AgentConfig;

use super::compose::{load_survey_prompt, render_survey_input};
use super::discover::discover_plans;
use super::render::render_survey_output;
use super::schema::parse_survey_response;

/// Fallback model when neither `--model` nor `models.survey` is
/// configured. A cheap, fast model is appropriate: survey is a
/// summarisation task over plain-text inputs.
pub const DEFAULT_SURVEY_MODEL: &str = "claude-haiku-4-5";

/// Resolve which model to use for the survey call. Precedence:
///   1. explicit `--model` flag on the CLI
///   2. `models.survey` in the agent's config
///   3. `DEFAULT_SURVEY_MODEL` constant
fn resolve_model(agent_config: &AgentConfig, flag_override: Option<String>) -> String {
    flag_override
        .or_else(|| agent_config.models.get("survey").cloned())
        .unwrap_or_else(|| DEFAULT_SURVEY_MODEL.to_string())
}

/// End-to-end survey runner. Gathers plans across every `--root`,
/// composes the prompt, invokes the `claude` CLI headlessly, and
/// prints the LLM's response to stdout.
pub async fn run_survey(
    config_root: &Path,
    roots: &[PathBuf],
    model_override: Option<String>,
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

    let mut all_plans = Vec::new();
    for root in roots {
        if !root.is_dir() {
            anyhow::bail!(
                "Plan root {} does not exist or is not a directory.",
                root.display()
            );
        }
        let plans = discover_plans(root)?;
        if plans.is_empty() {
            eprintln!(
                "warning: plan root {} contained no plan directories (no phase.md found)",
                root.display()
            );
        }
        all_plans.extend(plans);
    }
    if all_plans.is_empty() {
        anyhow::bail!("No plans discovered in any of the supplied --root directories.");
    }

    let survey_prompt = load_survey_prompt(config_root)?;
    let plan_input = render_survey_input(&all_plans);
    let full_prompt = format!("{survey_prompt}\n\n---\n{plan_input}");

    eprintln!(
        "Surveying {} plan(s) across {} root(s) using model {}...",
        all_plans.len(),
        roots.len(),
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
    stdout.read_to_string(&mut output).await?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude CLI exited with status {status}");
    }

    let response = parse_survey_response(&output)?;
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
}
