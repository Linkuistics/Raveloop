//! Detect and repair stale task statuses in a backlog.
//!
//! Two drift modes are repaired automatically — neither needs operator
//! judgement, so every cycle spent re-asking the LLM to perform them is
//! waste:
//!
//! 1. `in_progress` + non-empty `results` → `done`. The work agent
//!    recorded its work but forgot to flip the status; the results
//!    field is authoritative.
//! 2. `blocked` + every structural `dependencies` entry now `done` →
//!    `not_started`. The blocker resolved; the task is unblocked.
//!
//! Other apparent drifts (notably `not_started` with a non-empty
//! `results` field) are intentionally NOT repaired. A result on a task
//! still marked `not_started` signals intent — a revert, an amended
//! result, an operator-staged change — not forgetting. Silent flipping
//! there would lose information.
//!
//! The scan is pure; `run_repair_stale_statuses` reads the backlog,
//! applies the repairs in memory (unless `--dry-run`), writes the file
//! back, and emits the report.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::schema::{BacklogFile, Status, Task};
use super::verbs::OutputFormat;
use super::yaml_io::{read_backlog, write_backlog};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairReason {
    ResultsNonEmpty,
    DependenciesSatisfied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repair {
    pub task_id: String,
    pub old_status: Status,
    pub new_status: Status,
    pub reason: RepairReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairReport {
    pub repairs: Vec<Repair>,
}

/// Pure analysis: returns the list of repairs that *would* apply to
/// `backlog`, without mutating it. Filesystem-independent so unit
/// tests can pin the semantics without tempdir setup.
pub fn analyse_repairs(backlog: &BacklogFile) -> RepairReport {
    let done_ids: HashSet<&str> = backlog
        .tasks
        .iter()
        .filter(|t| t.status == Status::Done)
        .map(|t| t.id.as_str())
        .collect();

    let repairs = backlog
        .tasks
        .iter()
        .filter_map(|task| detect_repair(task, &done_ids))
        .collect();

    RepairReport { repairs }
}

fn detect_repair(task: &Task, done_ids: &HashSet<&str>) -> Option<Repair> {
    match task.status {
        Status::InProgress if results_non_empty(task.results.as_deref()) => Some(Repair {
            task_id: task.id.clone(),
            old_status: Status::InProgress,
            new_status: Status::Done,
            reason: RepairReason::ResultsNonEmpty,
        }),
        Status::Blocked if dependencies_satisfied(&task.dependencies, done_ids) => Some(Repair {
            task_id: task.id.clone(),
            old_status: Status::Blocked,
            new_status: Status::NotStarted,
            reason: RepairReason::DependenciesSatisfied,
        }),
        _ => None,
    }
}

/// Prose convention is that an empty or `_pending_` results body means
/// "not yet complete". The typed YAML surface uses `Option<String>`,
/// so `None` is one empty form; whitespace-only or the literal
/// `_pending_` marker are the others.
fn results_non_empty(results: Option<&str>) -> bool {
    match results {
        Some(body) => {
            let trimmed = body.trim();
            !trimmed.is_empty() && trimmed != "_pending_"
        }
        None => false,
    }
}

/// A `blocked` task auto-unblocks only when it has at least one
/// structural dependency AND every one is `done`. With zero explicit
/// dependencies the blocker is external (operator decision, upstream
/// project, awaiting review) and must not silently resolve.
fn dependencies_satisfied(dependencies: &[String], done_ids: &HashSet<&str>) -> bool {
    if dependencies.is_empty() {
        return false;
    }
    dependencies.iter().all(|dep| done_ids.contains(dep.as_str()))
}

/// CLI entry point: load `<plan_dir>/backlog.yaml`, compute the
/// repairs, optionally apply them, write back, emit the report.
/// Returns the number of repairs applied (or that *would* apply, under
/// `--dry-run`) so the dispatcher can exit non-zero as a scripting
/// signal without the caller having to re-parse the output.
pub fn run_repair_stale_statuses(
    plan_dir: &Path,
    dry_run: bool,
    format: OutputFormat,
) -> Result<usize> {
    let mut backlog = read_backlog(plan_dir)?;
    let report = analyse_repairs(&backlog);

    if !dry_run && !report.repairs.is_empty() {
        apply_repairs(&mut backlog, &report);
        write_backlog(plan_dir, &backlog)?;
    }

    emit(&report, format)?;
    Ok(report.repairs.len())
}

fn apply_repairs(backlog: &mut BacklogFile, report: &RepairReport) {
    for repair in &report.repairs {
        if let Some(task) = backlog.tasks.iter_mut().find(|t| t.id == repair.task_id) {
            task.status = repair.new_status;
            // Unblocking a task must clear `blocked_reason` — leaving
            // the reason behind would fossilise a stale blocker note
            // on a now-actionable task.
            if repair.new_status != Status::Blocked {
                task.blocked_reason = None;
            }
        }
    }
}

fn emit(report: &RepairReport, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(report)?,
        OutputFormat::Json => serde_json::to_string_pretty(report)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn task(id: &str, status: Status, deps: &[&str], results: Option<&str>) -> Task {
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
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            description: "body\n".into(),
            results: results.map(String::from),
            handoff: None,
        }
    }

    fn backlog_with(tasks: Vec<Task>) -> BacklogFile {
        BacklogFile {
            tasks,
            extra: Default::default(),
        }
    }

    #[test]
    fn in_progress_with_non_empty_results_repairs_to_done() {
        let backlog = backlog_with(vec![task(
            "foo",
            Status::InProgress,
            &[],
            Some("did the thing\n"),
        )]);
        let report = analyse_repairs(&backlog);
        assert_eq!(report.repairs.len(), 1);
        let r = &report.repairs[0];
        assert_eq!(r.task_id, "foo");
        assert_eq!(r.old_status, Status::InProgress);
        assert_eq!(r.new_status, Status::Done);
        assert_eq!(r.reason, RepairReason::ResultsNonEmpty);
    }

    #[test]
    fn not_started_with_non_empty_results_is_not_repaired() {
        // A result on a not_started task signals intent (revert, amend,
        // staged change) rather than drift. Silent flipping would lose
        // information.
        let backlog = backlog_with(vec![task(
            "foo",
            Status::NotStarted,
            &[],
            Some("leftover results\n"),
        )]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn in_progress_with_empty_results_is_not_repaired() {
        let backlog = backlog_with(vec![
            task("foo", Status::InProgress, &[], None),
            task("bar", Status::InProgress, &[], Some("  \n")),
            task("baz", Status::InProgress, &[], Some("_pending_")),
        ]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn blocked_with_all_deps_done_repairs_to_not_started() {
        let backlog = backlog_with(vec![
            task("foo", Status::Done, &[], Some("done\n")),
            task("bar", Status::Done, &[], Some("done\n")),
            task("baz", Status::Blocked, &["foo", "bar"], None),
        ]);
        let report = analyse_repairs(&backlog);
        assert_eq!(report.repairs.len(), 1);
        let r = &report.repairs[0];
        assert_eq!(r.task_id, "baz");
        assert_eq!(r.old_status, Status::Blocked);
        assert_eq!(r.new_status, Status::NotStarted);
        assert_eq!(r.reason, RepairReason::DependenciesSatisfied);
    }

    #[test]
    fn blocked_with_some_deps_pending_is_not_repaired() {
        let backlog = backlog_with(vec![
            task("foo", Status::Done, &[], Some("done\n")),
            task("bar", Status::NotStarted, &[], None),
            task("baz", Status::Blocked, &["foo", "bar"], None),
        ]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn blocked_with_no_dependencies_is_not_auto_unblocked() {
        // Zero explicit deps means the blocker is external — an
        // operator decision, an upstream project, awaiting review.
        // Do not silently resolve.
        let backlog = backlog_with(vec![task("foo", Status::Blocked, &[], None)]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn empty_backlog_yields_empty_report() {
        let backlog = backlog_with(vec![]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn run_applies_repairs_and_writes_back_when_not_dry_run() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![
            task("foo", Status::Done, &[], Some("done\n")),
            task("bar", Status::InProgress, &[], Some("completed\n")),
            task("baz", Status::Blocked, &["foo"], None),
        ]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), false, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 2, "two repairs expected (bar → done, baz → not_started)");

        let reloaded = read_backlog(tmp.path()).unwrap();
        let bar = reloaded.tasks.iter().find(|t| t.id == "bar").unwrap();
        let baz = reloaded.tasks.iter().find(|t| t.id == "baz").unwrap();
        assert_eq!(bar.status, Status::Done);
        assert_eq!(baz.status, Status::NotStarted);
        assert_eq!(
            baz.blocked_reason, None,
            "blocked_reason must clear when a task is unblocked"
        );
    }

    #[test]
    fn run_dry_run_reports_without_writing() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![task(
            "foo",
            Status::InProgress,
            &[],
            Some("done\n"),
        )]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), true, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 1);

        // Disk is untouched — status still in_progress.
        let reloaded = read_backlog(tmp.path()).unwrap();
        let foo = reloaded.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.status, Status::InProgress);
    }

    #[test]
    fn run_on_empty_backlog_returns_zero_and_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), false, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn report_serialises_with_snake_case_reason_tags() {
        let report = RepairReport {
            repairs: vec![
                Repair {
                    task_id: "foo".into(),
                    old_status: Status::InProgress,
                    new_status: Status::Done,
                    reason: RepairReason::ResultsNonEmpty,
                },
                Repair {
                    task_id: "bar".into(),
                    old_status: Status::Blocked,
                    new_status: Status::NotStarted,
                    reason: RepairReason::DependenciesSatisfied,
                },
            ],
        };
        let yaml = serde_yaml::to_string(&report).unwrap();
        // Reason tags must match the YAML shape advertised in the
        // CLI spec so downstream scripts can grep on known strings.
        assert!(yaml.contains("results_non_empty"), "yaml must use snake_case reason: {yaml}");
        assert!(yaml.contains("dependencies_satisfied"), "yaml must use snake_case reason: {yaml}");
    }
}
