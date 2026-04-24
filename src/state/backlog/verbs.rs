//! Handlers for every `state backlog <verb>` CLI verb.
//!
//! Each handler is a thin wrapper around the schema + yaml_io: pull the
//! file, mutate or project it, write it back (for mutations) or emit to
//! stdout (for reads). Filter predicates for `list` live here too; they
//! are pure functions against the BacklogFile so unit tests can pin
//! semantics without touching the filesystem.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Result};

use super::schema::{allocate_id, BacklogFile, Status, Task};
use super::yaml_io::{read_backlog, write_backlog};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Yaml,
    Json,
}

impl OutputFormat {
    pub fn parse(input: &str) -> Option<OutputFormat> {
        match input {
            "yaml" => Some(OutputFormat::Yaml),
            "json" => Some(OutputFormat::Json),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ListFilter {
    pub status: Option<Status>,
    pub category: Option<String>,
    pub ready: bool,
    pub has_handoff: bool,
    pub missing_results: bool,
}

/// True iff `task` passes every filter constraint set in `filter`.
pub fn task_matches(task: &Task, filter: &ListFilter, done_ids: &HashSet<&str>) -> bool {
    if let Some(status) = filter.status {
        if task.status != status {
            return false;
        }
    }
    if let Some(category) = &filter.category {
        if &task.category != category {
            return false;
        }
    }
    if filter.ready {
        // `--ready` = `status == not_started AND every dep is done`.
        if task.status != Status::NotStarted {
            return false;
        }
        if task
            .dependencies
            .iter()
            .any(|dep| !done_ids.contains(dep.as_str()))
        {
            return false;
        }
    }
    if filter.has_handoff && task.handoff.is_none() {
        return false;
    }
    if filter.missing_results && !(task.status == Status::Done && task.results.is_none()) {
        return false;
    }
    true
}

pub fn run_list(plan_dir: &Path, filter: &ListFilter, format: OutputFormat) -> Result<()> {
    let backlog = read_backlog(plan_dir)?;
    let done_ids: HashSet<&str> = backlog
        .tasks
        .iter()
        .filter(|t| t.status == Status::Done)
        .map(|t| t.id.as_str())
        .collect();

    let filtered: Vec<&Task> = backlog
        .tasks
        .iter()
        .filter(|t| task_matches(t, filter, &done_ids))
        .collect();

    let projection = BacklogFile {
        tasks: filtered.into_iter().cloned().collect(),
        extra: Default::default(),
    };

    emit(&projection, format)
}

fn emit(backlog: &BacklogFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(backlog)?,
        OutputFormat::Json => serde_json::to_string_pretty(backlog)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let backlog = read_backlog(plan_dir)?;
    let task = find_task(&backlog, id)?;
    let wrapper = BacklogFile {
        tasks: vec![task.clone()],
        extra: Default::default(),
    };
    emit(&wrapper, format)
}

pub(crate) fn find_task<'a>(backlog: &'a BacklogFile, id: &str) -> Result<&'a Task> {
    backlog
        .tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub title: String,
    pub category: String,
    pub dependencies: Vec<String>,
    pub description: String,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    // Validate deps up-front so a typo surfaces as an error rather than a
    // dangling reference in the stored file.
    let existing_ids: HashSet<&str> = backlog.tasks.iter().map(|t| t.id.as_str()).collect();
    for dep in &req.dependencies {
        if !existing_ids.contains(dep.as_str()) {
            bail!(
                "dependency id {dep:?} does not exist in backlog; known ids: {:?}",
                existing_ids
            );
        }
    }

    let id = allocate_id(&req.title, backlog.tasks.iter().map(|t| t.id.as_str()));
    backlog.tasks.push(Task {
        id,
        title: req.title.clone(),
        category: req.category.clone(),
        status: Status::NotStarted,
        blocked_reason: None,
        dependencies: req.dependencies.clone(),
        description: ensure_trailing_newline(&req.description),
        results: None,
        handoff: None,
    });
    write_backlog(plan_dir, &backlog)
}

pub fn run_init(plan_dir: &Path, seed: &BacklogFile) -> Result<()> {
    let existing = read_backlog(plan_dir)?;
    if !existing.tasks.is_empty() {
        bail!(
            "refusing to init: backlog.yaml at {} is non-empty ({} tasks). Use `add` for incremental inserts.",
            plan_dir.display(),
            existing.tasks.len()
        );
    }
    write_backlog(plan_dir, seed)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

pub fn run_set_status(
    plan_dir: &Path,
    id: &str,
    status: Status,
    reason: Option<&str>,
) -> Result<()> {
    if status == Status::Blocked && reason.is_none() {
        bail!("--reason <text> is required when setting status to `blocked`");
    }
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.status = status;
    task.blocked_reason = if status == Status::Blocked {
        reason.map(str::to_string)
    } else {
        None
    };
    write_backlog(plan_dir, &backlog)
}

pub fn run_set_results(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.results = Some(ensure_trailing_newline(body));
    write_backlog(plan_dir, &backlog)
}

/// Rewrite a task's `description` (the markdown body authored at
/// creation time). Unlike `run_set_results`, the body is required to be
/// non-empty — the field has a non-empty invariant enforced at `add`
/// time and a blind-replace that violated it would poison the task
/// brief. Whitespace-only bodies are rejected for the same reason.
pub fn run_set_description(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    if body.trim().is_empty() {
        bail!("description body must not be empty or whitespace-only");
    }
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.description = ensure_trailing_newline(body);
    write_backlog(plan_dir, &backlog)
}

pub fn run_set_handoff(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.handoff = Some(ensure_trailing_newline(body));
    write_backlog(plan_dir, &backlog)
}

pub fn run_clear_handoff(plan_dir: &Path, id: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.handoff = None;
    write_backlog(plan_dir, &backlog)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReorderPosition {
    Before,
    After,
}

impl ReorderPosition {
    pub fn parse(input: &str) -> Option<ReorderPosition> {
        match input {
            "before" => Some(ReorderPosition::Before),
            "after" => Some(ReorderPosition::After),
            _ => None,
        }
    }
}

pub fn run_set_title(plan_dir: &Path, id: &str, new_title: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let task = backlog
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    task.title = new_title.to_string();
    // id is intentionally not recomputed — its stability is the whole
    // point of persisting the slug at creation.
    write_backlog(plan_dir, &backlog)
}

/// Replace the `dependencies` field on a task post-hoc. `run_add`
/// validates the same way at creation time, but because deps there must
/// already exist, `add` cannot introduce a cycle; `set-dependencies`
/// can, so the cycle check is additional here.
pub fn run_set_dependencies(plan_dir: &Path, id: &str, deps: &[String]) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    let target_index = backlog
        .tasks
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;

    if deps.iter().any(|d| d == id) {
        bail!("task {id:?} cannot depend on itself");
    }

    let existing_ids: HashSet<&str> = backlog.tasks.iter().map(|t| t.id.as_str()).collect();
    for dep in deps {
        if !existing_ids.contains(dep.as_str()) {
            bail!(
                "dependency id {dep:?} does not exist in backlog; known ids: {:?}",
                existing_ids
            );
        }
    }

    // Cycle check: adding edge id → d would close a cycle iff the
    // existing graph already has a path d → … → id.
    for dep in deps {
        if dependency_path_exists(&backlog, dep, id) {
            bail!(
                "refusing to set dependencies on {id:?}: would create a cycle through {dep:?}"
            );
        }
    }

    backlog.tasks[target_index].dependencies = deps.to_vec();
    write_backlog(plan_dir, &backlog)
}

/// True iff following `dependencies` edges from `from` can reach `to`.
fn dependency_path_exists(backlog: &BacklogFile, from: &str, to: &str) -> bool {
    let mut stack = vec![from];
    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(current) = stack.pop() {
        if current == to {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(task) = backlog.tasks.iter().find(|t| t.id == current) {
            for dep in &task.dependencies {
                stack.push(dep.as_str());
            }
        }
    }
    false
}

pub fn run_reorder(
    plan_dir: &Path,
    id: &str,
    position: ReorderPosition,
    target_id: &str,
) -> Result<()> {
    if id == target_id {
        bail!("cannot reorder a task relative to itself");
    }
    let mut backlog = read_backlog(plan_dir)?;
    let source_index = backlog
        .tasks
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("no task with id {id:?} in backlog"))?;
    let task = backlog.tasks.remove(source_index);

    let target_index = backlog
        .tasks
        .iter()
        .position(|t| t.id == target_id)
        .ok_or_else(|| anyhow::anyhow!("no target task with id {target_id:?} in backlog"))?;

    let insert_at = match position {
        ReorderPosition::Before => target_index,
        ReorderPosition::After => target_index + 1,
    };
    backlog.tasks.insert(insert_at, task);
    write_backlog(plan_dir, &backlog)
}

pub fn run_delete(plan_dir: &Path, id: &str, force: bool) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    let dependents: Vec<String> = backlog
        .tasks
        .iter()
        .filter(|t| t.dependencies.iter().any(|dep| dep == id))
        .map(|t| t.id.clone())
        .collect();

    if !dependents.is_empty() && !force {
        bail!(
            "refusing to delete {id}: task is a dependency of {:?}. Rerun with --force to cascade-remove the dep references.",
            dependents
        );
    }

    if force {
        for task in backlog.tasks.iter_mut() {
            task.dependencies.retain(|dep| dep != id);
        }
    }

    let before = backlog.tasks.len();
    backlog.tasks.retain(|t| t.id != id);
    if backlog.tasks.len() == before {
        bail!("no task with id {id:?} in backlog");
    }
    write_backlog(plan_dir, &backlog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::yaml_io::write_backlog;
    use tempfile::TempDir;

    fn make_task(id: &str, status: Status, deps: &[&str]) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            category: "maintenance".into(),
            status,
            blocked_reason: None,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            description: "body\n".into(),
            results: if status == Status::Done { Some("did it\n".into()) } else { None },
            handoff: None,
        }
    }

    fn sample_backlog() -> BacklogFile {
        BacklogFile {
            tasks: vec![
                make_task("foo", Status::Done, &[]),
                make_task("bar", Status::NotStarted, &["foo"]),
                make_task("baz", Status::NotStarted, &["bar"]),
                make_task("qux", Status::InProgress, &[]),
            ],
            extra: Default::default(),
        }
    }

    fn done_ids(backlog: &BacklogFile) -> HashSet<&str> {
        backlog
            .tasks
            .iter()
            .filter(|t| t.status == Status::Done)
            .map(|t| t.id.as_str())
            .collect()
    }

    #[test]
    fn ready_filter_excludes_tasks_with_unmet_deps() {
        let backlog = sample_backlog();
        let done = done_ids(&backlog);
        let filter = ListFilter { ready: true, ..Default::default() };
        let matches: Vec<&str> = backlog
            .tasks
            .iter()
            .filter(|t| task_matches(t, &filter, &done))
            .map(|t| t.id.as_str())
            .collect();
        // `bar` is not_started AND its only dep (`foo`) is done → ready.
        // `baz` is not_started BUT its dep (`bar`) is not done → not ready.
        // `foo` is done → excluded by status=not_started check.
        // `qux` is in_progress → excluded.
        assert_eq!(matches, vec!["bar"]);
    }

    #[test]
    fn status_filter_narrows_to_exact_match() {
        let backlog = sample_backlog();
        let done = done_ids(&backlog);
        let filter = ListFilter {
            status: Some(Status::NotStarted),
            ..Default::default()
        };
        let matches: Vec<&str> = backlog
            .tasks
            .iter()
            .filter(|t| task_matches(t, &filter, &done))
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(matches, vec!["bar", "baz"]);
    }

    #[test]
    fn missing_results_filter_matches_done_tasks_without_results() {
        let mut backlog = sample_backlog();
        backlog.tasks[0].results = None; // foo is done but has no Results.
        let done = done_ids(&backlog);
        let filter = ListFilter { missing_results: true, ..Default::default() };
        let matches: Vec<&str> = backlog
            .tasks
            .iter()
            .filter(|t| task_matches(t, &filter, &done))
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(matches, vec!["foo"]);
    }

    #[test]
    fn run_list_reads_yaml_and_emits_filtered_projection() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let backlog = read_backlog(tmp.path()).unwrap();
        let done = done_ids(&backlog);
        let filter = ListFilter { ready: true, ..Default::default() };
        let count = backlog.tasks.iter().filter(|t| task_matches(t, &filter, &done)).count();
        assert_eq!(count, 1, "one ready task (bar) expected");
    }

    #[test]
    fn find_task_returns_task_by_id() {
        let backlog = sample_backlog();
        let task = find_task(&backlog, "bar").unwrap();
        assert_eq!(task.id, "bar");
    }

    #[test]
    fn find_task_errors_when_id_not_found() {
        let backlog = sample_backlog();
        let err = find_task(&backlog, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include the bad id: {msg}");
    }

    #[test]
    fn run_add_appends_task_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let req = AddRequest {
            title: "New task".into(),
            category: "maintenance".into(),
            dependencies: vec!["foo".into()],
            description: "Description body.\n".into(),
        };
        run_add(tmp.path(), &req).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        assert_eq!(updated.tasks.last().unwrap().id, "new-task");
        assert_eq!(updated.tasks.last().unwrap().title, "New task");
        assert_eq!(updated.tasks.last().unwrap().dependencies, vec!["foo"]);
    }

    #[test]
    fn run_add_errors_when_dependency_ids_are_unknown() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let req = AddRequest {
            title: "New task".into(),
            category: "maintenance".into(),
            dependencies: vec!["nonexistent".into()],
            description: "body".into(),
        };
        let err = run_add(tmp.path(), &req).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must name the missing dep: {msg}");
    }

    #[test]
    fn run_init_populates_empty_backlog() {
        let tmp = TempDir::new().unwrap();
        let initial = BacklogFile {
            tasks: vec![],
            extra: Default::default(),
        };
        write_backlog(tmp.path(), &initial).unwrap();

        let seed = sample_backlog();
        run_init(tmp.path(), &seed).unwrap();

        let stored = read_backlog(tmp.path()).unwrap();
        assert_eq!(stored.tasks.len(), seed.tasks.len());
    }

    #[test]
    fn run_init_refuses_to_overwrite_non_empty_backlog() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_init(tmp.path(), &sample_backlog()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must explain the refusal: {msg}");
    }

    #[test]
    fn run_set_status_updates_the_target_task() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_status(tmp.path(), "bar", Status::InProgress, None).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert_eq!(bar.status, Status::InProgress);
        assert_eq!(bar.blocked_reason, None);
    }

    #[test]
    fn run_set_status_requires_reason_for_blocked() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_status(tmp.path(), "bar", Status::Blocked, None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("reason"), "error must mention reason: {msg}");

        // With a reason, it succeeds.
        run_set_status(tmp.path(), "bar", Status::Blocked, Some("upstream")).unwrap();
        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert_eq!(bar.status, Status::Blocked);
        assert_eq!(bar.blocked_reason.as_deref(), Some("upstream"));
    }

    #[test]
    fn run_set_status_clears_reason_when_moving_out_of_blocked() {
        let tmp = TempDir::new().unwrap();
        let mut backlog = sample_backlog();
        backlog.tasks[1].status = Status::Blocked;
        backlog.tasks[1].blocked_reason = Some("upstream".into());
        write_backlog(tmp.path(), &backlog).unwrap();

        run_set_status(tmp.path(), "bar", Status::NotStarted, None).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert_eq!(bar.blocked_reason, None, "blocked_reason must clear when status leaves blocked");
    }

    #[test]
    fn run_set_description_rewrites_body_and_preserves_other_fields() {
        let tmp = TempDir::new().unwrap();
        let mut seed = sample_backlog();
        // Seed `foo` with results + handoff so we can prove they survive.
        seed.tasks[0].results = Some("keep me\n".into());
        seed.tasks[0].handoff = Some("keep me too\n".into());
        write_backlog(tmp.path(), &seed).unwrap();

        run_set_description(tmp.path(), "foo", "Fresh brief.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.description, "Fresh brief.\n");
        assert_eq!(foo.results.as_deref(), Some("keep me\n"));
        assert_eq!(foo.handoff.as_deref(), Some("keep me too\n"));
        assert_eq!(foo.status, Status::Done);
        assert_eq!(foo.title, "foo");
    }

    #[test]
    fn run_set_description_ensures_trailing_newline() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_description(tmp.path(), "foo", "No newline at end").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.description, "No newline at end\n");
    }

    #[test]
    fn run_set_description_rejects_empty_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_description(tmp.path(), "foo", "").unwrap_err();
        assert!(
            format!("{err:#}").contains("empty"),
            "error must mention empty body: {err:#}"
        );

        // Disk unchanged.
        let reloaded = read_backlog(tmp.path()).unwrap();
        assert_eq!(reloaded.tasks[0].description, "body\n");
    }

    #[test]
    fn run_set_description_rejects_whitespace_only_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_description(tmp.path(), "foo", "   \n\t\n").unwrap_err();
        assert!(format!("{err:#}").contains("empty"));
    }

    #[test]
    fn run_set_description_errors_on_unknown_task_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_description(tmp.path(), "nonexistent", "Body.\n").unwrap_err();
        assert!(format!("{err:#}").contains("nonexistent"));
    }

    #[test]
    fn run_set_results_writes_markdown_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_results(tmp.path(), "foo", "Body of results.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.results.as_deref(), Some("Body of results.\n"));
    }

    #[test]
    fn run_set_handoff_writes_markdown_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_handoff(tmp.path(), "foo", "Promote follow-up.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.handoff.as_deref(), Some("Promote follow-up.\n"));
    }

    #[test]
    fn run_clear_handoff_nulls_the_field() {
        let tmp = TempDir::new().unwrap();
        let mut backlog = sample_backlog();
        backlog.tasks[0].handoff = Some("some handoff\n".into());
        write_backlog(tmp.path(), &backlog).unwrap();

        run_clear_handoff(tmp.path(), "foo").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.handoff, None);
    }

    #[test]
    fn run_set_title_updates_title_but_preserves_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_title(tmp.path(), "bar", "Bar's New Title").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert_eq!(bar.title, "Bar's New Title");
        assert_eq!(bar.id, "bar", "id must not change when title changes");
    }

    #[test]
    fn run_reorder_before_moves_task_to_earlier_position() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_reorder(tmp.path(), "qux", ReorderPosition::Before, "foo").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let ids: Vec<&str> = updated.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["qux", "foo", "bar", "baz"]);
    }

    #[test]
    fn run_reorder_after_moves_task_to_later_position() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_reorder(tmp.path(), "foo", ReorderPosition::After, "baz").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let ids: Vec<&str> = updated.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["bar", "baz", "foo", "qux"]);
    }

    #[test]
    fn run_set_dependencies_replaces_the_dependency_list() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        // `bar` starts with deps ["foo"]; swap to ["qux"].
        run_set_dependencies(tmp.path(), "bar", &["qux".into()]).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert_eq!(bar.dependencies, vec!["qux".to_string()]);
    }

    #[test]
    fn run_set_dependencies_clears_when_empty_slice_is_passed() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_dependencies(tmp.path(), "bar", &[]).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert!(bar.dependencies.is_empty());
    }

    #[test]
    fn run_set_dependencies_errors_on_unknown_task_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "nonexistent", &["foo".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must cite the bad id: {msg}");
    }

    #[test]
    fn run_set_dependencies_errors_on_unknown_dependency_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "bar", &["ghost".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "error must name the unknown dep: {msg}");
    }

    #[test]
    fn run_set_dependencies_rejects_self_reference() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "bar", &["bar".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("itself") || msg.contains("self"),
            "error must call out the self-reference: {msg}");
    }

    #[test]
    fn run_set_dependencies_rejects_cycles() {
        let tmp = TempDir::new().unwrap();
        // baz → bar → foo. Making foo depend on baz closes a cycle
        // foo → baz → bar → foo.
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "foo", &["baz".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cycle"), "error must mention cycle: {msg}");

        // Disk is unchanged — foo still has no deps.
        let reloaded = read_backlog(tmp.path()).unwrap();
        let foo = reloaded.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert!(foo.dependencies.is_empty(), "cycle rejection must not persist the bad write");
    }

    #[test]
    fn run_delete_errors_when_task_has_dependents() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        // `foo` is a dep of `bar`; deletion must refuse by default.
        let err = run_delete(tmp.path(), "foo", false).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("bar"), "error must cite the dependent: {msg}");
    }

    #[test]
    fn run_delete_with_force_cascades_dep_reference_cleanup() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_delete(tmp.path(), "foo", true).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        assert!(!updated.tasks.iter().any(|t| t.id == "foo"));
        // `bar`'s dep on `foo` must be stripped, not left dangling.
        let bar = updated.tasks.iter().find(|t| t.id == "bar").unwrap();
        assert!(!bar.dependencies.contains(&"foo".to_string()));
    }
}
