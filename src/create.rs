// src/create.rs
//
// Interactive plan-creation subcommand. Spawns a headful `claude`
// session that reads the `create-plan.md` prompt template from the
// user's config directory, appends the target plan path, and inherits
// the parent's stdio so the user drives the conversation directly.
//
// Unlike `survey` (read-only one-shot), `create` writes files — so
// it needs the agent's interactive REPL and tool-approval flow. The
// Raveloop process is a thin wrapper: path validation, prompt
// composition, subprocess spawn with inherited stdio, post-hoc
// verification.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};

/// Relative path to the create-plan prompt template inside a config dir.
pub const CREATE_PLAN_PROMPT_PATH: &str = "create-plan.md";

/// Compose the full prompt given the template and target plan path.
/// Pure function — broken out so tests cover the composition
/// deterministically without spawning claude.
pub fn compose_create_prompt(template: &str, abs_plan_dir: &Path) -> String {
    format!(
        "{template}\n\n---\n\n\
         Create a new plan at this absolute path:\n\n  {target}\n\n\
         The directory does not exist yet. Create it, then create the \
         plan files inside it according to the conventions above. \
         When the plan is ready, confirm with the user by listing what \
         you created.\n",
        target = abs_plan_dir.display()
    )
}

/// Validate the target path. Returns the absolute path to the plan
/// directory on success. Hard errors if the directory already exists
/// or its parent is missing.
pub fn validate_target(plan_dir: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(plan_dir)
        .with_context(|| format!("Failed to resolve absolute path for {}", plan_dir.display()))?;

    if abs.exists() {
        anyhow::bail!(
            "Plan directory {} already exists. create will not overwrite an existing plan.",
            abs.display()
        );
    }

    let parent = abs
        .parent()
        .with_context(|| format!("Plan path {} has no parent directory", abs.display()))?;
    if !parent.is_dir() {
        anyhow::bail!(
            "Parent directory {} does not exist. Create it first, then re-run raveloop create.",
            parent.display()
        );
    }

    Ok(abs)
}

pub async fn run_create(config_root: &Path, plan_dir: PathBuf) -> Result<()> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "create currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let abs_plan_dir = validate_target(&plan_dir)?;
    let parent = abs_plan_dir
        .parent()
        .expect("validated parent exists")
        .to_path_buf();

    let prompt_path = config_root.join(CREATE_PLAN_PROMPT_PATH);
    let template = fs::read_to_string(&prompt_path)
        .with_context(|| format!("Failed to read create-plan prompt at {}", prompt_path.display()))?;
    let prompt = compose_create_prompt(&template, &abs_plan_dir);

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    // Plan creation is work-phase-like reasoning; reuse the configured
    // work model rather than introducing a separate model axis.
    let model = agent_config.models.get("work").cloned().ok_or_else(|| {
        anyhow::anyhow!("Agent config is missing a `models.work` entry; cannot select a model for create.")
    })?;

    eprintln!(
        "Launching interactive claude session (model: {}) to create plan at {}...",
        model,
        abs_plan_dir.display()
    );

    let mut child = TokioCommand::new("claude")
        .arg(&prompt)
        .arg("--model")
        .arg(&model)
        .arg("--add-dir")
        .arg(&parent)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn claude CLI. Ensure it is installed and on PATH.")?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude exited with status {status}");
    }

    // Post-hoc verification: the defining artifact of a plan is
    // phase.md. Its presence confirms the session actually created
    // the plan rather than, say, exiting early after conversation.
    let phase_md = abs_plan_dir.join("phase.md");
    if phase_md.exists() {
        println!("\nPlan created at {}", abs_plan_dir.display());
    } else {
        eprintln!(
            "\nwarning: {} does not exist — the session may have exited before the plan was written.",
            phase_md.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn compose_prompt_appends_target_path_section() {
        let out = compose_create_prompt("TEMPLATE", Path::new("/abs/plan"));
        assert!(out.starts_with("TEMPLATE"));
        assert!(out.contains("Create a new plan at this absolute path"));
        assert!(out.contains("/abs/plan"));
    }

    #[test]
    fn compose_prompt_separates_template_from_instructions_with_hr() {
        let out = compose_create_prompt("TEMPLATE", Path::new("/x"));
        assert!(out.contains("\n\n---\n\n"));
    }

    #[test]
    fn validate_target_rejects_existing_directory() {
        let tmp = TempDir::new().unwrap();
        let err = validate_target(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("already exists"));
    }

    #[test]
    fn validate_target_rejects_existing_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("a-file");
        fs::write(&file_path, "").unwrap();
        let err = validate_target(&file_path).unwrap_err();
        assert!(format!("{err:#}").contains("already exists"));
    }

    #[test]
    fn validate_target_rejects_when_parent_missing() {
        let tmp = TempDir::new().unwrap();
        let missing_parent = tmp.path().join("does-not-exist").join("plan-name");
        let err = validate_target(&missing_parent).unwrap_err();
        assert!(format!("{err:#}").contains("Parent directory"));
    }

    #[test]
    fn validate_target_accepts_new_path_under_existing_parent() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("new-plan");
        let resolved = validate_target(&target).unwrap();
        assert!(resolved.is_absolute(), "validate_target must return an absolute path");
        assert_eq!(resolved.file_name().unwrap(), "new-plan");
    }
}
