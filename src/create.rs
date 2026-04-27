// src/create.rs
//
// Interactive plan-creation subcommand. Spawns a headful `claude`
// session that reads the `create-plan.md` prompt template from the
// user's config directory, appends the target plan path, and inherits
// the parent's stdio so the user drives the conversation directly.
//
// Unlike `survey` (read-only one-shot), `create` writes files — so
// it needs the agent's interactive REPL and tool-approval flow. The
// Ravel-Lite process is a thin wrapper: path validation, prompt
// composition, subprocess spawn with inherited stdio, post-hoc
// verification.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};
use crate::state::filenames::{
    BACKLOG_FILENAME, DREAM_WORD_COUNT_FILENAME, MEMORY_FILENAME, PHASE_FILENAME,
};

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
         you created.\n\n\
         INVARIANT: Your ONLY output from this session is a plan \
         directory at {target}. If the user's description sounds like \
         a single concrete task (a bug report, a feature request, a \
         specific question), that is the plan's initial task — \
         capture it in the backlog and scaffold the plan around it. A \
         single-task plan is a valid plan. Do not attempt to do the \
         work the user described; your job is to write plan files, \
         not to solve the problem they described.\n",
        target = abs_plan_dir.display()
    )
}

/// Validate and prepare the target path. Returns the absolute path to the
/// plan directory on success. Hard errors only if the target already exists
/// or its parent path resolves to an existing file (not a directory).
///
/// Missing parent directories are created automatically — the user's intent
/// is clear, and the parent must exist on disk at spawn time because
/// `claude --add-dir <parent>` resolves the path eagerly.
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
    if parent.exists() && !parent.is_dir() {
        anyhow::bail!(
            "Parent path {} exists but is not a directory.",
            parent.display()
        );
    }
    if !parent.exists() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create parent directory {}", parent.display())
        })?;
    }

    Ok(abs)
}

/// Scaffold the minimum set of files a plan directory must contain
/// before the create-plan LLM session runs. Creates the directory
/// itself (refusing if it already exists) and writes:
///
/// - `phase.md` = `work\n`
/// - `backlog.yaml` = `tasks: []\n`
/// - `memory.yaml` = `entries: []\n`
/// - `dream-word-count` = `0`
///
/// Parent directories are NOT created here — `validate_target` handles
/// that — so this function only succeeds when called against a freshly
/// validated target path.
///
/// After scaffolding, the LLM populates backlog and memory via
/// `ravel-lite state backlog add` / `state memory add` rather than
/// writing YAML directly. This keeps the "no LLM-authored mechanical
/// scaffolding" contract intact.
pub fn scaffold_plan_dir(abs_plan_dir: &Path) -> Result<()> {
    fs::create_dir(abs_plan_dir).with_context(|| {
        format!(
            "Failed to create plan directory {}",
            abs_plan_dir.display()
        )
    })?;

    let writes: [(&str, &[u8]); 4] = [
        (PHASE_FILENAME, b"work\n"),
        (BACKLOG_FILENAME, b"tasks: []\n"),
        (MEMORY_FILENAME, b"entries: []\n"),
        (DREAM_WORD_COUNT_FILENAME, b"0"),
    ];
    for (name, bytes) in writes {
        let path = abs_plan_dir.join(name);
        fs::write(&path, bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(())
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

    // Runner-owned scaffolding runs BEFORE the claude spawn so the LLM
    // never has to create mechanical files (phase.md, empty YAML shells,
    // dream-word-count). The create-plan prompt directs it to populate
    // backlog/memory exclusively through `state backlog add` /
    // `state memory add` — no raw writes, no `state backlog init`.
    scaffold_plan_dir(&abs_plan_dir)?;

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

    // Post-hoc verification: scaffolding guarantees phase.md exists
    // from the pre-spawn write, so a still-empty backlog signals that
    // the LLM session exited before populating any tasks. Anything
    // stricter (e.g. requiring N tasks) would fight single-task plans.
    let backlog = crate::state::backlog::read_backlog(&abs_plan_dir)
        .context("Failed to read scaffolded backlog.yaml after claude session")?;
    if backlog.tasks.is_empty() {
        eprintln!(
            "\nwarning: {} still has no tasks — the session may have exited before the plan was populated.",
            abs_plan_dir.display()
        );
    } else {
        println!("\nPlan created at {}", abs_plan_dir.display());
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
    fn compose_prompt_asserts_plan_only_output_invariant() {
        let out = compose_create_prompt("TEMPLATE", Path::new("/abs/plan"));
        assert!(
            out.contains("ONLY output from this session is a plan directory"),
            "composed prompt must bind the agent against pivoting away \
             from plan creation when the user pastes a concrete problem; \
             got:\n{out}"
        );
        assert!(
            out.contains("single-task plan is a valid plan"),
            "composed prompt must tell the agent that a concrete one-task \
             description is acceptable plan scope, not a pivot signal"
        );
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
    fn validate_target_creates_missing_parent_directories() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b").join("c").join("plan-name");
        let resolved = validate_target(&nested).unwrap();
        assert!(resolved.is_absolute());
        assert!(
            nested.parent().unwrap().is_dir(),
            "validate_target should have created the missing parent chain"
        );
        assert!(
            !nested.exists(),
            "validate_target must NOT create the plan directory itself"
        );
    }

    #[test]
    fn validate_target_rejects_when_parent_is_a_file() {
        let tmp = TempDir::new().unwrap();
        let file_as_parent = tmp.path().join("not-a-dir");
        fs::write(&file_as_parent, "").unwrap();
        let target = file_as_parent.join("plan-name");
        let err = validate_target(&target).unwrap_err();
        assert!(format!("{err:#}").contains("not a directory"));
    }

    #[test]
    fn validate_target_accepts_new_path_under_existing_parent() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("new-plan");
        let resolved = validate_target(&target).unwrap();
        assert!(resolved.is_absolute(), "validate_target must return an absolute path");
        assert_eq!(resolved.file_name().unwrap(), "new-plan");
    }

    #[test]
    fn scaffold_plan_dir_creates_directory_and_required_files() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();
        assert!(plan.is_dir(), "plan directory must exist after scaffold");
        assert_eq!(fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap(), "work\n");
        assert_eq!(fs::read_to_string(plan.join(BACKLOG_FILENAME)).unwrap(), "tasks: []\n");
        assert_eq!(fs::read_to_string(plan.join(MEMORY_FILENAME)).unwrap(), "entries: []\n");
        assert_eq!(fs::read_to_string(plan.join(DREAM_WORD_COUNT_FILENAME)).unwrap(), "0");
    }

    #[test]
    fn scaffold_plan_dir_writes_cli_parseable_state_files() {
        // The YAML shells must parse via the canonical readers so the
        // LLM's first `state backlog add` / `state memory add` lands on
        // valid files rather than triggering a format error.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        let backlog = crate::state::backlog::read_backlog(&plan).unwrap();
        assert!(backlog.tasks.is_empty(), "scaffolded backlog must have no tasks");

        let memory = crate::state::memory::read_memory(&plan).unwrap();
        assert!(memory.entries.is_empty(), "scaffolded memory must have no entries");
    }

    #[test]
    fn scaffold_plan_dir_refuses_existing_directory() {
        let tmp = TempDir::new().unwrap();
        let err = scaffold_plan_dir(tmp.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("create plan directory"),
            "scaffold_plan_dir must error when the plan dir already exists; got: {err:#}"
        );
    }
}
