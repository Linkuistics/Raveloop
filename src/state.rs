//! CLI-facing plan-state mutations used by phase prompts.
//!
//! Two verbs: `set-phase` (rewrite `<plan>/phase.md`) and `push-plan`
//! (append a frame to `<plan>/stack.yaml`). Both exist so LLM prompts
//! can mutate plan state via one `Bash(ravel-lite state *)` allowlist
//! entry instead of a `Read` + `Write` tool-call pair per transition.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pivot::{self, Frame};
use crate::types::Phase;

/// Enumerated for error messages so a typo'd phase string comes back
/// with an actionable list of accepted values.
const VALID_PHASES: &[&str] = &[
    "work",
    "analyse-work",
    "reflect",
    "dream",
    "triage",
    "git-commit-work",
    "git-commit-reflect",
    "git-commit-dream",
    "git-commit-triage",
];

pub fn run_set_phase(plan_dir: &Path, phase: &str) -> Result<()> {
    if Phase::parse(phase).is_none() {
        bail!(
            "Invalid phase '{phase}'. Accepted values: {}",
            VALID_PHASES.join(", ")
        );
    }
    let target = plan_dir.join("phase.md");
    if !target.exists() {
        bail!(
            "phase.md not found at {}. set-phase refuses to create a new plan dir.",
            target.display()
        );
    }
    atomic_write(&target, phase.as_bytes())
}

/// Write `bytes` to `path` via tmp-file + rename, so a concurrent reader
/// (e.g. the driver sampling phase.md between prompt turns) never sees
/// a truncated file. The tmp file sits next to the target so the rename
/// stays on-device.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn run_push_plan(
    plan_dir: &Path,
    target_plan_dir: &Path,
    reason: Option<String>,
) -> Result<()> {
    // Validate the coordinator's own plan-dir before touching anything.
    // validate_push below covers the target — this guards against
    // typo'd <plan-dir> silently creating a stack.yaml under a
    // non-plan directory.
    if !plan_dir.join("phase.md").exists() {
        bail!(
            "phase.md not found at {}. push-plan refuses to operate on a non-plan directory.",
            plan_dir.join("phase.md").display()
        );
    }
    let stack_path = plan_dir.join("stack.yaml");
    let mut stack = pivot::read_stack(&stack_path)?.unwrap_or_default();
    // First-pivot shape: the coordinator's own frame is implicit until
    // the first push — seed it now so the on-disk stack always records
    // the full ancestry.
    if stack.frames.is_empty() {
        stack.frames.push(Frame {
            path: plan_dir.to_path_buf(),
            pushed_at: None,
            reason: None,
        });
    }
    let new_frame = Frame {
        path: target_plan_dir.to_path_buf(),
        pushed_at: Some(pivot::push_timestamp()),
        reason,
    };
    pivot::validate_push(&stack, &new_frame)?;
    stack.frames.push(new_frame);
    pivot::write_stack(&stack_path, &stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::pivot;

    /// Minimal plan: a directory containing a `phase.md` at the given
    /// phase. Used to build coordinator + child fixtures quickly.
    fn make_plan(root: &Path, name: &str, phase: &str) -> std::path::PathBuf {
        let p = root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("phase.md"), phase).unwrap();
        p
    }

    #[test]
    fn push_plan_rejects_cycle_when_target_already_in_stack() {
        let tmp = TempDir::new().unwrap();
        let coordinator = make_plan(tmp.path(), "coord", "work");
        let child = make_plan(tmp.path(), "child", "work");

        run_push_plan(&coordinator, &child, None).unwrap();
        // Pushing the same child again would re-enter a plan already
        // mid-execution — that's a cycle, guarded by validate_push.
        let err = run_push_plan(&coordinator, &child, None).unwrap_err();
        assert!(format!("{err:#}").to_lowercase().contains("cycle"));

        let stack = pivot::read_stack(&coordinator.join("stack.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(
            stack.frames.len(),
            2,
            "failed push must not alter on-disk stack"
        );
    }

    #[test]
    fn push_plan_rejects_target_without_phase_md() {
        let tmp = TempDir::new().unwrap();
        let coordinator = make_plan(tmp.path(), "coord", "work");
        // Directory exists but has no phase.md — not a valid plan.
        let not_a_plan = tmp.path().join("not-a-plan");
        std::fs::create_dir_all(&not_a_plan).unwrap();

        let err = run_push_plan(&coordinator, &not_a_plan, None).unwrap_err();
        assert!(format!("{err:#}").contains("phase.md"));
        assert!(
            !coordinator.join("stack.yaml").exists(),
            "failed push must not create stack.yaml"
        );
    }

    #[test]
    fn push_plan_rejects_when_coordinator_has_no_phase_md() {
        let tmp = TempDir::new().unwrap();
        // Coordinator directory exists but has no phase.md — the caller
        // gave us a bogus plan-dir path. Refuse rather than seed a
        // stack.yaml inside what would become a malformed plan.
        let coord = tmp.path().join("coord");
        std::fs::create_dir_all(&coord).unwrap();
        let child = make_plan(tmp.path(), "child", "work");

        let err = run_push_plan(&coord, &child, None).unwrap_err();
        assert!(format!("{err:#}").contains("phase.md"));
    }

    #[test]
    fn push_plan_appends_to_existing_multi_frame_stack() {
        let tmp = TempDir::new().unwrap();
        let coordinator = make_plan(tmp.path(), "coord", "work");
        let child1 = make_plan(tmp.path(), "child-1", "work");
        let child2 = make_plan(tmp.path(), "child-2", "work");

        run_push_plan(&coordinator, &child1, None).unwrap();
        run_push_plan(&coordinator, &child2, Some("second".into())).unwrap();

        let stack = pivot::read_stack(&coordinator.join("stack.yaml"))
            .unwrap()
            .unwrap();
        let paths: Vec<_> = stack.frames.iter().map(|f| &f.path).collect();
        assert_eq!(paths, vec![&coordinator, &child1, &child2]);
        assert_eq!(stack.frames[2].reason.as_deref(), Some("second"));
    }

    #[test]
    fn push_plan_creates_stack_with_root_and_target_when_absent() {
        let tmp = TempDir::new().unwrap();
        let coordinator = make_plan(tmp.path(), "coord", "work");
        let child = make_plan(tmp.path(), "child", "work");

        run_push_plan(&coordinator, &child, Some("because".into())).unwrap();

        let stack = pivot::read_stack(&coordinator.join("stack.yaml"))
            .unwrap()
            .expect("stack.yaml must exist after push");
        assert_eq!(stack.frames.len(), 2, "expected [root, target]");
        assert_eq!(stack.frames[0].path, coordinator);
        assert_eq!(stack.frames[1].path, child);
        assert_eq!(stack.frames[1].reason.as_deref(), Some("because"));
        assert!(
            stack.frames[1].pushed_at.is_some(),
            "target frame must carry a pushed_at timestamp"
        );
    }

    #[test]
    fn set_phase_writes_valid_llm_phase_to_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let content = std::fs::read_to_string(plan.join("phase.md")).unwrap();
        assert_eq!(content.trim(), "analyse-work");
    }

    #[test]
    fn set_phase_rejects_missing_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        // Deliberately no phase.md — simulates a typo'd plan-dir arg.
        let err = run_set_phase(plan, "reflect").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("phase.md"), "error must name phase.md: {msg}");
        assert!(!plan.join("phase.md").exists(), "must not silently create phase.md");
    }

    #[test]
    fn set_phase_rejects_typo_and_lists_valid_phases() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();

        let err = run_set_phase(plan, "analyze-work").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("analyze-work"), "error must include the bad input: {msg}");
        // Enumeration of valid phase names in the error lets the LLM
        // self-correct without a second round-trip.
        for valid in ["work", "analyse-work", "reflect", "dream", "triage",
                      "git-commit-work", "git-commit-reflect", "git-commit-dream",
                      "git-commit-triage"] {
            assert!(msg.contains(valid), "error must list '{valid}': {msg}");
        }

        let content = std::fs::read_to_string(plan.join("phase.md")).unwrap();
        assert_eq!(content.trim(), "work", "phase.md must be unchanged on error");
    }
}
