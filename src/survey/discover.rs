// src/survey/discover.rs
//
// Plan loading: read a single plan directory's state files and
// classify by project (the basename of the nearest ancestor
// containing `.git`). Used by `run_survey` to bundle plans
// individually named on the CLI rather than walking a plan root.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::git::find_project_root;

/// A single plan's state, bundled for inclusion in the survey prompt.
/// `input_hash` is a SHA-256 over the four state files
/// (`phase.md` + `backlog.md` + `memory.md` + `related-plans.md`),
/// computed at load time and injected into the survey response's
/// `PlanRow` post-parse. `session-log.md` is deliberately excluded:
/// it's append-only and would defeat change detection.
#[derive(Debug)]
pub struct PlanSnapshot {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub backlog: Option<String>,
    pub memory: Option<String>,
    pub input_hash: String,
}

/// Derive the project name for a plan by walking up from the plan's
/// own directory to the nearest ancestor containing `.git`, then
/// taking that ancestor's basename. Hard errors if no `.git` is found
/// above the plan — plans outside a git repo are unsupported.
fn project_name_for_plan(plan_path: &Path) -> Result<String> {
    let git_root = find_project_root(plan_path)?;
    Path::new(&git_root)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .with_context(|| format!("Could not derive project name from git root {git_root}"))
}

/// Load a single plan directory's state into a `PlanSnapshot`. A
/// directory is a plan iff it contains a `phase.md` file; this matches
/// the convention used everywhere else in Ravel-Lite. The project
/// name is the basename of the nearest ancestor containing `.git`, so
/// plans from different repos are labelled correctly even when
/// co-located on the CLI.
pub fn load_plan(plan_dir: &Path) -> Result<PlanSnapshot> {
    let phase_file = plan_dir.join("phase.md");
    if !phase_file.exists() {
        anyhow::bail!(
            "{} is not a plan directory (no phase.md found)",
            plan_dir.display()
        );
    }

    let plan = plan_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unnamed)")
        .to_string();
    let project = project_name_for_plan(plan_dir)?;
    let phase_raw = fs::read_to_string(&phase_file)
        .with_context(|| format!("Failed to read {}", phase_file.display()))?;
    let phase = phase_raw.trim().to_string();
    let backlog = fs::read_to_string(plan_dir.join("backlog.md")).ok();
    let memory = fs::read_to_string(plan_dir.join("memory.md")).ok();
    let related_plans = fs::read_to_string(plan_dir.join("related-plans.md")).ok();

    let input_hash = compute_input_hash(
        &phase_raw,
        backlog.as_deref(),
        memory.as_deref(),
        related_plans.as_deref(),
    );

    Ok(PlanSnapshot {
        project,
        plan,
        phase,
        backlog,
        memory,
        input_hash,
    })
}

/// SHA-256 hex digest over the four plan-state files whose contents
/// define the survey input. The hash uses length-prefixed sections so
/// that a byte swap between two files cannot produce a hash collision
/// with a different file layout. `None` is encoded as a distinct
/// length prefix (the literal string `absent`) so "empty file present"
/// and "file absent" hash differently — that distinction matters
/// because it's user-visible in the survey's `(missing)` marker.
fn compute_input_hash(
    phase: &str,
    backlog: Option<&str>,
    memory: Option<&str>,
    related_plans: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hash_section(&mut hasher, "phase", Some(phase));
    hash_section(&mut hasher, "backlog", backlog);
    hash_section(&mut hasher, "memory", memory);
    hash_section(&mut hasher, "related-plans", related_plans);
    hex_encode(&hasher.finalize())
}

fn hash_section(hasher: &mut Sha256, label: &str, content: Option<&str>) {
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    match content {
        Some(s) => {
            hasher.update(b"present\0");
            hasher.update((s.len() as u64).to_le_bytes());
            hasher.update(s.as_bytes());
        }
        None => {
            hasher.update(b"absent\0");
        }
    }
    hasher.update(b"\0");
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Create a fake git project at `project_dir` with an empty `.git`
    /// directory — `find_project_root` only checks for `.git`'s
    /// existence, not its validity, so this is sufficient for tests.
    fn mark_as_git_project(project_dir: &Path) {
        fs::create_dir_all(project_dir.join(".git")).unwrap();
    }

    fn write_plan_files(
        plan_dir: &Path,
        phase: &str,
        backlog: Option<&str>,
        memory: Option<&str>,
        related_plans: Option<&str>,
    ) {
        fs::create_dir_all(plan_dir).unwrap();
        fs::write(plan_dir.join("phase.md"), phase).unwrap();
        if let Some(b) = backlog {
            fs::write(plan_dir.join("backlog.md"), b).unwrap();
        }
        if let Some(m) = memory {
            fs::write(plan_dir.join("memory.md"), m).unwrap();
        }
        if let Some(r) = related_plans {
            fs::write(plan_dir.join("related-plans.md"), r).unwrap();
        }
    }

    #[test]
    fn load_plan_reads_phase_backlog_and_memory() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("plan-a");
        mark_as_git_project(&project);
        write_plan_files(&plan_dir, "work\n", Some("# b\n"), Some("# m\n"), None);

        let snapshot = load_plan(&plan_dir).unwrap();
        assert_eq!(snapshot.project, "p");
        assert_eq!(snapshot.plan, "plan-a");
        assert_eq!(snapshot.phase, "work");
        assert_eq!(snapshot.backlog.as_deref(), Some("# b\n"));
        assert_eq!(snapshot.memory.as_deref(), Some("# m\n"));
    }

    #[test]
    fn load_plan_derives_project_from_ancestor_git_dir() {
        // Project layout:
        //   tmp/my-project/.git          <- project marker
        //   tmp/my-project/LLM_STATE/plan-x/phase.md
        // The project name should be "my-project", NOT "LLM_STATE".
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("my-project");
        let plan_dir = project.join("LLM_STATE").join("plan-x");
        mark_as_git_project(&project);
        write_plan_files(&plan_dir, "work\n", None, None, None);

        let snapshot = load_plan(&plan_dir).unwrap();
        assert_eq!(snapshot.project, "my-project");
    }

    #[test]
    fn load_plan_errors_when_phase_md_absent() {
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path().join("not-a-plan");
        fs::create_dir_all(&plan_dir).unwrap();
        let err = load_plan(&plan_dir).unwrap_err();
        assert!(format!("{err:#}").contains("not a plan directory"));
    }

    #[test]
    fn load_plan_errors_when_no_git_above_plan() {
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path().join("rogue-plan");
        write_plan_files(&plan_dir, "work\n", None, None, None);
        let err = load_plan(&plan_dir).unwrap_err();
        assert!(format!("{err:#}").contains("No .git found"));
    }

    #[test]
    fn load_plan_trims_phase_whitespace() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("plan-a");
        mark_as_git_project(&project);
        write_plan_files(&plan_dir, "  \n work \n\n", None, None, None);
        let snapshot = load_plan(&plan_dir).unwrap();
        assert_eq!(snapshot.phase, "work");
    }

    #[test]
    fn load_plan_records_missing_files_as_none() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("plan-a");
        mark_as_git_project(&project);
        write_plan_files(&plan_dir, "work\n", None, None, None);
        let snapshot = load_plan(&plan_dir).unwrap();
        assert!(snapshot.backlog.is_none());
        assert!(snapshot.memory.is_none());
    }

    #[test]
    fn load_plan_errors_when_plan_dir_missing() {
        let missing = PathBuf::from("/definitely/not/a/path/for/survey/test");
        assert!(load_plan(&missing).is_err());
    }

    #[test]
    fn load_plan_computes_stable_input_hash_for_same_inputs() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_a = project.join("plan-a");
        let plan_b = project.join("plan-b");
        write_plan_files(&plan_a, "work\n", Some("# b\n"), Some("# m\n"), Some("# r\n"));
        write_plan_files(&plan_b, "work\n", Some("# b\n"), Some("# m\n"), Some("# r\n"));

        let hash_a = load_plan(&plan_a).unwrap().input_hash;
        let hash_b = load_plan(&plan_b).unwrap().input_hash;
        assert_eq!(hash_a, hash_b, "identical file contents must hash equally");
        assert_eq!(hash_a.len(), 64, "SHA-256 hex digest is 64 chars: {hash_a}");
    }

    #[test]
    fn load_plan_input_hash_changes_when_any_section_changes() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");

        write_plan_files(&plan_dir, "work\n", Some("# b\n"), Some("# m\n"), Some("# r\n"));
        let hash_initial = load_plan(&plan_dir).unwrap().input_hash;

        // Mutate each file in turn — each mutation must produce a
        // different hash from the initial state.
        fs::write(plan_dir.join("phase.md"), "triage\n").unwrap();
        let hash_phase = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_phase);

        fs::write(plan_dir.join("phase.md"), "work\n").unwrap();
        fs::write(plan_dir.join("backlog.md"), "# b2\n").unwrap();
        let hash_backlog = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_backlog);

        fs::write(plan_dir.join("backlog.md"), "# b\n").unwrap();
        fs::write(plan_dir.join("memory.md"), "# m2\n").unwrap();
        let hash_memory = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_memory);

        fs::write(plan_dir.join("memory.md"), "# m\n").unwrap();
        fs::write(plan_dir.join("related-plans.md"), "# r2\n").unwrap();
        let hash_related = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_related);
    }

    #[test]
    fn load_plan_input_hash_distinguishes_absent_from_empty() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_absent = project.join("absent");
        let plan_empty = project.join("empty");

        // Absent: no backlog.md file.
        write_plan_files(&plan_absent, "work\n", None, None, None);
        // Empty: backlog.md exists but is empty.
        write_plan_files(&plan_empty, "work\n", Some(""), None, None);

        let hash_absent = load_plan(&plan_absent).unwrap().input_hash;
        let hash_empty = load_plan(&plan_empty).unwrap().input_hash;
        assert_ne!(
            hash_absent, hash_empty,
            "absent and empty should hash differently so change detection can tell them apart"
        );
    }

    #[test]
    fn load_plan_input_hash_ignores_session_log() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");
        write_plan_files(&plan_dir, "work\n", Some("# b\n"), None, None);
        let hash_initial = load_plan(&plan_dir).unwrap().input_hash;

        // Writing session-log.md must NOT affect the hash — it's
        // append-only and would otherwise invalidate the hash every cycle.
        fs::write(plan_dir.join("session-log.md"), "many words\n").unwrap();
        let hash_after_log = load_plan(&plan_dir).unwrap().input_hash;
        assert_eq!(hash_initial, hash_after_log);
    }
}
