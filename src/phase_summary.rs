//! Deterministic labelled-line summary for the triage, reflect, and
//! dream phases.
//!
//! Diffs `backlog.yaml` (triage) or `memory.yaml` (reflect/dream)
//! between a baseline commit and the current working-tree state, then
//! emits the labelled summary lines that the phase prompts currently
//! ask the LLM to author — extracting the mechanical transcription of
//! the diff to Rust while preserving the LLM's narrative preamble.
//!
//! ## Structural-only labels
//!
//! The intent-carrying labels (`[PROMOTED]` / `[ARCHIVED]` / `[BLOCKER]`
//! as subtypes of `[NEW]`; `[IMPRECISE]` as a subtype of `[STALE]`;
//! `[OVERLAPPING]` / `[VERBOSE]` / `[AWKWARD]` as subtypes of dream
//! rewrites) cannot be recovered from a pure file diff — they require
//! knowledge of operator intent that the mutation alone does not
//! carry. This renderer emits only the structural labels derivable
//! from the diff. The richer intent distinction remains in the LLM's
//! reasoning preamble, which every phase prompt explicitly preserves.
//!
//! Adding intent tagging later (a sidecar `ops.log.yaml` written by
//! `--intent <label>` flags on the mutating verbs) would upgrade the
//! renderer without breaking the current structural contract.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::state::backlog::schema::{BacklogFile, Status};
use crate::state::filenames::{BACKLOG_FILENAME, MEMORY_FILENAME};
use crate::state::memory::schema::MemoryFile;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Phase {
    Triage,
    Reflect,
    Dream,
}

impl Phase {
    pub fn parse(input: &str) -> Option<Phase> {
        match input {
            "triage" => Some(Phase::Triage),
            "reflect" => Some(Phase::Reflect),
            "dream" => Some(Phase::Dream),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderFormat {
    Text,
    Yaml,
}

impl RenderFormat {
    pub fn parse(input: &str) -> Option<RenderFormat> {
        match input {
            "text" => Some(RenderFormat::Text),
            "yaml" => Some(RenderFormat::Yaml),
            _ => None,
        }
    }
}

/// One labelled entry in the phase summary. `kind` is the bracketed
/// label (e.g. `"NEW"`, `"DONE"`, `"STATS"`); `subject` is the body
/// text that follows; `continuation` is an optional second line for
/// dream's two-line `→` entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub kind: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

impl Label {
    fn simple(kind: &str, subject: impl Into<String>) -> Label {
        Label {
            kind: kind.to_string(),
            subject: subject.into(),
            continuation: None,
        }
    }

    fn two_line(kind: &str, subject: impl Into<String>, continuation: impl Into<String>) -> Label {
        Label {
            kind: kind.to_string(),
            subject: subject.into(),
            continuation: Some(continuation.into()),
        }
    }
}

pub fn run_render(
    plan_dir: &Path,
    phase: Phase,
    baseline_sha: &str,
    format: RenderFormat,
) -> Result<()> {
    let labels = compute_labels(plan_dir, phase, baseline_sha)?;
    let output = format_labels(&labels, format)?;
    print!("{output}");
    Ok(())
}

pub fn compute_labels(plan_dir: &Path, phase: Phase, baseline_sha: &str) -> Result<Vec<Label>> {
    match phase {
        Phase::Triage => compute_triage_labels_from_disk(plan_dir, baseline_sha),
        Phase::Reflect => compute_reflect_labels_from_disk(plan_dir, baseline_sha),
        Phase::Dream => compute_dream_labels_from_disk(plan_dir, baseline_sha),
    }
}

// ----- Triage --------------------------------------------------------

fn compute_triage_labels_from_disk(plan_dir: &Path, baseline_sha: &str) -> Result<Vec<Label>> {
    let current = crate::state::backlog::read_backlog(plan_dir)?;
    let baseline = read_baseline_yaml::<BacklogFile>(plan_dir, BACKLOG_FILENAME, baseline_sha)?
        .unwrap_or_default();
    Ok(compute_triage_labels(&baseline, &current))
}

/// Triage labels recoverable from a structural diff:
/// - `[DONE] <title>` for any task whose status flipped to `Done`
/// - `[NEW] <title>` for any task id not present in the baseline
/// - `[OBSOLETE] <title>` for any task id missing from the current
/// - `[REPRIORITISED] <title>` for any task whose position changed
///
/// Ordering is stable and deterministic: DONE first, then NEW, then
/// REPRIORITISED, then OBSOLETE — each group in the order tasks appear
/// in the relevant file (current for DONE/NEW/REPRIORITISED; baseline
/// for OBSOLETE).
pub fn compute_triage_labels(baseline: &BacklogFile, current: &BacklogFile) -> Vec<Label> {
    use std::collections::HashMap;

    let baseline_by_id: HashMap<&str, &crate::state::backlog::schema::Task> = baseline
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    let baseline_positions: HashMap<&str, usize> = baseline
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let current_ids: std::collections::HashSet<&str> =
        current.tasks.iter().map(|t| t.id.as_str()).collect();

    let mut done = Vec::new();
    let mut new_tasks = Vec::new();
    let mut reprioritised = Vec::new();

    for (pos, task) in current.tasks.iter().enumerate() {
        match baseline_by_id.get(task.id.as_str()) {
            Some(prev) if prev.status != Status::Done && task.status == Status::Done => {
                done.push(Label::simple("DONE", task.title.clone()));
            }
            Some(_) => {
                if let Some(&prev_pos) = baseline_positions.get(task.id.as_str()) {
                    if prev_pos != pos {
                        reprioritised.push(Label::simple("REPRIORITISED", task.title.clone()));
                    }
                }
            }
            None => new_tasks.push(Label::simple("NEW", task.title.clone())),
        }
    }

    let mut obsolete = Vec::new();
    for task in &baseline.tasks {
        if !current_ids.contains(task.id.as_str()) {
            obsolete.push(Label::simple("OBSOLETE", task.title.clone()));
        }
    }

    done.into_iter()
        .chain(new_tasks)
        .chain(reprioritised)
        .chain(obsolete)
        .collect()
}

// ----- Reflect -------------------------------------------------------

fn compute_reflect_labels_from_disk(plan_dir: &Path, baseline_sha: &str) -> Result<Vec<Label>> {
    let current = crate::state::memory::read_memory(plan_dir)?;
    let baseline = read_baseline_yaml::<MemoryFile>(plan_dir, MEMORY_FILENAME, baseline_sha)?
        .unwrap_or_default();
    Ok(compute_reflect_labels(&baseline, &current))
}

/// Reflect labels recoverable from a structural diff:
/// - `[NEW] <heading>` for any memory id not present in the baseline
/// - `[OBSOLETE] <heading>` for any memory id missing from the current
/// - `[STALE] <heading>` for any memory entry whose body or title
///   differs from baseline
///
/// Dream reuses this same shape; the only distinction is the `[STATS]`
/// line dream appends (see `compute_dream_labels`).
pub fn compute_reflect_labels(baseline: &MemoryFile, current: &MemoryFile) -> Vec<Label> {
    use std::collections::HashMap;

    let baseline_by_id: HashMap<&str, &crate::state::memory::schema::MemoryEntry> = baseline
        .entries
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let current_ids: std::collections::HashSet<&str> =
        current.entries.iter().map(|e| e.id.as_str()).collect();

    let mut new_entries = Vec::new();
    let mut stale = Vec::new();
    for entry in &current.entries {
        match baseline_by_id.get(entry.id.as_str()) {
            Some(prev) if prev.title != entry.title || prev.body != entry.body => {
                stale.push(Label::simple("STALE", entry.title.clone()));
            }
            Some(_) => {}
            None => new_entries.push(Label::simple("NEW", entry.title.clone())),
        }
    }

    let mut obsolete = Vec::new();
    for entry in &baseline.entries {
        if !current_ids.contains(entry.id.as_str()) {
            obsolete.push(Label::simple("OBSOLETE", entry.title.clone()));
        }
    }

    new_entries
        .into_iter()
        .chain(stale)
        .chain(obsolete)
        .collect()
}

// ----- Dream ---------------------------------------------------------

fn compute_dream_labels_from_disk(plan_dir: &Path, baseline_sha: &str) -> Result<Vec<Label>> {
    let current = crate::state::memory::read_memory(plan_dir)?;
    let baseline = read_baseline_yaml::<MemoryFile>(plan_dir, MEMORY_FILENAME, baseline_sha)?
        .unwrap_or_default();
    Ok(compute_dream_labels(&baseline, &current))
}

/// Dream labels: same NEW / STALE / OBSOLETE as reflect, plus a final
/// `[STATS] <before> → <after>` two-line entry carrying the pre- and
/// post-rewrite word counts. Dream's contract is strictly lossless
/// prose tightening, so the STATS line is the signal that the rewrite
/// happened; an unchanged word count is still emitted so the summary
/// is self-describing.
pub fn compute_dream_labels(baseline: &MemoryFile, current: &MemoryFile) -> Vec<Label> {
    let mut labels = compute_reflect_labels(baseline, current);
    let before = memory_word_count(baseline);
    let after = memory_word_count(current);
    labels.push(Label::two_line(
        "STATS",
        before.to_string(),
        after.to_string(),
    ));
    labels
}

fn memory_word_count(memory: &MemoryFile) -> usize {
    memory
        .entries
        .iter()
        .map(|e| word_count(&e.title) + word_count(&e.body))
        .sum()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

// ----- Baseline reader ----------------------------------------------

/// Read a YAML file from `plan_dir` at `baseline_sha` and parse as `T`.
/// Returns `Ok(None)` when the file does not exist at that commit
/// (first-cycle case — caller treats as empty baseline). Errors propagate
/// only for genuine failures (git unavailable, malformed YAML).
fn read_baseline_yaml<T: serde::de::DeserializeOwned>(
    plan_dir: &Path,
    filename: &str,
    baseline_sha: &str,
) -> Result<Option<T>> {
    if baseline_sha.trim().is_empty() {
        return Ok(None);
    }

    let full_name_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["ls-files", "--full-name", filename])
        .output()
        .map_err(|e| anyhow!("git ls-files failed: {e}"))?;

    if !full_name_out.status.success() {
        return Err(anyhow!(
            "git ls-files exited {}: {}",
            full_name_out.status,
            String::from_utf8_lossy(&full_name_out.stderr).trim()
        ));
    }

    let full_name = String::from_utf8_lossy(&full_name_out.stdout)
        .trim()
        .to_string();
    if full_name.is_empty() {
        return Ok(None);
    }

    let show_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["show", &format!("{baseline_sha}:{full_name}")])
        .output()
        .map_err(|e| anyhow!("git show failed: {e}"))?;

    if !show_out.status.success() {
        // File did not exist at this SHA (most likely first cycle).
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&show_out.stdout).into_owned();
    let parsed = serde_yaml::from_str::<T>(&text)
        .map_err(|e| anyhow!("baseline {filename} YAML parse: {e}"))?;
    Ok(Some(parsed))
}

// ----- Output formatting --------------------------------------------

fn format_labels(labels: &[Label], format: RenderFormat) -> Result<String> {
    match format {
        RenderFormat::Text => {
            let mut out = String::new();
            for label in labels {
                out.push_str(&format!("[{}] {}\n", label.kind, label.subject));
                if let Some(cont) = &label.continuation {
                    out.push_str(&format!("       → {cont}\n"));
                }
            }
            Ok(out)
        }
        RenderFormat::Yaml => Ok(serde_yaml::to_string(labels)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::schema::Task;
    use crate::state::memory::schema::MemoryEntry;

    fn task(id: &str, title: &str, status: Status) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            category: "core".into(),
            status,
            blocked_reason: if status == Status::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: vec![],
            description: "body\n".into(),
            results: if status == Status::Done {
                Some("done\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    fn mem(id: &str, title: &str, body: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.into(),
            title: title.into(),
            body: body.into(),
        }
    }

    // ---- Triage ----

    #[test]
    fn triage_with_no_mutations_emits_no_labels() {
        let backlog = BacklogFile {
            tasks: vec![task("a", "A", Status::NotStarted)],
            extra: Default::default(),
        };
        assert!(compute_triage_labels(&backlog, &backlog).is_empty());
    }

    #[test]
    fn triage_emits_new_label_for_tasks_absent_in_baseline() {
        let baseline = BacklogFile::default();
        let current = BacklogFile {
            tasks: vec![task("a", "Alpha", Status::NotStarted)],
            extra: Default::default(),
        };
        let labels = compute_triage_labels(&baseline, &current);
        assert_eq!(labels, vec![Label::simple("NEW", "Alpha")]);
    }

    #[test]
    fn triage_emits_done_label_only_for_status_flips_into_done() {
        let baseline = BacklogFile {
            tasks: vec![
                task("a", "Alpha", Status::NotStarted),
                task("b", "Bravo", Status::Done),
            ],
            extra: Default::default(),
        };
        let current = BacklogFile {
            tasks: vec![
                task("a", "Alpha", Status::Done),
                task("b", "Bravo", Status::Done),
            ],
            extra: Default::default(),
        };
        let labels = compute_triage_labels(&baseline, &current);
        assert_eq!(labels, vec![Label::simple("DONE", "Alpha")]);
    }

    #[test]
    fn triage_emits_obsolete_label_for_baseline_ids_missing_from_current() {
        let baseline = BacklogFile {
            tasks: vec![
                task("a", "Alpha", Status::NotStarted),
                task("b", "Bravo", Status::NotStarted),
            ],
            extra: Default::default(),
        };
        let current = BacklogFile {
            tasks: vec![task("a", "Alpha", Status::NotStarted)],
            extra: Default::default(),
        };
        let labels = compute_triage_labels(&baseline, &current);
        assert_eq!(labels, vec![Label::simple("OBSOLETE", "Bravo")]);
    }

    #[test]
    fn triage_emits_reprioritised_when_index_position_changes() {
        let baseline = BacklogFile {
            tasks: vec![
                task("a", "Alpha", Status::NotStarted),
                task("b", "Bravo", Status::NotStarted),
            ],
            extra: Default::default(),
        };
        let current = BacklogFile {
            tasks: vec![
                task("b", "Bravo", Status::NotStarted),
                task("a", "Alpha", Status::NotStarted),
            ],
            extra: Default::default(),
        };
        let labels = compute_triage_labels(&baseline, &current);
        // Both moved; both get the label.
        assert_eq!(
            labels,
            vec![
                Label::simple("REPRIORITISED", "Bravo"),
                Label::simple("REPRIORITISED", "Alpha"),
            ]
        );
    }

    #[test]
    fn triage_orders_done_before_new_before_reprioritised_before_obsolete() {
        let baseline = BacklogFile {
            tasks: vec![
                task("stale", "Stale", Status::NotStarted),
                task("kept", "Kept", Status::NotStarted),
                task("finish", "Finish", Status::InProgress),
            ],
            extra: Default::default(),
        };
        let current = BacklogFile {
            tasks: vec![
                task("kept", "Kept", Status::NotStarted),  // reprioritised
                task("finish", "Finish", Status::Done),    // done
                task("fresh", "Fresh", Status::NotStarted), // new
            ],
            extra: Default::default(),
        };
        let labels = compute_triage_labels(&baseline, &current);
        let kinds: Vec<&str> = labels.iter().map(|l| l.kind.as_str()).collect();
        assert_eq!(kinds, vec!["DONE", "NEW", "REPRIORITISED", "OBSOLETE"]);
    }

    // ---- Reflect ----

    #[test]
    fn reflect_emits_stale_when_body_changes_for_same_id() {
        let baseline = MemoryFile {
            entries: vec![mem("foo", "Foo rule", "old body\n")],
            extra: Default::default(),
        };
        let current = MemoryFile {
            entries: vec![mem("foo", "Foo rule", "new tighter body\n")],
            extra: Default::default(),
        };
        let labels = compute_reflect_labels(&baseline, &current);
        assert_eq!(labels, vec![Label::simple("STALE", "Foo rule")]);
    }

    #[test]
    fn reflect_emits_stale_when_title_changes_even_if_body_identical() {
        let baseline = MemoryFile {
            entries: vec![mem("foo", "Old title", "same body\n")],
            extra: Default::default(),
        };
        let current = MemoryFile {
            entries: vec![mem("foo", "New title", "same body\n")],
            extra: Default::default(),
        };
        let labels = compute_reflect_labels(&baseline, &current);
        assert_eq!(labels, vec![Label::simple("STALE", "New title")]);
    }

    #[test]
    fn reflect_emits_new_and_obsolete_for_added_and_removed_entries() {
        let baseline = MemoryFile {
            entries: vec![mem("old", "Retired rule", "body\n")],
            extra: Default::default(),
        };
        let current = MemoryFile {
            entries: vec![mem("fresh", "Fresh rule", "body\n")],
            extra: Default::default(),
        };
        let labels = compute_reflect_labels(&baseline, &current);
        assert_eq!(
            labels,
            vec![
                Label::simple("NEW", "Fresh rule"),
                Label::simple("OBSOLETE", "Retired rule"),
            ]
        );
    }

    #[test]
    fn reflect_with_no_changes_emits_no_labels() {
        let memory = MemoryFile {
            entries: vec![mem("a", "A", "body\n")],
            extra: Default::default(),
        };
        assert!(compute_reflect_labels(&memory, &memory).is_empty());
    }

    // ---- Dream ----

    #[test]
    fn dream_always_appends_a_stats_two_line_entry() {
        let memory = MemoryFile {
            entries: vec![mem("a", "A", "one two three\n")],
            extra: Default::default(),
        };
        let labels = compute_dream_labels(&memory, &memory);
        assert_eq!(labels.len(), 1, "unchanged memory still emits STATS");
        let stats = &labels[0];
        assert_eq!(stats.kind, "STATS");
        assert_eq!(stats.subject, "4"); // "A" + "one two three" = 4 words
        assert_eq!(stats.continuation.as_deref(), Some("4"));
    }

    #[test]
    fn dream_stats_reports_before_and_after_word_counts_distinctly() {
        let baseline = MemoryFile {
            entries: vec![mem(
                "a",
                "A",
                "one two three four five six seven eight nine ten\n",
            )],
            extra: Default::default(),
        };
        let current = MemoryFile {
            entries: vec![mem("a", "A", "shorter body here\n")],
            extra: Default::default(),
        };
        let labels = compute_dream_labels(&baseline, &current);
        let stats = labels.iter().find(|l| l.kind == "STATS").unwrap();
        assert_eq!(stats.subject, "11"); // "A" + 10 words
        assert_eq!(stats.continuation.as_deref(), Some("4")); // "A" + 3 words
    }

    #[test]
    fn dream_consolidation_surfaces_as_new_plus_obsolete_plus_stats() {
        // Merging A + B into C — renderer can't know it's a merge,
        // so it emits structural labels: NEW for C, OBSOLETE for A and B.
        let baseline = MemoryFile {
            entries: vec![
                mem("a", "Fact A", "one two\n"),
                mem("b", "Fact B", "three four\n"),
            ],
            extra: Default::default(),
        };
        let current = MemoryFile {
            entries: vec![mem("c", "Merged fact", "merged content\n")],
            extra: Default::default(),
        };
        let labels = compute_dream_labels(&baseline, &current);
        let kinds: Vec<&str> = labels.iter().map(|l| l.kind.as_str()).collect();
        assert_eq!(kinds, vec!["NEW", "OBSOLETE", "OBSOLETE", "STATS"]);
    }

    // ---- Output formatting ----

    #[test]
    fn format_text_emits_one_line_per_simple_label() {
        let labels = vec![
            Label::simple("NEW", "Alpha"),
            Label::simple("OBSOLETE", "Bravo"),
        ];
        let out = format_labels(&labels, RenderFormat::Text).unwrap();
        assert_eq!(out, "[NEW] Alpha\n[OBSOLETE] Bravo\n");
    }

    #[test]
    fn format_text_emits_continuation_line_for_two_line_labels() {
        let labels = vec![Label::two_line("STATS", "42", "23")];
        let out = format_labels(&labels, RenderFormat::Text).unwrap();
        assert_eq!(out, "[STATS] 42\n       → 23\n");
    }

    #[test]
    fn format_yaml_emits_sequence_skipping_none_continuation() {
        let labels = vec![Label::simple("NEW", "Alpha")];
        let out = format_labels(&labels, RenderFormat::Yaml).unwrap();
        assert!(out.contains("kind: NEW"));
        assert!(out.contains("subject: Alpha"));
        assert!(!out.contains("continuation:"), "None must skip serialize: {out}");
    }

    #[test]
    fn empty_summary_in_text_format_is_an_empty_string() {
        let out = format_labels(&[], RenderFormat::Text).unwrap();
        assert_eq!(out, "");
    }

    // ---- Parsers ----

    #[test]
    fn phase_parse_accepts_the_three_supported_names() {
        assert_eq!(Phase::parse("triage"), Some(Phase::Triage));
        assert_eq!(Phase::parse("reflect"), Some(Phase::Reflect));
        assert_eq!(Phase::parse("dream"), Some(Phase::Dream));
        assert_eq!(Phase::parse("work"), None);
        assert_eq!(Phase::parse(""), None);
    }

    #[test]
    fn render_format_parse_accepts_text_and_yaml_only() {
        assert_eq!(RenderFormat::parse("text"), Some(RenderFormat::Text));
        assert_eq!(RenderFormat::parse("yaml"), Some(RenderFormat::Yaml));
        assert_eq!(RenderFormat::parse("json"), None);
    }
}
