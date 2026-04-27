//! CLI-facing plan-state mutations used by phase prompts.
//!
//! One verb today: `set-phase` (rewrite `<plan>/phase.md`). It exists so
//! LLM prompts can mutate plan state via one `Bash(ravel-lite state *)`
//! allowlist entry instead of a `Read` + `Write` tool-call pair per
//! transition.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::dream::seed_dream_word_count_if_missing;
use crate::state::filenames::PHASE_FILENAME;
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
    let target = plan_dir.join(PHASE_FILENAME);
    if !target.exists() {
        bail!(
            "{PHASE_FILENAME} not found at {}. set-phase refuses to create a new plan dir.",
            target.display()
        );
    }
    atomic_write(&target, phase.as_bytes())?;
    // Every LLM phase transition funnels through this CLI verb, so
    // seeding here guarantees a dream-word-count file exists on any
    // plan the driver touches — including coordinator plans that
    // never reach the `GitCommitReflect` handler, and plans whose
    // file was lost between cycles. Also doubles as the migration
    // point for plans that pre-date the rename from `dream-baseline`
    // to `dream-word-count`. Idempotent no-op in steady state.
    seed_dream_word_count_if_missing(plan_dir);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::filenames::DREAM_WORD_COUNT_FILENAME;
    use tempfile::TempDir;

    #[test]
    fn set_phase_writes_valid_llm_phase_to_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let content = std::fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap();
        assert_eq!(content.trim(), "analyse-work");
    }

    #[test]
    fn set_phase_rejects_missing_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        // Deliberately no phase.md — simulates a typo'd plan-dir arg.
        let err = run_set_phase(plan, "reflect").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(PHASE_FILENAME), "error must name {PHASE_FILENAME}: {msg}");
        assert!(!plan.join(PHASE_FILENAME).exists(), "must not silently create phase.md");
    }

    #[test]
    fn set_phase_seeds_dream_word_count_when_missing() {
        // Defense-in-depth: any LLM phase transition must leave the
        // plan with a dream-word-count file on disk. Coordinator plans
        // never reach `GitCommitReflect`, so this is their only seed
        // path.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();
        assert!(!plan.join(DREAM_WORD_COUNT_FILENAME).exists());

        run_set_phase(plan, "analyse-work").unwrap();

        let baseline = std::fs::read_to_string(plan.join(DREAM_WORD_COUNT_FILENAME)).unwrap();
        assert_eq!(baseline.trim(), "0");
    }

    #[test]
    fn set_phase_preserves_existing_dream_word_count() {
        // Idempotence: the seed must not clobber an already-written
        // value. Otherwise every phase transition would reset progress
        // toward the dream threshold.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();
        std::fs::write(plan.join(DREAM_WORD_COUNT_FILENAME), "1234").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let baseline = std::fs::read_to_string(plan.join(DREAM_WORD_COUNT_FILENAME)).unwrap();
        assert_eq!(baseline.trim(), "1234");
    }

    #[test]
    fn set_phase_migrates_legacy_dream_baseline_word_count_file() {
        // Plans created before the rename store the word count at
        // `dream-baseline`. set-phase is the earliest call site that
        // funnels every cycle, so it's the natural migration point —
        // the file is moved to `dream-word-count` and removed from
        // its old name to free that name for the SHA writer.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();
        std::fs::write(plan.join("dream-baseline"), "5678").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        assert_eq!(
            std::fs::read_to_string(plan.join(DREAM_WORD_COUNT_FILENAME)).unwrap().trim(),
            "5678"
        );
        assert!(!plan.join("dream-baseline").exists());
    }

    #[test]
    fn set_phase_rejects_typo_and_lists_valid_phases() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();

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

        let content = std::fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap();
        assert_eq!(content.trim(), "work", "phase.md must be unchanged on error");
    }
}
