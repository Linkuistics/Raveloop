use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::dream::{seed_dream_baseline_if_missing, should_dream, update_dream_baseline};
use crate::format::phase_info;
use crate::git::{
    git_commit_plan, git_save_work_baseline, paths_changed_since_baseline, work_tree_snapshot,
    working_tree_status,
};
use crate::pivot;
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

/// Read the raw phase string from `phase.md` for title-display purposes.
///
/// Deliberately bypasses `Phase::parse` — a decorative read must tolerate
/// a momentarily-absent or malformed file without panicking. Used from
/// `run_stack` push/pop sites where the LLM arm (which carries `lp`
/// directly) has not yet been entered for the new top-of-stack plan.
fn raw_phase_label(plan_dir: &Path) -> String {
    fs::read_to_string(plan_dir.join("phase.md"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Update the terminal title for a plan context. Reads `phase.md`
/// directly — callers here don't yet know which `LlmPhase` the next
/// iteration will dispatch, and script phases have no `LlmPhase` at all.
fn set_title_for_context(ctx: &PlanContext) {
    crate::term_title::set_title(
        &project_name(&ctx.project_dir),
        &plan_name(Path::new(&ctx.plan_dir)),
        &raw_phase_label(Path::new(&ctx.plan_dir)),
    );
}

/// Render the stack path for phase-header display: basenames joined by ` → `.
///
/// Single-plan stacks render as just the plan's basename (unchanged from
/// pre-pivot Ravel-Lite behaviour). Nested stacks show every plan in the
/// stack — `coord → sub-F → sub-F-sub1`.
pub fn format_breadcrumb(stack_paths: &[std::path::PathBuf]) -> String {
    stack_paths
        .iter()
        .map(|p| plan_name(p))
        .collect::<Vec<_>>()
        .join(" → ")
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

#[allow(dead_code)]
fn log_phase_header(ui: &UI, phase: LlmPhase, project: &str, plan: &str) {
    let info = phase_info(phase);
    ui.log(&format!("\n{HR}"));
    ui.log(&format!("  ◆  {}  ·  {}", info.label, header_scope(project, plan)));
    ui.log(&format!("  {}", info.description));
    ui.log(HR);
}

fn log_phase_header_with_breadcrumb(ui: &UI, phase: LlmPhase, project: &str, breadcrumb: &str) {
    let info = phase_info(phase);
    ui.log(&format!("\n{HR}"));
    ui.log(&format!("  ◆  {}  ·  {}", info.label, header_scope(project, breadcrumb)));
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

/// Append the freshly-written `latest-session.md` entry to `session-log.md`
/// so each cycle's narrative accumulates as a durable audit trail.
///
/// `latest-session.md` is overwritten by analyse-work every cycle; without
/// this mirror write, prior sessions are lost. Runner-side on purpose —
/// mechanical file plumbing belongs here, not in a phase prompt.
///
/// Idempotent: if the entry is already the tail of the log (e.g. a crash
/// between this call and `write_phase` forced a retry of `GitCommitWork`),
/// the second call is a no-op.
fn append_session_log(plan_dir: &Path) -> Result<()> {
    let latest_path = plan_dir.join("latest-session.md");
    let log_path = plan_dir.join("session-log.md");

    let entry = fs::read_to_string(&latest_path).unwrap_or_default();
    let entry = entry.trim();
    if entry.is_empty() {
        return Ok(());
    }

    let existing = fs::read_to_string(&log_path)
        .unwrap_or_else(|_| String::from("# Session Log\n"));

    if existing.trim_end().ends_with(entry) {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push('\n');
    updated.push_str(entry);
    updated.push('\n');

    fs::write(&log_path, updated)
        .with_context(|| format!("Failed to append session-log.md at {}", log_path.display()))
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

#[allow(dead_code)]
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

/// Write the in-memory stack to disk, or delete the file if the stack
/// is at depth 1 (root only).
fn sync_stack_to_disk(stack_path: &Path, stack: &[PlanContext]) -> Result<()> {
    if stack.len() <= 1 {
        if stack_path.exists() {
            std::fs::remove_file(stack_path)
                .with_context(|| format!("Failed to remove {}", stack_path.display()))?;
        }
        return Ok(());
    }
    let frames: Vec<pivot::Frame> = stack
        .iter()
        .map(|ctx| pivot::Frame {
            path: std::path::PathBuf::from(&ctx.plan_dir),
            pushed_at: Some(pivot::push_timestamp()),
            reason: None,
        })
        .collect();
    pivot::write_stack(stack_path, &pivot::Stack { frames })
}

/// Read the current on-disk stack frame count. Returns 0 if the file is absent.
fn on_disk_stack_len(stack_path: &Path) -> usize {
    match pivot::read_stack(stack_path) {
        Ok(Some(s)) => s.frames.len(),
        _ => 0,
    }
}

/// Read the new top frame from stack.yaml (the last frame), if the file exists
/// and has at least `min_len` frames.
fn on_disk_new_top(stack_path: &Path, min_len: usize) -> Option<pivot::Frame> {
    match pivot::read_stack(stack_path) {
        Ok(Some(s)) if s.frames.len() >= min_len => s.frames.into_iter().last(),
        _ => None,
    }
}

/// Build the `pivot::Stack` snapshot of the in-memory driver stack (for
/// validate_push and decide_after_cycle calls).
fn stack_snapshot(stack: &[PlanContext]) -> pivot::Stack {
    pivot::Stack {
        frames: stack
            .iter()
            .map(|ctx| pivot::Frame {
                path: std::path::PathBuf::from(&ctx.plan_dir),
                pushed_at: None,
                reason: None,
            })
            .collect(),
    }
}

/// Push a new frame onto the in-memory stack after validation, log a
/// breadcrumb, and sync the stack file to disk. Returns the new context.
fn do_push(
    stack: &mut Vec<PlanContext>,
    pending_push: &mut Vec<Option<pivot::Frame>>,
    stack_path: &Path,
    new_frame: pivot::Frame,
    ui: &UI,
) -> Result<()> {
    pivot::validate_push(&stack_snapshot(stack), &new_frame)?;
    let new_ctx = pivot::frame_to_context(&new_frame, &stack.last().unwrap().config_root)?;
    let target_name = plan_name(&new_frame.path);
    let reason = new_frame.reason.as_deref().unwrap_or("");
    if reason.is_empty() {
        ui.log(&format!("\n▷▷ PUSH · {target_name}"));
    } else {
        ui.log(&format!("\n▷▷ PUSH · {target_name} · \"{reason}\""));
    }
    stack.push(new_ctx);
    pending_push.push(None);
    sync_stack_to_disk(stack_path, stack)?;
    set_title_for_context(stack.last().expect("just pushed"));
    Ok(())
}

/// Compose the LLM prompt for a phase, injecting the work-tree snapshot for
/// `AnalyseWork`. Mirrors the equivalent logic in the single-plan `phase_loop`.
fn build_prompt(
    config_root: &Path,
    lp: LlmPhase,
    ctx: &PlanContext,
    tokens: &std::collections::HashMap<String, String>,
    plan_dir: &Path,
    project_dir: &Path,
) -> Result<String> {
    if lp == LlmPhase::AnalyseWork {
        let mut aug = tokens.clone();
        let sha = fs::read_to_string(plan_dir.join("work-baseline"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let snap = if sha.is_empty() {
            "(work-baseline missing; no snapshot available)".to_string()
        } else {
            work_tree_snapshot(project_dir, &sha)
        };
        aug.insert("WORK_TREE_STATUS".to_string(), snap);
        compose_prompt(config_root, lp, ctx, &aug)
    } else {
        compose_prompt(config_root, lp, ctx, tokens)
    }
}

/// Stack-aware phase loop entry point.
///
/// For single-plan invocations (the common case today), this behaves
/// identically to the pre-pivot `phase_loop` function.
///
/// When the running plan's work phase modifies `<root>/stack.yaml` and
/// leaves `phase.md` at `work`, the driver short-circuits into the child
/// plan's cycle. See docs/superpowers/specs/2026-04-20-hierarchical-pivot-design.md.
pub async fn run_stack(
    agent: Arc<dyn Agent>,
    root_ctx: PlanContext,
    config: &SharedConfig,
    ui: &UI,
) -> Result<()> {
    let stack_path = std::path::PathBuf::from(&root_ctx.plan_dir).join("stack.yaml");
    let mut stack: Vec<PlanContext> = match pivot::read_stack(&stack_path)? {
        Some(s) if !s.frames.is_empty() => s.frames.iter()
            .map(|f| pivot::frame_to_context(f, &root_ctx.config_root))
            .collect::<Result<Vec<_>>>()?,
        _ => vec![root_ctx.clone()],
    };
    let mut pending: Vec<Option<pivot::Frame>> = vec![None; stack.len()];
    // Setup runs against the originally-invoked plan. On startup-resume,
    // stack.last() would be the deepest nested frame, which is wrong for setup.
    if let Err(e) = agent.setup(&root_ctx).await {
        ui.log(&format!("  ✗  Setup failed: {e}"));
    }

    loop {
        let top = stack.last().cloned().expect("stack never empty");
        let plan_dir = std::path::PathBuf::from(&top.plan_dir);
        let project_dir = std::path::PathBuf::from(&top.project_dir);
        let config_root = std::path::PathBuf::from(&top.config_root);
        let depth = stack.len();

        match read_phase(&plan_dir)? {
            Phase::Script(sp) => {
                let eoc = sp == ScriptPhase::GitCommitTriage;
                let ok = handle_script_phase(sp, &plan_dir, &project_dir, config.headroom, ui).await?;
                if !ok {
                    // Nested plan: user declined confirm → pop to parent and resume,
                    // keeping the stack.yaml cleanup invariant.
                    if depth > 1 {
                        let popped_name = plan_name(&plan_dir);
                        stack.pop();
                        pending.pop();
                        sync_stack_to_disk(&stack_path, &stack)?;
                        let parent_name = plan_name(&std::path::PathBuf::from(&stack.last().unwrap().plan_dir));
                        ui.log(&format!("\n◁◁ POP · {popped_name} → {parent_name}"));
                        set_title_for_context(stack.last().expect("stack non-empty after pop"));
                        continue;
                    }
                    ui.log("\nExiting.");
                    return Ok(());
                }
                if eoc {
                    let pp = pending[depth - 1].take();
                    let grew = pp.is_some();
                    match pivot::decide_after_cycle(depth, grew, pp) {
                        pivot::NextAfterCycle::Continue => {}
                        pivot::NextAfterCycle::Pop => {
                            let popped_name = plan_name(&plan_dir);
                            stack.pop();
                            pending.pop();
                            sync_stack_to_disk(&stack_path, &stack)?;
                            let parent_name = plan_name(&std::path::PathBuf::from(&stack.last().unwrap().plan_dir));
                            ui.log(&format!("\n◁◁ POP · {popped_name} → {parent_name}"));
                            set_title_for_context(stack.last().expect("stack non-empty after pop"));
                        }
                        pivot::NextAfterCycle::Push(f) => { do_push(&mut stack, &mut pending, &stack_path, f, ui)?; }
                    }
                }
            }
            Phase::Llm(lp) => {
                let stack_paths: Vec<std::path::PathBuf> = stack.iter()
                    .map(|c| std::path::PathBuf::from(&c.plan_dir))
                    .collect();
                let breadcrumb = format_breadcrumb(&stack_paths);
                crate::term_title::set_title(
                    &project_name(&top.project_dir),
                    &plan_name(&plan_dir),
                    lp.as_str(),
                );
                log_phase_header_with_breadcrumb(ui, lp, &project_name(&top.project_dir), &breadcrumb);
                if lp == LlmPhase::Work && !plan_dir.join("work-baseline").exists() {
                    git_save_work_baseline(&plan_dir);
                }
                let tokens = agent.tokens();
                let prompt = build_prompt(&config_root, lp, &top, &tokens, &plan_dir, &project_dir)?;
                ui.register_agent("main");

                if lp == LlmPhase::Work {
                    let pre_len = on_disk_stack_len(&stack_path);
                    ui.suspend().await;
                    agent.invoke_interactive(&prompt, &top).await?;
                    ui.resume();
                    let phase_after = read_phase(&plan_dir)?;
                    let post_len = on_disk_stack_len(&stack_path);
                    let grew = post_len > pre_len;
                    let new_top = grew.then(|| on_disk_new_top(&stack_path, post_len)).flatten();
                    let lp_after = match phase_after { Phase::Llm(p) => p, Phase::Script(_) => LlmPhase::AnalyseWork };
                    match pivot::decide_after_work(lp_after, grew, new_top) {
                        pivot::NextAfterWork::ContinueNormalCycle => {}
                        pivot::NextAfterWork::PushAfterCycle(f) => { pending[depth - 1] = Some(f); }
                        pivot::NextAfterWork::PushImmediately(f) => { do_push(&mut stack, &mut pending, &stack_path, f, ui)?; continue; }
                        pivot::NextAfterWork::Error(msg) => { ui.log(&format!("\n  ✗  {msg}. Stopping.")); return Ok(()); }
                    }
                } else {
                    agent.invoke_headless(&prompt, &top, lp, "main", ui.sender()).await?;
                    if read_phase(&plan_dir)? == Phase::Llm(lp) {
                        ui.log(&format!("\n  ✗  Phase did not advance from {lp}. Stopping."));
                        return Ok(());
                    }
                    if lp == LlmPhase::Dream { update_dream_baseline(&plan_dir); }
                    if lp == LlmPhase::Triage { dispatch_subagents(agent.clone(), &plan_dir, ui).await?; }
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
