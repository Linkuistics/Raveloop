//! Stack-based pivot mechanism for nested plan execution.
//!
//! When a plan's work phase appends a frame to `<root>/stack.yaml`,
//! the driver pushes the new top onto an in-memory `Vec<PlanContext>`
//! and runs a full cycle of the target plan before popping back.
//!
//! This module owns:
//! - `Frame` / `Stack` — the on-disk schema.
//! - `read_stack` / `write_stack` — file I/O.
//! - `validate_push` — depth cap + cycle detection + target validity.
//! - `decide_after_work` / `decide_after_cycle` — pure state transitions.
//!
//! No tokio, no async. The async driver in `phase_loop.rs` orchestrates.
//! See docs/superpowers/specs/2026-04-20-hierarchical-pivot-design.md.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Canonical form for a `Frame::pushed_at` value. Returned by the driver
/// (`sync_stack_to_disk`) and the `state push-plan` CLI verb so both write
/// identical timestamps.
pub fn push_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!("@{secs}")
}

/// Hard compile-time cap on nesting depth. Prevents runaway recursion
/// from a buggy coordinator prompt.
pub const MAX_STACK_DEPTH: usize = 5;

/// One entry in `stack.yaml`. Path is the only required field; `pushed_at`
/// and `reason` are informational and appear in the TUI / session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The on-disk stack. Ordered; `frames.last()` is the currently-executing plan.
/// A stack with `len <= 1` means "just the root"; the file is normally
/// deleted in that state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stack {
    #[serde(default)]
    pub frames: Vec<Frame>,
}

/// Read `stack.yaml` from `path`. Returns `Ok(None)` if the file is absent
/// (normal state at depth 1), `Ok(Some(stack))` on successful parse, or an
/// error that includes the path for diagnosability.
pub fn read_stack(path: &Path) -> Result<Option<Stack>> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let stack: Stack = serde_yaml::from_str(&s)
                .with_context(|| format!("Failed to parse stack.yaml at {}", path.display()))?;
            Ok(Some(stack))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => {
            Err(e).with_context(|| format!("Failed to read stack.yaml at {}", path.display()))
        }
    }
}

/// Write `stack` to `path`, creating parent directories as needed.
pub fn write_stack(path: &Path, stack: &Stack) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent for {}", path.display()))?;
    }
    let s = serde_yaml::to_string(stack).context("Failed to serialize Stack")?;
    fs::write(path, s)
        .with_context(|| format!("Failed to write stack.yaml at {}", path.display()))
}

/// Reject a push attempt that would:
/// - exceed `MAX_STACK_DEPTH` (runaway recursion guard)
/// - revisit an already-stacked plan (cycle)
/// - point at a path that isn't a valid plan directory
///
/// Path validation uses the real filesystem (existence + `phase.md` presence).
/// Returns `Err` with a one-line diagnostic on any violation.
pub fn validate_push(stack: &Stack, new_frame: &Frame) -> Result<()> {
    if stack.frames.len() >= MAX_STACK_DEPTH {
        bail!(
            "Max stack depth {} exceeded; attempted push target: {}",
            MAX_STACK_DEPTH,
            new_frame.path.display()
        );
    }
    let canonical_new = new_frame.path.canonicalize().unwrap_or_else(|_| new_frame.path.clone());
    for existing in &stack.frames {
        let canonical_existing = existing.path.canonicalize().unwrap_or_else(|_| existing.path.clone());
        if canonical_existing == canonical_new {
            bail!(
                "Cycle detected: {} already in stack",
                new_frame.path.display()
            );
        }
    }
    if !new_frame.path.is_dir() {
        bail!(
            "Invalid pivot target: {} does not exist or is not a directory",
            new_frame.path.display()
        );
    }
    let phase_file = new_frame.path.join("phase.md");
    if !phase_file.exists() {
        bail!(
            "Invalid pivot target: {} has no phase.md",
            new_frame.path.display()
        );
    }
    Ok(())
}

use crate::types::LlmPhase;

/// What the driver should do after a work phase finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextAfterWork {
    /// Normal case: agent advanced phase.md and did not touch stack.yaml
    /// (or at least did not add a new top). Run analyse-work next.
    ContinueNormalCycle,
    /// Agent advanced phase.md AND added a new top. Run the full
    /// coordinator cycle; push happens at end of git-commit-triage.
    PushAfterCycle(Frame),
    /// Agent left phase.md at `work` AND added a new top. Push immediately
    /// and run the child's cycle; coordinator's remaining phases are
    /// skipped for this cycle.
    PushImmediately(Frame),
    /// Phase did not advance and no pivot was requested. Existing
    /// "phase did not advance" error case.
    Error(String),
}

/// Decide what to do after a work phase ends.
///
/// - `phase_after`: the LLM phase read from `phase.md` after work exits.
/// - `stack_grew`: whether `stack.yaml` gained a new top frame during work.
/// - `new_top`: the newly-added top frame, if `stack_grew` is true.
///
/// Caller is responsible for validating `new_top` via `validate_push`
/// before acting on `PushImmediately` / `PushAfterCycle`.
pub fn decide_after_work(
    phase_after: LlmPhase,
    stack_grew: bool,
    new_top: Option<Frame>,
) -> NextAfterWork {
    match (phase_after == LlmPhase::Work, stack_grew) {
        (true, true) => match new_top {
            Some(f) => NextAfterWork::PushImmediately(f),
            None => NextAfterWork::Error(
                "stack_grew=true but new_top=None — caller bug".into(),
            ),
        },
        (true, false) => NextAfterWork::Error(
            "phase did not advance and no pivot was requested".into(),
        ),
        (false, true) => match new_top {
            Some(f) => NextAfterWork::PushAfterCycle(f),
            None => NextAfterWork::Error(
                "stack_grew=true but new_top=None — caller bug".into(),
            ),
        },
        (false, false) => NextAfterWork::ContinueNormalCycle,
    }
}

/// What the driver should do after a plan's full cycle ends
/// (post-git-commit-triage). Called at every cycle boundary; handles
/// the three terminal cases of a cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextAfterCycle {
    /// Continue the current (root) plan's loop at its next work phase.
    /// Only valid at depth 1.
    Continue,
    /// A new top was added during this cycle — stateful pivot takes
    /// effect now. Push the frame and enter its plan's cycle.
    Push(Frame),
    /// Current plan is nested and had nothing new to push; pop to
    /// parent and resume its next phase.
    Pop,
}

/// Decide what to do after `git-commit-triage` completes for the
/// currently-executing plan.
///
/// - `current_depth`: stack depth *including* the currently-executing plan.
///   Depth 1 means "root only".
/// - `stack_grew`: whether a new top was added during this plan's cycle.
/// - `new_top`: the newly-added frame, if any.
pub fn decide_after_cycle(
    current_depth: usize,
    stack_grew: bool,
    new_top: Option<Frame>,
) -> NextAfterCycle {
    if stack_grew {
        if let Some(f) = new_top {
            return NextAfterCycle::Push(f);
        }
    }
    if current_depth > 1 {
        NextAfterCycle::Pop
    } else {
        NextAfterCycle::Continue
    }
}

// ── Reconstruction ───────────────────────────────────────────────────────────

use crate::types::PlanContext;

/// Walk upwards from `plan_dir` looking for a `.git` directory. Returns
/// the ancestor directory that contains `.git`, or an error if none found.
fn find_project_root(plan_dir: &Path) -> Result<PathBuf> {
    let mut cur = plan_dir.canonicalize()
        .with_context(|| format!("Failed to canonicalize {}", plan_dir.display()))?;
    loop {
        if cur.join(".git").exists() {
            return Ok(cur);
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => bail!("No .git ancestor found for plan {}", plan_dir.display()),
        }
    }
}

/// Build a `PlanContext` for a pivoted plan, reusing the root's config_root.
/// Looks up the target's project_dir by walking up for a `.git` ancestor,
/// and derives dev_root as the project's parent directory.
///
/// `related_plans` is read from `<plan_dir>/related-plans.md` if present,
/// matching the behaviour of `main::run_phase_loop` at startup.
pub fn frame_to_context(frame: &Frame, config_root: &str) -> Result<PlanContext> {
    let plan_dir = frame.path.canonicalize()
        .with_context(|| format!("Failed to canonicalize frame path {}", frame.path.display()))?;

    let project_dir = find_project_root(&plan_dir)?;
    let dev_root = project_dir
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let related_plans = std::fs::read_to_string(plan_dir.join("related-plans.md"))
        .unwrap_or_default();

    Ok(PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: project_dir.to_string_lossy().to_string(),
        dev_root,
        related_plans,
        config_root: config_root.to_string(),
    })
}
