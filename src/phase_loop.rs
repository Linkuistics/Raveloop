use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::dream::{should_dream, update_dream_baseline};
use crate::format::phase_info;
use crate::git::{git_commit_plan, git_save_work_baseline, work_tree_snapshot, working_tree_status};
use crate::prompt::compose_prompt;
use crate::subagent::dispatch_subagents;
use crate::types::*;
use crate::ui::UI;

const HR: &str = "────────────────────────────────────────────────────";

fn read_phase(plan_dir: &Path) -> Result<Phase> {
    let content = fs::read_to_string(plan_dir.join("phase.md"))
        .context("Failed to read phase.md")?;
    Phase::parse(content.trim())
        .with_context(|| format!("Unknown phase: {}", content.trim()))
}

/// Writes the next phase marker to `phase.md`. Errors are propagated so
/// the loop doesn't silently advance past a filesystem failure (permissions,
/// full disk, stale handle) — the phase file is the single source of truth
/// for the loop's position, so a dropped write would re-invoke the agent on
/// the same phase and hide the real error.
fn write_phase(plan_dir: &Path, phase: Phase) -> Result<()> {
    let path = plan_dir.join("phase.md");
    fs::write(&path, phase.to_string())
        .with_context(|| format!("Failed to write phase marker: {}", path.display()))
}

fn plan_name(plan_dir: &Path) -> String {
    plan_dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Basename of the project directory, for the phase header. Many plans
/// share generic names like "core", so the project disambiguates which
/// session a banner belongs to in scrollback or when several sessions are up.
fn project_name(project_dir: &str) -> String {
    Path::new(project_dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Format the `project / plan` discriminator. Falls back to just the plan
/// when the project basename is empty (defensive — `project_dir` is normally
/// an absolute path under a real repo).
fn header_scope(project: &str, plan: &str) -> String {
    if project.is_empty() {
        plan.to_string()
    } else {
        format!("{project} / {plan}")
    }
}

fn log_phase_header(ui: &UI, phase: LlmPhase, project: &str, plan: &str) {
    let info = phase_info(phase);
    ui.log(&format!("\n{HR}"));
    ui.log(&format!("  ◆  {}  ·  {}", info.label, header_scope(project, plan)));
    ui.log(&format!("  {}", info.description));
    ui.log(HR);
}

fn log_commit(ui: &UI, phase_name: &str, plan: &str, result: &crate::git::CommitResult) {
    if result.committed {
        let first_line = result.message.lines().next().unwrap_or("");
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  {first_line}"));
    } else {
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  nothing to commit"));
    }
}

/// Maximum number of dirty paths to enumerate inline. The full list lives in
/// `git status` — the warning just needs enough context to alarm the user.
const DIRTY_PATH_DISPLAY_LIMIT: usize = 20;

/// After the work-phase commit, the project tree should be clean: the agent
/// is expected to have committed every source-file edit it made during the
/// work phase, and `git_commit_plan` itself just committed the plan
/// bookkeeping. A non-empty `git status` here means the agent edited files
/// without committing them — the silent failure mode that has caused the
/// loop to advance past lost work as if the backlog were empty. Surface a
/// loud warning so the user can recover before phase state advances.
///
/// Soft-failure: a transient git error here shouldn't kill the loop, so the
/// warning is best-effort. The status check itself is read-only.
fn warn_if_project_tree_dirty(ui: &UI, project_dir: &Path) {
    let dirty = match working_tree_status(project_dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    if dirty.is_empty() {
        return;
    }
    ui.log("\n  ⚠  WARNING: uncommitted changes remain in the project tree");
    ui.log("     after the work commit. The work agent likely edited files");
    ui.log("     without committing them. Review and recover before continuing:");
    for line in dirty.iter().take(DIRTY_PATH_DISPLAY_LIMIT) {
        ui.log(&format!("       {line}"));
    }
    if dirty.len() > DIRTY_PATH_DISPLAY_LIMIT {
        ui.log(&format!(
            "       ... and {} more (run `git status` for the full list)",
            dirty.len() - DIRTY_PATH_DISPLAY_LIMIT
        ));
    }
}

async fn handle_script_phase(
    phase: ScriptPhase,
    plan_dir: &Path,
    project_dir: &Path,
    headroom: usize,
    ui: &UI,
) -> Result<bool> {
    let name = plan_name(plan_dir);
    let project = project_name(&project_dir.to_string_lossy());
    let scope = header_scope(&project, &name);

    // Invariant: each script-phase handler advances `phase.md` BEFORE
    // calling `git_commit_plan`, so the phase transition is captured in
    // the same commit as that phase's other plan-state writes. Order
    // matters — writing after the commit would leave `phase.md` dirty at
    // the user-prompt points, which leaks into sibling plans in
    // multi-plan monorepos (where `warn_if_project_tree_dirty` scans the
    // whole project dir and mistakes the leak for work the agent forgot
    // to commit).
    match phase {
        ScriptPhase::GitCommitWork => {
            write_phase(plan_dir, Phase::Llm(LlmPhase::Reflect))?;
            let result = git_commit_plan(plan_dir, &name, "work")?;
            log_commit(ui, "work", &scope, &result);
            warn_if_project_tree_dirty(ui, project_dir);
            Ok(ui.confirm("Proceed to reflect phase?").await)
        }
        ScriptPhase::GitCommitReflect => {
            let skip_dream = !should_dream(plan_dir, headroom);
            if skip_dream {
                write_phase(plan_dir, Phase::Llm(LlmPhase::Triage))?;
            } else {
                write_phase(plan_dir, Phase::Llm(LlmPhase::Dream))?;
            }
            let result = git_commit_plan(plan_dir, &name, "reflect")?;
            log_commit(ui, "reflect", &scope, &result);
            if skip_dream {
                // Render a full DREAM header with a skip description so the
                // absence-of-work is as legible as the other phases.
                let info = phase_info(LlmPhase::Dream);
                ui.log(&format!("\n{HR}"));
                ui.log(&format!("  ◆  {}  ·  {scope}", info.label));
                ui.log("  Skipped — memory within headroom");
                ui.log(HR);
            }
            Ok(true)
        }
        ScriptPhase::GitCommitDream => {
            write_phase(plan_dir, Phase::Llm(LlmPhase::Triage))?;
            let result = git_commit_plan(plan_dir, &name, "dream")?;
            log_commit(ui, "dream", &scope, &result);
            Ok(true)
        }
        ScriptPhase::GitCommitTriage => {
            // Prepare for the next work cycle as part of this commit, so
            // `work-baseline` is also captured atomically. The
            // `LlmPhase::Work` entry retains a first-run fallback for
            // fresh plans that start at `work` without a preceding triage.
            //
            // `latest-session.md` is intentionally NOT touched here:
            // analyse-work overwrites it next cycle (see
            // `defaults/phases/analyse-work.md` step 8), and leaving it
            // in place through the triage commit keeps the prior
            // session's record available for operator inspection in the
            // gap between cycles.
            write_phase(plan_dir, Phase::Llm(LlmPhase::Work))?;
            git_save_work_baseline(plan_dir);
            let result = git_commit_plan(plan_dir, &name, "triage")?;
            log_commit(ui, "triage", &scope, &result);
            Ok(ui.confirm("Proceed to next work phase?").await)
        }
    }
}

pub async fn phase_loop(
    agent: Arc<dyn Agent>,
    ctx: &PlanContext,
    config: &SharedConfig,
    ui: &UI,
) -> Result<()> {
    let tokens = agent.tokens();
    let plan_dir = Path::new(&ctx.plan_dir);
    let project_dir = Path::new(&ctx.project_dir);
    let config_root = Path::new(&ctx.config_root);
    let name = plan_name(plan_dir);
    let project = project_name(&ctx.project_dir);

    if let Err(e) = agent.setup(ctx).await {
        ui.log(&format!("  ✗  Setup failed: {e}"));
    }

    loop {
        let phase = read_phase(plan_dir)?;

        match phase {
            Phase::Script(sp) => {
                if !handle_script_phase(sp, plan_dir, project_dir, config.headroom, ui).await? {
                    ui.log("\nExiting.");
                    return Ok(());
                }
                continue;
            }
            Phase::Llm(lp) => {
                let agent_id = "main";
                log_phase_header(ui, lp, &project, &name);

                // First-run fallback: in steady state, `git-commit-triage`
                // prepares `work-baseline` as part of its atomic commit.
                // On a brand-new plan that starts at `work` with no prior
                // triage, `work-baseline` doesn't exist yet — seed it so
                // analyse-work has a baseline SHA to diff against.
                if lp == LlmPhase::Work && !plan_dir.join("work-baseline").exists() {
                    git_save_work_baseline(plan_dir);
                }

                // analyse-work needs a live snapshot of the work tree so the
                // prompt can (a) show the LLM exactly what changed since the
                // baseline and (b) force it to commit or justify every path.
                // Captured at prompt-compose time so any hand-edits the user
                // made between work exit and analyse-work start are included.
                let prompt = if lp == LlmPhase::AnalyseWork {
                    let mut augmented = tokens.clone();
                    let baseline_sha = fs::read_to_string(plan_dir.join("work-baseline"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let snapshot = if baseline_sha.is_empty() {
                        "(work-baseline missing; no snapshot available)".to_string()
                    } else {
                        work_tree_snapshot(project_dir, &baseline_sha)
                    };
                    augmented.insert("WORK_TREE_STATUS".to_string(), snapshot);
                    compose_prompt(config_root, lp, ctx, &augmented)?
                } else {
                    compose_prompt(config_root, lp, ctx, &tokens)?
                };
                let tx = ui.sender();

                ui.register_agent(agent_id);

                if lp == LlmPhase::Work {
                    ui.suspend();
                    agent.invoke_interactive(&prompt, ctx).await?;
                    ui.resume();
                } else {
                    agent.invoke_headless(&prompt, ctx, lp, agent_id, tx).await?;
                }

                let new_phase = read_phase(plan_dir)?;
                if new_phase == phase {
                    ui.log(&format!("\n  ✗  Phase did not advance from {phase}. Stopping."));
                    return Ok(());
                }

                if lp == LlmPhase::Dream {
                    update_dream_baseline(plan_dir);
                }

                if lp == LlmPhase::Triage {
                    dispatch_subagents(agent.clone(), plan_dir, ui).await?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_strips_to_basename() {
        assert_eq!(project_name("/Users/x/Development/ravel-lite"), "ravel-lite");
        assert_eq!(project_name("ravel-lite"), "ravel-lite");
    }

    #[test]
    fn project_name_handles_trailing_slash() {
        // Path::file_name returns None for paths ending in `..` but a real
        // trailing slash collapses to the directory basename.
        assert_eq!(project_name("/Users/x/ravel-lite/"), "ravel-lite");
    }

    #[test]
    fn project_name_empty_when_unparseable() {
        assert_eq!(project_name(""), "");
        assert_eq!(project_name("/"), "");
    }

    #[test]
    fn header_scope_combines_project_and_plan() {
        assert_eq!(header_scope("ravel-lite", "core"), "ravel-lite / core");
    }

    #[test]
    fn header_scope_falls_back_to_plan_when_project_empty() {
        // Defensive: if project_dir somehow resolves to "" the banner still
        // identifies the plan rather than rendering "  / core" with a dangling slash.
        assert_eq!(header_scope("", "core"), "core");
    }

    #[test]
    fn write_phase_writes_marker_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write_phase(dir.path(), Phase::Llm(LlmPhase::Reflect)).unwrap();
        let contents = fs::read_to_string(dir.path().join("phase.md")).unwrap();
        assert_eq!(contents, "reflect");
    }

    #[test]
    fn write_phase_errors_when_directory_is_missing() {
        // Guard: fs::write previously returned silently via `let _ = ...`, so
        // a missing plan dir would advance the loop with stale phase state.
        // The new signature surfaces the error with the target path.
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let err = write_phase(&missing, Phase::Llm(LlmPhase::Work))
            .expect_err("write should fail on a missing directory");
        assert!(
            err.to_string().contains("phase.md"),
            "error should name the target file: {err}"
        );
    }
}
