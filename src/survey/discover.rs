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

use crate::git::project_root_for_plan;
use crate::state::backlog::{read_backlog, PlanRowCounts, TaskCounts};
use crate::state::filenames::{BACKLOG_FILENAME, MEMORY_FILENAME};
#[cfg(test)]
use crate::state::filenames::SESSION_LOG_FILENAME;
use crate::state::memory::read_memory;

/// A single plan's state, bundled for inclusion in the survey prompt.
/// `input_hash` is a SHA-256 over the four state files
/// (`phase.md` + `backlog.yaml` + `memory.yaml` + `related-plans.md`),
/// computed at load time and injected into the survey response's
/// `PlanRow` post-parse. `session-log.yaml` is deliberately excluded:
/// it's append-only and would defeat change detection.
#[derive(Debug)]
pub struct PlanSnapshot {
    pub project: String,
    pub plan: String,
    pub phase: String,
    /// Serialised `backlog.yaml` content, inlined verbatim into the
    /// survey prompt. `None` when `backlog.yaml` is missing or fails
    /// to parse — surfaces to the LLM as `(missing)` rather than
    /// taking the whole survey down.
    pub backlog: Option<String>,
    /// Serialised `memory.yaml` content, same semantics as `backlog`.
    pub memory: Option<String>,
    pub input_hash: String,
    /// Per-status task tally computed from the plan's parsed
    /// `backlog.yaml` via `BacklogFile::task_counts`. `None` when the
    /// file is absent or the YAML parser rejects it; callers inject
    /// this into the survey's `PlanRow` so the LLM never has to count
    /// tasks itself.
    pub task_counts: Option<TaskCounts>,
    /// Readiness + received-handoff counts computed from the plan's
    /// parsed `backlog.yaml` via `BacklogFile::plan_row_counts`. `None`
    /// with the same "absent or unparseable" semantics as `task_counts`.
    /// Callers inject the three fields into the survey's `PlanRow` so
    /// the LLM no longer derives them from the prompt.
    pub plan_row_counts: Option<PlanRowCounts>,
}

/// Derive the project name for a plan by deriving the subtree root
/// (`<plan>/../..`) and taking its basename. In a single-repo layout
/// this is the repo name; in a monorepo it's the subtree name — which
/// is what should label the plan, since plans are per-subtree.
fn project_name_for_plan(plan_path: &Path) -> Result<String> {
    let root = project_root_for_plan(plan_path)?;
    Path::new(&root)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .with_context(|| format!("Could not derive project name from subtree root {root}"))
}

/// Load a single plan directory's state into a `PlanSnapshot`. A
/// directory is a plan iff it contains a `phase.md` file; this matches
/// the convention used everywhere else in Ravel-Lite. The project
/// name is the basename of the nearest ancestor containing `.git`, so
/// plans from different repos are labelled correctly even when
/// co-located on the CLI.
///
/// Structured plan-state (backlog, memory) is routed through the typed
/// YAML API — `read_backlog` / `read_memory` — so `load_plan` cannot
/// be silently bypassed by deleting the legacy `.md` originals via
/// `state migrate --delete-originals`.
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

    let backlog_file = if plan_dir.join(BACKLOG_FILENAME).exists() {
        read_backlog(plan_dir).ok()
    } else {
        None
    };
    let memory_file = if plan_dir.join(MEMORY_FILENAME).exists() {
        read_memory(plan_dir).ok()
    } else {
        None
    };

    let backlog = backlog_file
        .as_ref()
        .map(serde_yaml::to_string)
        .transpose()
        .with_context(|| format!("failed to re-serialise {BACKLOG_FILENAME} for survey input"))?;
    let memory = memory_file
        .as_ref()
        .map(serde_yaml::to_string)
        .transpose()
        .with_context(|| format!("failed to re-serialise {MEMORY_FILENAME} for survey input"))?;

    let related_plans = fs::read_to_string(plan_dir.join("related-plans.md")).ok();

    let input_hash = compute_input_hash(
        &phase_raw,
        backlog.as_deref(),
        memory.as_deref(),
        related_plans.as_deref(),
    );

    let task_counts = backlog_file.as_ref().map(|bf| bf.task_counts());
    let plan_row_counts = backlog_file.as_ref().map(|bf| bf.plan_row_counts());

    Ok(PlanSnapshot {
        project,
        plan,
        phase,
        backlog,
        memory,
        input_hash,
        task_counts,
        plan_row_counts,
    })
}

/// SHA-256 hex digest over the four plan-state sections whose contents
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
    use crate::state::backlog::schema::{BacklogFile, Status, Task};
    use crate::state::backlog::write_backlog;
    use crate::state::memory::schema::{MemoryEntry, MemoryFile};
    use crate::state::memory::write_memory;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Ensure `project_dir` exists on disk. Under the old git-walkup
    /// scheme this also seeded `.git/`; under the current path-math
    /// scheme the `.git` seed is irrelevant but harmless. Kept to
    /// simulate a realistic project layout.
    fn mark_as_git_project(project_dir: &Path) {
        fs::create_dir_all(project_dir.join(".git")).unwrap();
    }

    /// Test fixture: a one-task backlog whose identity varies with the
    /// provided title. Keeps call sites terse when the actual field
    /// values aren't what the test is asserting on.
    fn one_task_backlog(title: &str) -> BacklogFile {
        BacklogFile {
            tasks: vec![Task {
                id: "fixture-task".into(),
                title: title.into(),
                category: "maintenance".into(),
                status: Status::NotStarted,
                blocked_reason: None,
                dependencies: vec![],
                description: "Fixture body.\n".into(),
                results: None,
                handoff: None,
            }],
            extra: Default::default(),
        }
    }

    /// Test fixture: one memory entry with the given title. Body is a
    /// constant so identity still tracks title changes.
    fn one_entry_memory(title: &str) -> MemoryFile {
        MemoryFile {
            entries: vec![MemoryEntry {
                id: "fixture-entry".into(),
                title: title.into(),
                body: "Fixture body.\n".into(),
            }],
            extra: Default::default(),
        }
    }

    struct PlanOptions<'a> {
        phase: &'a str,
        backlog: Option<&'a BacklogFile>,
        memory: Option<&'a MemoryFile>,
        related_plans: Option<&'a str>,
    }

    fn write_plan(plan_dir: &Path, options: &PlanOptions) {
        fs::create_dir_all(plan_dir).unwrap();
        fs::write(plan_dir.join("phase.md"), options.phase).unwrap();
        if let Some(backlog) = options.backlog {
            write_backlog(plan_dir, backlog).unwrap();
        }
        if let Some(memory) = options.memory {
            write_memory(plan_dir, memory).unwrap();
        }
        if let Some(related_plans) = options.related_plans {
            fs::write(plan_dir.join("related-plans.md"), related_plans).unwrap();
        }
    }

    #[test]
    fn load_plan_reads_phase_backlog_and_memory() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("LLM_STATE").join("plan-a");
        mark_as_git_project(&project);
        let backlog = one_task_backlog("Plan-a task");
        let memory = one_entry_memory("Plan-a entry");
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: Some(&backlog),
                memory: Some(&memory),
                related_plans: None,
            },
        );

        let snapshot = load_plan(&plan_dir).unwrap();
        assert_eq!(snapshot.project, "p");
        assert_eq!(snapshot.plan, "plan-a");
        assert_eq!(snapshot.phase, "work");
        let backlog_yaml = snapshot.backlog.as_deref().expect("backlog surfaced");
        assert!(
            backlog_yaml.contains("Plan-a task"),
            "serialised backlog must contain the task title: {backlog_yaml}"
        );
        let memory_yaml = snapshot.memory.as_deref().expect("memory surfaced");
        assert!(
            memory_yaml.contains("Plan-a entry"),
            "serialised memory must contain the entry title: {memory_yaml}"
        );
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
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: None,
                memory: None,
                related_plans: None,
            },
        );

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
    fn load_plan_trims_phase_whitespace() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("plan-a");
        mark_as_git_project(&project);
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "  \n work \n\n",
                backlog: None,
                memory: None,
                related_plans: None,
            },
        );
        let snapshot = load_plan(&plan_dir).unwrap();
        assert_eq!(snapshot.phase, "work");
    }

    #[test]
    fn load_plan_records_missing_files_as_none() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let plan_dir = project.join("plan-a");
        mark_as_git_project(&project);
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: None,
                memory: None,
                related_plans: None,
            },
        );
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
        let backlog = one_task_backlog("Shared task");
        let memory = one_entry_memory("Shared entry");
        for plan_dir in [&plan_a, &plan_b] {
            write_plan(
                plan_dir,
                &PlanOptions {
                    phase: "work\n",
                    backlog: Some(&backlog),
                    memory: Some(&memory),
                    related_plans: Some("# r\n"),
                },
            );
        }

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

        let backlog_initial = one_task_backlog("Initial task");
        let memory_initial = one_entry_memory("Initial entry");
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: Some(&backlog_initial),
                memory: Some(&memory_initial),
                related_plans: Some("# r\n"),
            },
        );
        let hash_initial = load_plan(&plan_dir).unwrap().input_hash;

        // Mutate each section in turn — each mutation must produce a
        // different hash from the initial state.
        fs::write(plan_dir.join("phase.md"), "triage\n").unwrap();
        let hash_phase = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_phase);

        fs::write(plan_dir.join("phase.md"), "work\n").unwrap();
        let backlog_mutated = one_task_backlog("Mutated task");
        write_backlog(&plan_dir, &backlog_mutated).unwrap();
        let hash_backlog = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_backlog);

        write_backlog(&plan_dir, &backlog_initial).unwrap();
        let memory_mutated = one_entry_memory("Mutated entry");
        write_memory(&plan_dir, &memory_mutated).unwrap();
        let hash_memory = load_plan(&plan_dir).unwrap().input_hash;
        assert_ne!(hash_initial, hash_memory);

        write_memory(&plan_dir, &memory_initial).unwrap();
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

        // Absent: no backlog.yaml file.
        write_plan(
            &plan_absent,
            &PlanOptions {
                phase: "work\n",
                backlog: None,
                memory: None,
                related_plans: None,
            },
        );
        // Empty: backlog.yaml exists but has an empty tasks list.
        let empty_backlog = BacklogFile::default();
        write_plan(
            &plan_empty,
            &PlanOptions {
                phase: "work\n",
                backlog: Some(&empty_backlog),
                memory: None,
                related_plans: None,
            },
        );

        let hash_absent = load_plan(&plan_absent).unwrap().input_hash;
        let hash_empty = load_plan(&plan_empty).unwrap().input_hash;
        assert_ne!(
            hash_absent, hash_empty,
            "absent and empty should hash differently so change detection can tell them apart"
        );
    }

    #[test]
    fn load_plan_populates_task_counts_from_parseable_backlog_yaml() {
        // One task in each status. `load_plan`'s Rust-side parse via
        // `read_backlog` must populate `task_counts` so the survey
        // prompt no longer has to tally them.
        fn task(id: &str, status: Status) -> Task {
            Task {
                id: id.into(),
                title: id.into(),
                category: "maintenance".into(),
                status,
                blocked_reason: if status == Status::Blocked {
                    Some("upstream".into())
                } else {
                    None
                },
                dependencies: vec![],
                description: "Body.\n".into(),
                results: if status == Status::Done {
                    Some("Done successfully.\n".into())
                } else {
                    None
                },
                handoff: None,
            }
        }
        let backlog = BacklogFile {
            tasks: vec![
                task("not-started-task", Status::NotStarted),
                task("in-progress-task", Status::InProgress),
                task("done-task", Status::Done),
                task("blocked-task", Status::Blocked),
            ],
            extra: Default::default(),
        };

        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: Some(&backlog),
                memory: None,
                related_plans: None,
            },
        );

        let snapshot = load_plan(&plan_dir).unwrap();
        let counts = snapshot.task_counts.expect("backlog parsed; counts populated");
        assert_eq!(counts.total, 4);
        assert_eq!(counts.not_started, 1);
        assert_eq!(counts.in_progress, 1);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.blocked, 1);
    }

    #[test]
    fn load_plan_leaves_task_counts_none_when_backlog_yaml_absent() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: None,
                memory: None,
                related_plans: None,
            },
        );
        let snapshot = load_plan(&plan_dir).unwrap();
        assert!(snapshot.task_counts.is_none());
    }

    #[test]
    fn load_plan_leaves_task_counts_none_when_backlog_yaml_is_unparseable() {
        // Not a hard error — survey carries on with `task_counts: None`
        // and `backlog: None` so a malformed backlog.yaml doesn't take
        // the whole survey down. The content field collapses to None
        // alongside the counts, which shows the user `(missing)` in
        // the prompt rather than content that failed validation.
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");
        fs::create_dir_all(&plan_dir).unwrap();
        fs::write(plan_dir.join("phase.md"), "work\n").unwrap();
        fs::write(
            plan_dir.join(BACKLOG_FILENAME),
            "tasks:\n  - id: bad\n    status: not_a_real_status\n",
        )
        .unwrap();

        let snapshot = load_plan(&plan_dir).unwrap();
        assert!(snapshot.task_counts.is_none());
        assert!(snapshot.backlog.is_none());
    }

    #[test]
    fn load_plan_input_hash_ignores_session_log() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        mark_as_git_project(&project);
        let plan_dir = project.join("plan-a");
        let backlog = one_task_backlog("Base task");
        write_plan(
            &plan_dir,
            &PlanOptions {
                phase: "work\n",
                backlog: Some(&backlog),
                memory: None,
                related_plans: None,
            },
        );
        let hash_initial = load_plan(&plan_dir).unwrap().input_hash;

        // Writing a session log (either .md legacy or .yaml structured)
        // must NOT affect the hash — session logs are append-only and
        // would otherwise invalidate the hash every cycle.
        fs::write(plan_dir.join("session-log.md"), "many words\n").unwrap();
        let hash_after_md_log = load_plan(&plan_dir).unwrap().input_hash;
        assert_eq!(hash_initial, hash_after_md_log);

        fs::write(plan_dir.join(SESSION_LOG_FILENAME), "sessions: []\n").unwrap();
        let hash_after_yaml_log = load_plan(&plan_dir).unwrap().input_hash;
        assert_eq!(hash_initial, hash_after_yaml_log);
    }
}
