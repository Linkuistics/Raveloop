//! Typed schema for `<plan>/backlog.yaml`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotStarted,
    InProgress,
    Done,
    Blocked,
}

impl Status {
    pub fn parse(input: &str) -> Option<Status> {
        match input {
            "not_started" => Some(Status::NotStarted),
            "in_progress" => Some(Status::InProgress),
            "done" => Some(Status::Done),
            "blocked" => Some(Status::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub category: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacklogFile {
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// Preserve unknown top-level keys so a roundtrip through older readers
    /// never drops fields a newer writer added. Future-proofs the schema
    /// against the R2–R5 extensions.
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_yaml::Value>,
}

/// Per-status tally of a backlog's tasks. Computed from a parsed
/// `BacklogFile` via `BacklogFile::task_counts` so survey (and any
/// other caller) never has to ask an LLM to count — mechanical work
/// belongs in Rust.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCounts {
    pub total: usize,
    pub not_started: usize,
    pub in_progress: usize,
    pub done: usize,
    pub blocked: usize,
}

impl BacklogFile {
    /// Tally tasks by status. `total` is the length of the task list;
    /// the per-status fields are exact counts of tasks with that
    /// `Status`. A task always contributes to exactly one per-status
    /// field, so the sum of `not_started + in_progress + done + blocked`
    /// equals `total`.
    pub fn task_counts(&self) -> TaskCounts {
        let mut counts = TaskCounts {
            total: self.tasks.len(),
            ..TaskCounts::default()
        };
        for task in &self.tasks {
            match task.status {
                Status::NotStarted => counts.not_started += 1,
                Status::InProgress => counts.in_progress += 1,
                Status::Done => counts.done += 1,
                Status::Blocked => counts.blocked += 1,
            }
        }
        counts
    }
}

/// Derive a slug from a task title. Lowercase, non-alphanumerics → `-`,
/// collapse repeats, trim leading/trailing `-`. Used at task creation;
/// the slug is persisted as `Task::id` and never recomputed on read.
pub fn slug_from_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut prev_dash = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Assign `slug_from_title(title)` with a numeric suffix to avoid
/// collisions with `existing_ids`. First attempt has no suffix; the
/// second is `-2`, third `-3`, etc.
pub fn allocate_id<'a>(title: &str, existing_ids: impl IntoIterator<Item = &'a str>) -> String {
    let base = slug_from_title(title);
    let existing: std::collections::HashSet<&str> = existing_ids.into_iter().collect();
    if !existing.contains(base.as_str()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_yaml() {
        for status in [Status::NotStarted, Status::InProgress, Status::Done, Status::Blocked] {
            let serialised = serde_yaml::to_string(&status).unwrap();
            let parsed: Status = serde_yaml::from_str(&serialised).unwrap();
            assert_eq!(status, parsed, "roundtrip failed for {status:?}");
        }
    }

    #[test]
    fn status_parse_accepts_snake_case_cli_input() {
        assert_eq!(Status::parse("not_started"), Some(Status::NotStarted));
        assert_eq!(Status::parse("in_progress"), Some(Status::InProgress));
        assert_eq!(Status::parse("done"), Some(Status::Done));
        assert_eq!(Status::parse("blocked"), Some(Status::Blocked));
        assert_eq!(Status::parse("NotStarted"), None);
        assert_eq!(Status::parse(""), None);
    }

    #[test]
    fn slug_from_title_lowercases_and_punctuation_maps_to_dash() {
        assert_eq!(
            slug_from_title("Add clippy `-D warnings` CI gate"),
            "add-clippy-d-warnings-ci-gate"
        );
        assert_eq!(
            slug_from_title("Research: expose plan-state data"),
            "research-expose-plan-state-data"
        );
        assert_eq!(
            slug_from_title("  trim leading/trailing  "),
            "trim-leading-trailing"
        );
    }

    #[test]
    fn allocate_id_suffixes_on_collision() {
        let existing = ["foo", "foo-2"];
        assert_eq!(allocate_id("Foo", existing), "foo-3");
        assert_eq!(allocate_id("Foo!", existing), "foo-3");
        assert_eq!(allocate_id("Bar", existing), "bar");
    }

    #[test]
    fn task_round_trips_preserving_optional_fields() {
        let task = Task {
            id: "example".into(),
            title: "Example task".into(),
            category: "maintenance".into(),
            status: Status::NotStarted,
            blocked_reason: None,
            dependencies: vec![],
            description: "Body.\n".into(),
            results: None,
            handoff: None,
        };
        let yaml = serde_yaml::to_string(&task).unwrap();
        // `skip_serializing_if` keeps optional nulls out of the wire form.
        assert!(!yaml.contains("blocked_reason"), "optional None must not emit: {yaml}");
        assert!(!yaml.contains("results"), "optional None must not emit: {yaml}");
        assert!(!yaml.contains("handoff"), "optional None must not emit: {yaml}");
        let decoded: Task = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.id, task.id);
        assert_eq!(decoded.status, task.status);
    }

    #[test]
    fn task_counts_tallies_every_status_and_sums_to_total() {
        fn task(status: Status) -> Task {
            Task {
                id: format!("t-{status:?}").to_lowercase(),
                title: "t".into(),
                category: "maintenance".into(),
                status,
                blocked_reason: if status == Status::Blocked {
                    Some("upstream".into())
                } else {
                    None
                },
                dependencies: vec![],
                description: "body\n".into(),
                results: None,
                handoff: None,
            }
        }
        let backlog = BacklogFile {
            tasks: vec![
                task(Status::NotStarted),
                task(Status::NotStarted),
                task(Status::InProgress),
                task(Status::Done),
                task(Status::Blocked),
            ],
            extra: Default::default(),
        };
        let counts = backlog.task_counts();
        assert_eq!(counts.total, 5);
        assert_eq!(counts.not_started, 2);
        assert_eq!(counts.in_progress, 1);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.blocked, 1);
        assert_eq!(
            counts.not_started + counts.in_progress + counts.done + counts.blocked,
            counts.total,
            "per-status sum must equal total"
        );
    }

    #[test]
    fn task_counts_on_empty_backlog_is_all_zero() {
        let backlog = BacklogFile::default();
        let counts = backlog.task_counts();
        assert_eq!(counts, TaskCounts::default());
        assert_eq!(counts.total, 0);
    }

    #[test]
    fn backlog_file_preserves_unknown_top_level_keys() {
        // Future schema extensions (R2+) may add top-level keys. The flatten
        // extra buffer keeps them alive across an R1 read/write cycle.
        let input = r#"
tasks: []
schema_version: 1
"#;
        let parsed: BacklogFile = serde_yaml::from_str(input).unwrap();
        assert!(parsed.extra.contains_key("schema_version"));
        let re_emitted = serde_yaml::to_string(&parsed).unwrap();
        assert!(re_emitted.contains("schema_version"), "extra keys must round-trip: {re_emitted}");
    }
}
