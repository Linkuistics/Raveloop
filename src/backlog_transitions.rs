//! Computes a human-readable summary of what changed in a plan's
//! `backlog.yaml` between a baseline commit and the current working-tree
//! state. Injected into the analyse-work prompt as `{{BACKLOG_TRANSITIONS}}`
//! so the LLM can author a specific commit title ("Mark <task-id> done,
//! record results") rather than falling back to the phase name.
//!
//! Runner-side on purpose: scanning a baseline YAML and a current YAML to
//! compute a delta is pure mechanical work — exactly the kind of thing the
//! "Never do in an LLM what you can do in code" rule says should live here.
//!
//! Soft-fails on every failure mode (missing baseline, git error, parse
//! error) because the prompt's `{{BACKLOG_TRANSITIONS}}` slot always needs
//! *something*; an `Err` would bubble up into `compose_prompt` and wedge
//! the phase loop — exactly the same rationale as `work_tree_snapshot`.

use std::path::Path;
use std::process::Command;

use crate::state::backlog::schema::{BacklogFile, Status, Task};
use crate::state::filenames::BACKLOG_FILENAME;

/// Top-level entry point used by `phase_loop`. Always returns a printable
/// string; never propagates an error.
pub fn backlog_transitions(plan_dir: &Path, baseline_sha: &str) -> String {
    if baseline_sha.is_empty() {
        return "(no baseline SHA available; first cycle has no prior state to diff)".to_string();
    }

    let current = match crate::state::backlog::read_backlog(plan_dir) {
        Ok(b) => b,
        Err(e) => return format!("(failed to read current {BACKLOG_FILENAME}: {e})"),
    };

    let baseline = match read_baseline_backlog(plan_dir, baseline_sha) {
        BaselineResult::Ok(b) => b,
        BaselineResult::Missing => {
            return render_additions_only(&current);
        }
        BaselineResult::Error(msg) => return format!("(baseline lookup failed: {msg})"),
    };

    let transitions = compute_transitions(&baseline, &current);
    render_transitions(&transitions)
}

enum BaselineResult {
    Ok(BacklogFile),
    Missing,
    Error(String),
}

/// Retrieve `backlog.yaml` content at `baseline_sha`. Uses
/// `git ls-files --full-name <backlog>` to resolve the path
/// relative to the git repo root, so this works identically in a
/// single-repo layout and in a monorepo subtree.
fn read_baseline_backlog(plan_dir: &Path, baseline_sha: &str) -> BaselineResult {
    let full_name_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["ls-files", "--full-name", BACKLOG_FILENAME])
        .output();

    let full_name = match full_name_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(o) => {
            return BaselineResult::Error(format!(
                "git ls-files exited {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Err(e) => return BaselineResult::Error(format!("git ls-files failed: {e}")),
    };

    if full_name.is_empty() {
        return BaselineResult::Missing;
    }

    let show_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["show", &format!("{baseline_sha}:{full_name}")])
        .output();

    match show_out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            match serde_yaml::from_str::<BacklogFile>(&text) {
                Ok(b) => BaselineResult::Ok(b),
                Err(e) => BaselineResult::Error(format!("baseline YAML parse: {e}")),
            }
        }
        Ok(_) => BaselineResult::Missing,
        Err(e) => BaselineResult::Error(format!("git show failed: {e}")),
    }
}

/// Transition record for one task id. Absent-in-baseline and
/// absent-in-current are the two "pure" cases; everything else is a
/// field-level diff where at least one of status/results/title/deps
/// differs.
#[derive(Debug, Default, PartialEq, Eq)]
struct Transitions {
    status_flips: Vec<(String, Status, Status)>,
    results_added: Vec<(String, usize)>,
    results_modified: Vec<(String, usize, usize)>,
    tasks_added: Vec<(String, String)>,
    tasks_deleted: Vec<(String, String)>,
    title_changes: Vec<(String, String, String)>,
    dependency_changes: Vec<(String, Vec<String>, Vec<String>)>,
    handoff_changes: Vec<(String, HandoffChange)>,
}

#[derive(Debug, PartialEq, Eq)]
enum HandoffChange {
    Added,
    Modified,
    Cleared,
}

fn compute_transitions(baseline: &BacklogFile, current: &BacklogFile) -> Transitions {
    use std::collections::HashMap;

    let base_by_id: HashMap<&str, &Task> = baseline
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    let curr_by_id: HashMap<&str, &Task> = current
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();

    let mut out = Transitions::default();

    for curr in &current.tasks {
        match base_by_id.get(curr.id.as_str()) {
            None => {
                out.tasks_added.push((curr.id.clone(), curr.title.clone()));
            }
            Some(base) => {
                diff_task_fields(base, curr, &mut out);
            }
        }
    }

    for base in &baseline.tasks {
        if !curr_by_id.contains_key(base.id.as_str()) {
            out.tasks_deleted.push((base.id.clone(), base.title.clone()));
        }
    }

    out
}

fn diff_task_fields(base: &Task, curr: &Task, out: &mut Transitions) {
    if base.status != curr.status {
        out.status_flips.push((curr.id.clone(), base.status, curr.status));
    }

    let base_results_len = base.results.as_deref().map(line_count).unwrap_or(0);
    let curr_results_len = curr.results.as_deref().map(line_count).unwrap_or(0);
    match (base_results_len, curr_results_len) {
        (0, n) if n > 0 => out.results_added.push((curr.id.clone(), n)),
        (b, c) if b > 0 && c > 0 && base.results != curr.results => {
            out.results_modified.push((curr.id.clone(), b, c));
        }
        _ => {}
    }

    if base.title != curr.title {
        out.title_changes
            .push((curr.id.clone(), base.title.clone(), curr.title.clone()));
    }

    if base.dependencies != curr.dependencies {
        out.dependency_changes.push((
            curr.id.clone(),
            base.dependencies.clone(),
            curr.dependencies.clone(),
        ));
    }

    let change = match (base.handoff.as_deref(), curr.handoff.as_deref()) {
        (None, Some(s)) if !s.is_empty() => Some(HandoffChange::Added),
        (Some(b), None) if !b.is_empty() => Some(HandoffChange::Cleared),
        (Some(b), Some(c)) if b != c => Some(HandoffChange::Modified),
        _ => None,
    };
    if let Some(hc) = change {
        out.handoff_changes.push((curr.id.clone(), hc));
    }
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

fn render_transitions(t: &Transitions) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !t.status_flips.is_empty() {
        let mut lines = vec!["status flips:".to_string()];
        for (id, from, to) in &t.status_flips {
            lines.push(format!("  - {id}: {} → {}", status_label(*from), status_label(*to)));
        }
        sections.push(lines.join("\n"));
    }

    if !t.results_added.is_empty() {
        let mut lines = vec!["results added:".to_string()];
        for (id, n) in &t.results_added {
            lines.push(format!("  - {id}: {n} line(s)"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.results_modified.is_empty() {
        let mut lines = vec!["results modified:".to_string()];
        for (id, from, to) in &t.results_modified {
            lines.push(format!("  - {id}: {from} → {to} line(s)"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.tasks_added.is_empty() {
        let mut lines = vec!["tasks added:".to_string()];
        for (id, title) in &t.tasks_added {
            lines.push(format!("  + {id}: {title}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.tasks_deleted.is_empty() {
        let mut lines = vec!["tasks deleted:".to_string()];
        for (id, title) in &t.tasks_deleted {
            lines.push(format!("  - {id}: {title}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.title_changes.is_empty() {
        let mut lines = vec!["title changes:".to_string()];
        for (id, from, to) in &t.title_changes {
            lines.push(format!("  - {id}: {from:?} → {to:?}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.dependency_changes.is_empty() {
        let mut lines = vec!["dependency changes:".to_string()];
        for (id, from, to) in &t.dependency_changes {
            lines.push(format!("  - {id}: {from:?} → {to:?}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.handoff_changes.is_empty() {
        let mut lines = vec!["handoff changes:".to_string()];
        for (id, change) in &t.handoff_changes {
            let label = match change {
                HandoffChange::Added => "added",
                HandoffChange::Modified => "modified",
                HandoffChange::Cleared => "cleared",
            };
            lines.push(format!("  - {id}: {label}"));
        }
        sections.push(lines.join("\n"));
    }

    if sections.is_empty() {
        "(no backlog changes since baseline)".to_string()
    } else {
        sections.join("\n\n")
    }
}

/// Fallback renderer for the "baseline-backlog-missing" case, which
/// happens on a plan's very first cycle: every task in the current
/// backlog is "new" by definition, so we render additions only.
fn render_additions_only(current: &BacklogFile) -> String {
    if current.tasks.is_empty() {
        return "(no baseline; current backlog is empty)".to_string();
    }
    let mut lines = vec!["(no baseline found at this SHA — rendering current backlog as additions)".to_string(), String::new(), "tasks added:".to_string()];
    for task in &current.tasks {
        lines.push(format!("  + {}: {}", task.id, task.title));
    }
    lines.join("\n")
}

fn status_label(s: Status) -> &'static str {
    match s {
        Status::NotStarted => "not_started",
        Status::InProgress => "in_progress",
        Status::Done => "done",
        Status::Blocked => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::schema::Task;

    fn task(id: &str, status: Status) -> Task {
        Task {
            id: id.into(),
            title: format!("Task {id}"),
            category: "core".into(),
            status,
            blocked_reason: None,
            dependencies: vec![],
            description: "desc".into(),
            results: None,
            handoff: None,
        }
    }

    #[test]
    fn empty_baseline_and_current_renders_no_change_marker() {
        let base = BacklogFile::default();
        let curr = BacklogFile::default();
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert_eq!(rendered, "(no backlog changes since baseline)");
    }

    #[test]
    fn status_flip_is_rendered_with_arrow() {
        let base = BacklogFile { tasks: vec![task("foo", Status::NotStarted)], ..Default::default() };
        let curr = BacklogFile { tasks: vec![task("foo", Status::Done)], ..Default::default() };
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("status flips:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: not_started → done"), "missing line: {rendered}");
    }

    #[test]
    fn results_added_counts_lines() {
        let base = BacklogFile { tasks: vec![task("foo", Status::Done)], ..Default::default() };
        let mut with_results = task("foo", Status::Done);
        with_results.results = Some("line one\nline two\nline three".into());
        let curr = BacklogFile { tasks: vec![with_results], ..Default::default() };
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("results added:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: 3 line(s)"), "missing line: {rendered}");
    }

    #[test]
    fn results_modified_shows_old_and_new_line_count() {
        let mut base_task = task("foo", Status::Done);
        base_task.results = Some("old\ntext".into());
        let mut curr_task = task("foo", Status::Done);
        curr_task.results = Some("new\ntext\nmore\nlines".into());

        let t = compute_transitions(
            &BacklogFile { tasks: vec![base_task], ..Default::default() },
            &BacklogFile { tasks: vec![curr_task], ..Default::default() },
        );
        let rendered = render_transitions(&t);
        assert!(rendered.contains("results modified:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: 2 → 4 line(s)"), "missing line: {rendered}");
    }

    #[test]
    fn added_task_renders_with_plus_marker() {
        let base = BacklogFile::default();
        let curr = BacklogFile { tasks: vec![task("new-id", Status::NotStarted)], ..Default::default() };
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("tasks added:"), "missing header: {rendered}");
        assert!(rendered.contains("+ new-id: Task new-id"), "missing line: {rendered}");
    }

    #[test]
    fn deleted_task_renders_with_minus_marker() {
        let base = BacklogFile { tasks: vec![task("old-id", Status::Done)], ..Default::default() };
        let curr = BacklogFile::default();
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("tasks deleted:"), "missing header: {rendered}");
        assert!(rendered.contains("- old-id: Task old-id"), "missing line: {rendered}");
    }

    #[test]
    fn dependency_changes_are_reported() {
        let mut base_task = task("foo", Status::NotStarted);
        base_task.dependencies = vec!["dep-a".into()];
        let mut curr_task = task("foo", Status::NotStarted);
        curr_task.dependencies = vec!["dep-a".into(), "dep-b".into()];
        let t = compute_transitions(
            &BacklogFile { tasks: vec![base_task], ..Default::default() },
            &BacklogFile { tasks: vec![curr_task], ..Default::default() },
        );
        let rendered = render_transitions(&t);
        assert!(rendered.contains("dependency changes:"), "missing header: {rendered}");
        assert!(rendered.contains("foo:"), "missing id line: {rendered}");
    }

    #[test]
    fn handoff_added_is_classified_correctly() {
        let base_task = task("foo", Status::Done);
        let mut curr_task = task("foo", Status::Done);
        curr_task.handoff = Some("design decision".into());
        let t = compute_transitions(
            &BacklogFile { tasks: vec![base_task], ..Default::default() },
            &BacklogFile { tasks: vec![curr_task], ..Default::default() },
        );
        assert_eq!(t.handoff_changes, vec![("foo".into(), HandoffChange::Added)]);
    }

    #[test]
    fn empty_baseline_sha_yields_explanatory_placeholder() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = backlog_transitions(tmp.path(), "");
        assert!(out.contains("no baseline SHA"), "expected placeholder, got: {out}");
    }

    /// End-to-end: baseline backlog committed in git, current backlog on
    /// disk differs, `backlog_transitions` reports the diff. Covers the
    /// `git show <sha>:<full-name>` path resolution.
    #[test]
    fn backlog_transitions_reads_baseline_from_git_show() {
        use std::process::Command;

        let tmp = tempfile::TempDir::new().unwrap();
        let plan = tmp.path();
        Command::new("git").current_dir(plan).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.name", "t"]).output().unwrap();

        let base = BacklogFile { tasks: vec![task("foo", Status::NotStarted)], ..Default::default() };
        crate::state::backlog::write_backlog(plan, &base).unwrap();
        Command::new("git").current_dir(plan).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        let sha = String::from_utf8(
            Command::new("git").current_dir(plan).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
        ).unwrap().trim().to_string();

        let curr = BacklogFile { tasks: vec![task("foo", Status::Done)], ..Default::default() };
        crate::state::backlog::write_backlog(plan, &curr).unwrap();

        let rendered = backlog_transitions(plan, &sha);
        assert!(rendered.contains("foo: not_started → done"), "expected status flip, got: {rendered}");
    }

    /// First-cycle case: baseline commit predates backlog.yaml. The
    /// helper falls back to "additions only" rendering rather than
    /// errorring out.
    #[test]
    fn backlog_transitions_handles_missing_baseline_file() {
        use std::process::Command;

        let tmp = tempfile::TempDir::new().unwrap();
        let plan = tmp.path();
        Command::new("git").current_dir(plan).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.name", "t"]).output().unwrap();

        std::fs::write(plan.join("README"), "seed\n").unwrap();
        Command::new("git").current_dir(plan).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["commit", "-q", "-m", "seed"]).output().unwrap();
        let sha = String::from_utf8(
            Command::new("git").current_dir(plan).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
        ).unwrap().trim().to_string();

        let curr = BacklogFile { tasks: vec![task("foo", Status::NotStarted)], ..Default::default() };
        crate::state::backlog::write_backlog(plan, &curr).unwrap();

        let rendered = backlog_transitions(plan, &sha);
        assert!(rendered.contains("no baseline found"), "expected first-cycle marker: {rendered}");
        assert!(rendered.contains("+ foo: Task foo"), "expected additions-only block: {rendered}");
    }
}
