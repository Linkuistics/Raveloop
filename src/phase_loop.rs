use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::backlog_transitions::backlog_transitions;
use crate::dream::{seed_dream_baseline_if_missing, should_dream, update_dream_baseline};
use crate::format::phase_info;
use crate::git::{
    git_commit_plan, git_save_work_baseline, paths_changed_since_baseline, work_tree_snapshot,
    working_tree_status,
};
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

/// Extract the path from a `git status --porcelain` line. Lines are
/// `XY path` (2 status chars + space + path); renames are `R  old -> new`
/// and the new path is returned. Returns `None` on unparseable input —
/// callers treat that as "preserve the entry" since a conservative keep
/// beats a silent drop when the narrowing filter can't classify a line.
fn parse_porcelain_path(line: &str) -> Option<&str> {
    let rest = line.get(3..)?;
    if let Some(arrow) = rest.find(" -> ") {
        Some(rest[arrow + 4..].trim())
    } else {
        Some(rest.trim())
    }
}

/// After the work-phase commit, the project tree should be clean: the agent
/// is expected to have committed every source-file edit it made during the
/// work phase, and `git_commit_plan` itself just committed the plan
/// bookkeeping. A non-empty `git status` here means the agent edited files
/// without committing them — the silent failure mode that has caused the
/// loop to advance past lost work as if the backlog were empty. Surface a
/// loud warning so the user can recover before phase state advances.
///
/// The dirty set is narrowed to paths the work agent could plausibly have
/// touched: untracked files (new since baseline by definition) plus any
/// tracked file that differs from the work baseline per `git diff
/// --name-only <baseline>`. This filters out sibling-plan in-flight writes
/// in multi-plan monorepos where `git status` is repo-wide. If the
/// baseline is missing or the diff call fails, the narrowing is skipped
/// and the original (over-inclusive) dirty list is used — strictly more
/// noisy, never less accurate.
///
/// Soft-failure: a transient git error here shouldn't kill the loop, so
/// the warning is best-effort. The status check itself is read-only.
fn warn_if_project_tree_dirty(ui: &UI, project_dir: &Path, plan_dir: &Path) {
    let dirty = match working_tree_status(project_dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    if dirty.is_empty() {
        return;
    }

    let baseline_sha = fs::read_to_string(plan_dir.join("work-baseline"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let touched: Option<std::collections::HashSet<String>> = baseline_sha
        .as_deref()
        .and_then(|sha| paths_changed_since_baseline(project_dir, sha).ok());

    let filtered: Vec<&String> = dirty
        .iter()
        .filter(|line| {
            let Some(set) = &touched else { return true };
            if line.starts_with("??") {
                return true;
            }
            match parse_porcelain_path(line) {
                Some(path) => set.contains(path),
                None => true,
            }
        })
        .collect();

    if filtered.is_empty() {
        return;
    }

    ui.log("\n  ⚠  WARNING: uncommitted changes remain in the project tree");
    ui.log("     after the work commit. The work agent likely edited files");
    ui.log("     without committing them. Review and recover before continuing:");
    for line in filtered.iter().take(DIRTY_PATH_DISPLAY_LIMIT) {
        ui.log(&format!("       {line}"));
    }
    if filtered.len() > DIRTY_PATH_DISPLAY_LIMIT {
        ui.log(&format!(
            "       ... and {} more (run `git status` for the full list)",
            filtered.len() - DIRTY_PATH_DISPLAY_LIMIT
        ));
    }
}

/// Append the freshly-written `latest-session.yaml` record to
/// `session-log.yaml` so each cycle's narrative accumulates as a durable
/// audit trail.
///
/// `latest-session.yaml` is overwritten by analyse-work every cycle;
/// without this mirror write, prior sessions are lost. Runner-side on
/// purpose — mechanical file plumbing belongs here, not in a phase
/// prompt.
///
/// Idempotent on session id: if the log already contains a record whose
/// id matches `latest-session.yaml`'s id (e.g. a crash between this
/// call and `write_phase` forced a retry of `GitCommitWork`), the
/// second call is a no-op. This is strictly stronger than the earlier
/// tail-string check: a later manual edit to the log can't regress
/// the invariant.
///
/// Missing `latest-session.yaml` is also a no-op — the first work
/// cycle of a fresh plan has no session record to propagate. Analyse-
/// work is expected to produce the file on every real cycle.
fn append_session_log(plan_dir: &Path) -> Result<()> {
    crate::state::session_log::append_latest_to_log(plan_dir)
        .with_context(|| {
            format!(
                "Failed to append latest-session.yaml to session-log.yaml at {}",
                plan_dir.display()
            )
        })?;
    Ok(())
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
            append_session_log(plan_dir)?;
            write_phase(plan_dir, Phase::Llm(LlmPhase::Reflect))?;
            let result = git_commit_plan(plan_dir, &name, "work")?;
            log_commit(ui, "work", &scope, &result);
            warn_if_project_tree_dirty(ui, project_dir, plan_dir);
            Ok(true)
        }
        ScriptPhase::GitCommitReflect => {
            // First-run fallback for plans created before `dream-baseline`
            // was seeded by plan-creation. Without this, `should_dream`
            // returns `false` on every cycle (missing-file short-circuit),
            // and `update_dream_baseline` never fires (it runs only after
            // a dream phase) — a permanent deadlock. Seeding to the
            // current word count means the first dream triggers after
            // memory grows by `headroom` from here, matching the
            // post-dream steady state.
            seed_dream_baseline_if_missing(plan_dir);
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
            // `latest-session.md` is intentionally NOT touched here:
            // analyse-work overwrites it next cycle (see
            // `defaults/phases/analyse-work.md` step 8), and leaving it
            // in place through the triage commit keeps the prior
            // session's record available for operator inspection in the
            // gap between cycles.
            write_phase(plan_dir, Phase::Llm(LlmPhase::Work))?;

            // Commit triage mutations first so HEAD advances to the
            // triage commit. Only after this does `git rev-parse HEAD`
            // yield the SHA the next work phase should diff against.
            // Saving the baseline before this commit (the prior
            // arrangement) captured the reflect commit's SHA, which
            // conflated the previous cycle's triage mutations into
            // `{{BACKLOG_TRANSITIONS}}` on the next analyse-work.
            let result = git_commit_plan(plan_dir, &name, "triage")?;
            log_commit(ui, "triage", &scope, &result);

            // Baseline must land in a commit (not float in the working
            // tree) so `warn_if_project_tree_dirty` sees a clean subtree
            // at the post-cycle user prompt. A follow-on commit is
            // simpler than `git commit --amend`, which would orphan the
            // pre-amend SHA that `work-baseline` names.
            git_save_work_baseline(plan_dir);
            let baseline_result = git_commit_plan(plan_dir, &name, "save-work-baseline")?;
            log_commit(ui, "save-work-baseline", &scope, &baseline_result);

            // Exit phase_loop after one full cycle. Whether another
            // cycle starts — and, in multi-plan mode, which plan runs
            // next — is the outer loop's decision, not this function's.
            Ok(false)
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
                    // Script-phase handler signalled end-of-cycle. Today
                    // that's only `GitCommitTriage` finishing one full
                    // phase cycle; callers decide what happens next.
                    return Ok(());
                }
                continue;
            }
            Phase::Llm(lp) => {
                let agent_id = "main";
                crate::term_title::set_title(&project, &name, lp.as_str());
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
                    // Backlog delta since baseline: status flips, results
                    // additions, added/deleted tasks. Computed here rather
                    // than asked of the LLM because it's a pure YAML diff —
                    // the "Never do in an LLM what you can do in code" rule.
                    let transitions = backlog_transitions(plan_dir, &baseline_sha);
                    augmented.insert("BACKLOG_TRANSITIONS".to_string(), transitions);
                    compose_prompt(config_root, lp, ctx, &augmented)?
                } else {
                    compose_prompt(config_root, lp, ctx, &tokens)?
                };
                let tx = ui.sender();

                ui.register_agent(agent_id);

                if lp == LlmPhase::Work {
                    ui.suspend().await;
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

/// Top-level entry point for single-plan `ravel-lite run`. Repeatedly
/// invokes `phase_loop` (which now exits after one full cycle), asking
/// the user between cycles whether to continue. The prompt used to live
/// inside `handle_script_phase(GitCommitTriage)`; it moved out here so
/// multi-plan mode can run one cycle and return to its own survey-based
/// selection loop without a spurious confirm in between.
pub async fn run_single_plan(
    agent: Arc<dyn Agent>,
    ctx: PlanContext,
    config: &SharedConfig,
    ui: &UI,
) -> Result<()> {
    loop {
        phase_loop(agent.clone(), &ctx, config, ui).await?;
        if !ui.confirm("Proceed to next work phase?").await {
            ui.log("\nExiting.");
            return Ok(());
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
    fn parse_porcelain_path_handles_modified_file() {
        assert_eq!(parse_porcelain_path(" M src/foo.rs"), Some("src/foo.rs"));
        assert_eq!(parse_porcelain_path("M  src/foo.rs"), Some("src/foo.rs"));
    }

    #[test]
    fn parse_porcelain_path_handles_untracked() {
        assert_eq!(parse_porcelain_path("?? new.rs"), Some("new.rs"));
    }

    #[test]
    fn parse_porcelain_path_returns_new_name_for_rename() {
        assert_eq!(
            parse_porcelain_path("R  old/path.rs -> new/path.rs"),
            Some("new/path.rs")
        );
    }

    #[test]
    fn parse_porcelain_path_returns_none_on_too_short_input() {
        assert_eq!(parse_porcelain_path(""), None);
        assert_eq!(parse_porcelain_path("ab"), None);
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
