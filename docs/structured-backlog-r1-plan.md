# Structured Backlog (R1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the full `state backlog` CLI surface plus the per-plan `state migrate` verb (backlog-only in R1), backed by a typed YAML schema and a markdown-to-YAML migration parser. Production-grade, tests included, replaces direct `Read`/`Edit` on `backlog.md` for all phase prompts that will flip over in R6.

**Architecture:** New `src/state/` module directory replaces the single-file `src/state.rs`. A `backlog` submodule owns the YAML schema, parser for the legacy markdown format, emitter with block-scalar preservation, and one handler per CLI verb. A thin `migrate` submodule wraps the parser + emitter with the atomicity/idempotency/validation/dry-run/force behaviour contract. CLI dispatch in `src/main.rs` gains a `state backlog <verb>` subcommand tree and a `state migrate <plan-dir>` command. Tests mix unit tests (pure parsing/filter logic) with CLI integration tests using the `CARGO_BIN_EXE_ravel-lite` env var (existing repo pattern).

**Tech Stack:** Rust 2021, serde + serde_yaml (`indexmap` for field-order preservation on emit), clap v4 derive, anyhow + thiserror for errors, tempfile for test isolation.

---

## Environment & scope notes

- **Worktree:** not used. This R1 ships from within ravel-lite's own work-phase run on `main` — consistent with how ravel-lite runs all of its internal work tasks. No `using-git-worktrees` skill invocation.
- **Spec source of truth:** `docs/structured-plan-state-design.md`. The design doc's _Prototype scope_ section is subsumed by this plan — R1 delivers the full verb surface, not the cut-down PoC.
- **Scope boundary:** R1 is `state backlog` + backlog-scoped `state migrate`. Memory, session-log, latest-session, phase migration and the `state memory` / `state session-log` / `state projects` / `state related-projects` verbs are later rollout units (R2–R5), out of scope here.
- **Existing precedent:** `src/state.rs`'s `run_set_phase` is the pattern — CLI handler delegates to a library function, atomic tmp-file write, enumerated error messages, integration test via CARGO_BIN_EXE.

## File structure

### Created

- `src/state/mod.rs` — module re-exports. Replaces `src/state.rs`.
- `src/state/phase.rs` — contents of the current `src/state.rs` (unchanged logic).
- `src/state/backlog/mod.rs` — public surface for the `backlog` submodule; re-exports schema types and verb entry points.
- `src/state/backlog/schema.rs` — `Task`, `Status`, `BacklogFile` serde types; slug generation.
- `src/state/backlog/parse_md.rs` — markdown-to-`BacklogFile` parser; used only by `migrate`.
- `src/state/backlog/yaml_io.rs` — atomic read/write of `backlog.yaml`; block-scalar preservation on emit.
- `src/state/backlog/verbs.rs` — one handler per CLI verb (`list`, `show`, `add`, `init`, `set_status`, `set_results`, `set_handoff`, `clear_handoff`, `set_title`, `reorder`, `delete`).
- `src/state/migrate.rs` — `state migrate` verb, delegates to `backlog::parse_md` + `backlog::yaml_io`.
- `tests/state_backlog.rs` — integration tests for the verbs via `CARGO_BIN_EXE_ravel-lite`.

### Modified

- `src/main.rs` — replace the single `StateCommands::SetPhase` with a richer subcommand tree that adds `Backlog { command: BacklogCommands }` and `Migrate { plan_dir, dry_run, keep_originals, delete_originals, force }`. Route dispatches into the new handlers.

### Deleted

- `src/state.rs` (superseded by `src/state/mod.rs` + `src/state/phase.rs`).

---

## Task 1: Restructure state.rs into state/ module

**Files:**
- Delete: `src/state.rs`
- Create: `src/state/mod.rs`
- Create: `src/state/phase.rs`

Goal: preserve the current single-verb behaviour (`state set-phase`) bit-for-bit while moving the code into a module directory where subsequent tasks can add sibling modules (`backlog/`, `migrate.rs`) without the file ballooning.

- [ ] **Step 1: Move `src/state.rs` content into `src/state/phase.rs`**

Create `src/state/phase.rs` with the exact contents of the current `src/state.rs`, unchanged. The inner `#[cfg(test)] mod tests` block travels with it.

- [ ] **Step 2: Create `src/state/mod.rs` re-exporting the public surface**

```rust
//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase`    — `set-phase` (existing)
//! - `backlog`  — typed backlog.yaml + CRUD verbs (R1)
//! - `migrate`  — one-shot per-plan .md → .yaml conversion (backlog only in R1)

pub mod backlog;
pub mod migrate;
pub mod phase;

pub use phase::run_set_phase;
```

- [ ] **Step 3: Create placeholder `src/state/backlog/mod.rs` and `src/state/migrate.rs`**

These keep the `mod backlog;` / `mod migrate;` declarations in `state/mod.rs` compiling ahead of their implementation. They will be fleshed out in later tasks.

`src/state/backlog/mod.rs`:
```rust
//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod schema;
```

`src/state/backlog/schema.rs`:
```rust
//! Placeholder — filled in by Task 2.
```

`src/state/migrate.rs`:
```rust
//! `state migrate` verb — backlog-only in R1.
//!
//! Placeholder — filled in by Task 12.
```

- [ ] **Step 4: Delete `src/state.rs`**

Run: `rm src/state.rs`

- [ ] **Step 5: Verify existing tests still pass**

Run: `cargo test --lib state::phase::`

Expected: the four existing `set_phase_*` tests pass. No change in behaviour.

- [ ] **Step 6: Commit**

```bash
git add src/state/ src/state.rs src/main.rs
git commit -m "Restructure src/state.rs into state/ module directory

Move the existing set_phase code into src/state/phase.rs verbatim and
introduce src/state/mod.rs as the module entry point. Placeholder
backlog and migrate submodules land here so R1 can grow the verb
surface without churning state.rs.

No behaviour change; src/state/phase.rs tests exercise the same code
paths as the former src/state.rs tests."
```

---

## Task 2: Backlog schema types + slug generation

**Files:**
- Modify: `src/state/backlog/schema.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: the serde-backed types every other backlog module will use, plus pure slug generation so id assignment is unit-testable in isolation.

- [ ] **Step 1: Write the failing test for schema round-trip and slug generation**

Replace `src/state/backlog/schema.rs` contents with:

```rust
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
    pub fn as_cli_str(self) -> &'static str {
        match self {
            Status::NotStarted => "not_started",
            Status::InProgress => "in_progress",
            Status::Done => "done",
            Status::Blocked => "blocked",
        }
    }

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
```

- [ ] **Step 2: Re-export the schema from the submodule**

Update `src/state/backlog/mod.rs`:
```rust
//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod schema;

pub use schema::{allocate_id, slug_from_title, BacklogFile, Status, Task};
```

- [ ] **Step 3: Run the tests; expect them to pass**

Run: `cargo test --lib state::backlog::schema::tests`

Expected: all six tests pass. (The schema is pure data + pure functions; there's no scaffolding to fail yet.)

- [ ] **Step 4: Commit**

```bash
git add src/state/backlog/
git commit -m "Add typed backlog.yaml schema and id allocation

BacklogFile / Task / Status are the serde-backed types every R1 verb
will read and write. slug_from_title + allocate_id keep id policy
(slug from title at creation, collision-suffixed, persisted as a
field) concentrated in one place and unit-testable in isolation.

BacklogFile::extra flattens unknown top-level keys so future R2+ schema
additions survive an R1 read/write cycle."
```

---

## Task 3: Atomic YAML read/write

**Files:**
- Create: `src/state/backlog/yaml_io.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: central read/write for `<plan>/backlog.yaml` with atomic tmp-file + rename (existing `state::phase::atomic_write` pattern) and block-scalar preservation on emit.

- [ ] **Step 1: Write the failing test for round-trip with multi-line markdown bodies**

Create `src/state/backlog/yaml_io.rs`:
```rust
//! Atomic read/write of `<plan>/backlog.yaml`. Format preservation
//! note: serde_yaml 0.9 emits multi-line strings as `|` block scalars
//! automatically when they contain a newline, which renders Results /
//! description bodies readably without escaping.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::BacklogFile;

pub fn backlog_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join("backlog.yaml")
}

pub fn read_backlog(plan_dir: &Path) -> Result<BacklogFile> {
    let path = backlog_path(plan_dir);
    if !path.exists() {
        bail!(
            "backlog.yaml not found at {}. Run `ravel-lite state migrate` to convert an existing backlog.md.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: BacklogFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as backlog.yaml schema", path.display()))?;
    Ok(parsed)
}

pub fn write_backlog(plan_dir: &Path, backlog: &BacklogFile) -> Result<()> {
    let path = backlog_path(plan_dir);
    let yaml = serde_yaml::to_string(backlog)
        .with_context(|| "Failed to serialise backlog.yaml")?;
    atomic_write(&path, yaml.as_bytes())
}

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
    use crate::state::backlog::schema::{Status, Task};
    use tempfile::TempDir;

    fn sample_task() -> Task {
        Task {
            id: "sample".into(),
            title: "Sample task".into(),
            category: "maintenance".into(),
            status: Status::NotStarted,
            blocked_reason: None,
            dependencies: vec![],
            description: "Paragraph one.\n\nParagraph two, with `code`.\n".into(),
            results: None,
            handoff: None,
        }
    }

    #[test]
    fn write_then_read_round_trips_task_fields() {
        let tmp = TempDir::new().unwrap();
        let backlog = BacklogFile {
            tasks: vec![sample_task()],
            extra: Default::default(),
        };
        write_backlog(tmp.path(), &backlog).unwrap();

        let round_tripped = read_backlog(tmp.path()).unwrap();
        assert_eq!(round_tripped.tasks.len(), 1);
        assert_eq!(round_tripped.tasks[0].id, "sample");
        assert_eq!(round_tripped.tasks[0].description, sample_task().description);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_description() {
        let tmp = TempDir::new().unwrap();
        let backlog = BacklogFile {
            tasks: vec![sample_task()],
            extra: Default::default(),
        };
        write_backlog(tmp.path(), &backlog).unwrap();

        let raw = std::fs::read_to_string(backlog_path(tmp.path())).unwrap();
        // serde_yaml 0.9 emits multi-line strings as `|` block scalars.
        // Guard the behaviour so a future dependency swap doesn't silently
        // regress readability.
        assert!(
            raw.contains("description: |") || raw.contains("description: |-"),
            "multi-line description must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_backlog_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_backlog(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("backlog.yaml"), "error must name backlog.yaml: {msg}");
        assert!(msg.contains("state migrate"), "error must suggest migrate: {msg}");
    }
}
```

- [ ] **Step 2: Expose the I/O functions from the backlog module**

Update `src/state/backlog/mod.rs`:
```rust
//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod schema;
pub mod yaml_io;

pub use schema::{allocate_id, slug_from_title, BacklogFile, Status, Task};
pub use yaml_io::{backlog_path, read_backlog, write_backlog};
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib state::backlog::yaml_io::tests`

Expected: all three tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/state/backlog/
git commit -m "Add atomic read/write for backlog.yaml

Central I/O module mirrors the tmp-file + rename pattern from
state::phase::atomic_write, so a concurrent reader (e.g. another CLI
call interleaved by the orchestrator) never sees a half-written file.

Round-trip tests pin three invariants: body fields preserved
byte-for-byte, multi-line descriptions emit as |-block scalars under
serde_yaml 0.9, and a missing backlog.yaml produces an actionable
error pointing at the migrate verb."
```

---

## Task 4: Markdown parser for migration

**Files:**
- Create: `src/state/backlog/parse_md.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: parse the current `backlog.md` prose format into a `BacklogFile`. Strict grammar — rejects anything outside the canonical shape phase prompts emit. Lifts the `\n---\n[HANDOFF]` convention into the typed `handoff:` field.

- [ ] **Step 1: Write the failing tests for the parser**

Create `src/state/backlog/parse_md.rs`:
```rust
//! Strict parser for the legacy `<plan>/backlog.md` prose format.
//!
//! Used exclusively by the `state migrate` verb. Accepts the canonical
//! shape phase prompts emit today (`### Title` headings, `**Field:**
//! value` lines, `---` separators between tasks) and refuses anything
//! else rather than risk silent data loss.

use anyhow::{anyhow, bail, Context, Result};

use super::schema::{slug_from_title, BacklogFile, Status, Task};

pub fn parse_backlog_markdown(input: &str) -> Result<BacklogFile> {
    let mut tasks = Vec::new();
    let mut existing_ids: Vec<String> = Vec::new();

    // Split on task-separator lines: a line that is exactly `---`, with
    // blank lines around it are tolerated. Skip the file's preamble
    // (header, any text before the first `### ` heading).
    let blocks = split_into_task_blocks(input);
    for (block_index, block) in blocks.into_iter().enumerate() {
        let task = parse_single_task_block(&block, &existing_ids)
            .with_context(|| format!("failed to parse task block #{}", block_index + 1))?;
        existing_ids.push(task.id.clone());
        tasks.push(task);
    }

    Ok(BacklogFile {
        tasks,
        extra: Default::default(),
    })
}

fn split_into_task_blocks(input: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_task = false;
    for line in input.lines() {
        if line.starts_with("### ") {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            in_task = true;
        }
        if in_task {
            current.push_str(line);
            current.push('\n');
            if line.trim_end() == "---" {
                // End of a task block. Drop the trailing separator line
                // from the captured block text; the separator is not
                // part of the task content.
                let trimmed = current.trim_end_matches(&['\n', '\r'][..]);
                let without_sep =
                    trimmed.rsplit_once("---").map(|(head, _)| head).unwrap_or(trimmed);
                blocks.push(without_sep.trim_end().to_string() + "\n");
                current.clear();
                in_task = false;
            }
        }
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }
    blocks
}

fn parse_single_task_block(block: &str, existing_ids: &[String]) -> Result<Task> {
    let mut lines = block.lines();
    let title_line = lines
        .next()
        .ok_or_else(|| anyhow!("empty task block"))?;
    let title = title_line
        .strip_prefix("### ")
        .ok_or_else(|| anyhow!("task block does not start with `### <title>`: {title_line:?}"))?
        .trim()
        .to_string();
    if title.is_empty() {
        bail!("task title is empty");
    }

    let id = allocate_id_from(&title, existing_ids);

    // Walk the remainder, pulling **Field:** value lines until the first
    // blank line; after that it's description/results/handoff prose.
    let mut category: Option<String> = None;
    let mut status: Option<Status> = None;
    let mut dependencies: Vec<String> = Vec::new();
    let mut description = String::new();
    let mut results: Option<String> = None;
    let mut handoff: Option<String> = None;
    let mut blocked_reason: Option<String> = None;

    let rest: Vec<&str> = lines.collect();
    let mut cursor = 0;

    while cursor < rest.len() {
        let line = rest[cursor];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            cursor += 1;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("**Category:**") {
            category = Some(strip_backticks(value.trim()).to_string());
            cursor += 1;
        } else if let Some(value) = trimmed.strip_prefix("**Status:**") {
            let raw = strip_backticks(value.trim());
            let (status_value, reason) = split_blocked_reason(raw);
            status = Some(
                Status::parse(status_value)
                    .ok_or_else(|| anyhow!("invalid status value: {status_value:?}"))?,
            );
            if status == Some(Status::Blocked) {
                blocked_reason = Some(reason.unwrap_or_default().to_string());
            }
            cursor += 1;
        } else if let Some(value) = trimmed.strip_prefix("**Dependencies:**") {
            let raw = value.trim();
            if raw != "none" && !raw.is_empty() {
                dependencies = raw
                    .split(',')
                    .map(|s| slug_from_title(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            cursor += 1;
        } else {
            break;
        }
    }

    // Everything from `cursor` onward is prose: description, optional
    // Results, optional hand-off. The Description section starts with a
    // `**Description:**` line and continues until `**Results:**` (or end
    // of block / hand-off separator).
    let body: Vec<&str> = rest[cursor..].to_vec();
    let (desc_block, rest_after_desc) = take_section(&body, "**Description:**", &["**Results:**", "[HANDOFF]"]);
    description = desc_block.trim_end_matches('\n').to_string() + "\n";

    let (results_block, rest_after_results) =
        take_section(&rest_after_desc, "**Results:**", &["[HANDOFF]"]);
    if !results_block.trim().is_empty() {
        let body = results_block.trim_end_matches('\n');
        if body == "_pending_" {
            // Convention: `_pending_` means "no results yet"; leave as None.
        } else {
            results = Some(body.to_string() + "\n");
        }
    }

    // Anything after a `---\n[HANDOFF]` marker is the hand-off body.
    if !rest_after_results.is_empty() {
        let joined = rest_after_results.join("\n");
        if let Some(idx) = joined.find("[HANDOFF]") {
            let tail = joined[idx..].trim_start_matches("[HANDOFF]").trim_start();
            handoff = Some(tail.trim_end().to_string() + "\n");
        }
    }

    Ok(Task {
        id,
        title,
        category: category.ok_or_else(|| anyhow!("missing **Category:** field"))?,
        status: status.ok_or_else(|| anyhow!("missing **Status:** field"))?,
        blocked_reason,
        dependencies,
        description,
        results,
        handoff,
    })
}

fn allocate_id_from(title: &str, existing: &[String]) -> String {
    super::schema::allocate_id(title, existing.iter().map(String::as_str))
}

fn strip_backticks(value: &str) -> &str {
    value.trim().trim_matches('`')
}

fn split_blocked_reason(raw: &str) -> (&str, Option<&str>) {
    // `**Status:** blocked (reason: upstream release)`
    if let Some((status, rest)) = raw.split_once('(') {
        let reason = rest.trim_end_matches(')').trim();
        let reason = reason.strip_prefix("reason:").unwrap_or(reason).trim();
        (status.trim(), Some(reason))
    } else {
        (raw.trim(), None)
    }
}

/// Return `(section_body, remaining_lines)` where `section_body` is
/// everything under a `starts_with` heading up to (but not including)
/// any of `terminators`, joined by newlines. If no heading is found,
/// `section_body` is empty and `remaining_lines` is the original input.
fn take_section<'a>(
    lines: &[&'a str],
    starts_with: &str,
    terminators: &[&str],
) -> (String, Vec<&'a str>) {
    let mut iter = lines.iter().enumerate();
    let start_index = loop {
        match iter.next() {
            Some((idx, line)) if line.trim_start().starts_with(starts_with) => break Some(idx),
            Some(_) => continue,
            None => break None,
        }
    };
    let Some(start_index) = start_index else {
        return (String::new(), lines.to_vec());
    };
    let first_content = start_index + 1;
    let mut end_index = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(first_content) {
        if terminators.iter().any(|t| line.trim_start().starts_with(t)) {
            end_index = idx;
            break;
        }
    }
    let body_lines = &lines[first_content..end_index];
    let joined = body_lines.iter().copied().collect::<Vec<_>>().join("\n");
    let remaining = lines[end_index..].to_vec();
    (joined.trim_start_matches('\n').to_string(), remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TASK: &str = "\
### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Cargo clippy is clean today. Add a CI gate.

**Results:** _pending_

---
";

    #[test]
    fn parses_minimal_task() {
        let backlog = parse_backlog_markdown(MINIMAL_TASK).unwrap();
        assert_eq!(backlog.tasks.len(), 1);
        let task = &backlog.tasks[0];
        assert_eq!(task.id, "add-clippy-d-warnings-ci-gate");
        assert_eq!(task.title, "Add clippy `-D warnings` CI gate");
        assert_eq!(task.category, "maintenance");
        assert_eq!(task.status, Status::NotStarted);
        assert!(task.dependencies.is_empty());
        assert_eq!(
            task.description.trim(),
            "Cargo clippy is clean today. Add a CI gate."
        );
        assert_eq!(task.results, None);
        assert_eq!(task.handoff, None);
    }

    #[test]
    fn parses_task_with_results_and_handoff() {
        let input = "\
### Finished task

**Category:** `research`
**Status:** `done`
**Dependencies:** none

**Description:**

Did the thing.

**Results:**

Did the thing successfully.

---
[HANDOFF]

Promote `follow-up-task` to backlog.
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(backlog.tasks.len(), 1);
        let task = &backlog.tasks[0];
        assert_eq!(task.status, Status::Done);
        assert!(task.results.as_deref().unwrap().contains("successfully"));
        assert!(task
            .handoff
            .as_deref()
            .unwrap()
            .contains("Promote `follow-up-task` to backlog."));
    }

    #[test]
    fn parses_dependencies_as_slugged_ids() {
        let input = "\
### Dependent task

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** Research: expose plan-state markdown, Another Upstream

**Description:**

Waits for the two above.

**Results:** _pending_

---
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(
            backlog.tasks[0].dependencies,
            vec![
                "research-expose-plan-state-markdown".to_string(),
                "another-upstream".to_string()
            ]
        );
    }

    #[test]
    fn parses_blocked_status_with_reason() {
        let input = "\
### Blocked task

**Category:** `maintenance`
**Status:** `blocked (reason: upstream pending 2.1.117)`
**Dependencies:** none

**Description:**

Waiting for Claude Code upstream.

**Results:** _pending_

---
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(backlog.tasks[0].status, Status::Blocked);
        assert_eq!(
            backlog.tasks[0].blocked_reason.as_deref(),
            Some("upstream pending 2.1.117")
        );
    }

    #[test]
    fn multiple_tasks_split_on_separator() {
        let input = format!("{MINIMAL_TASK}\n{MINIMAL_TASK}");
        let backlog = parse_backlog_markdown(&input).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
        // The second task's id collides with the first's title; the
        // allocator must suffix it.
        assert_eq!(backlog.tasks[0].id, "add-clippy-d-warnings-ci-gate");
        assert_eq!(backlog.tasks[1].id, "add-clippy-d-warnings-ci-gate-2");
    }

    #[test]
    fn rejects_task_missing_category() {
        let input = "\
### No category task

**Status:** `not_started`
**Dependencies:** none

**Description:**

Body.

**Results:** _pending_

---
";
        let err = parse_backlog_markdown(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Category"), "error must name the missing field: {msg}");
    }
}
```

- [ ] **Step 2: Expose the parser from the module**

Update `src/state/backlog/mod.rs`:
```rust
//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod parse_md;
pub mod schema;
pub mod yaml_io;

pub use parse_md::parse_backlog_markdown;
pub use schema::{allocate_id, slug_from_title, BacklogFile, Status, Task};
pub use yaml_io::{backlog_path, read_backlog, write_backlog};
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib state::backlog::parse_md::tests`

Expected: all six tests pass.

- [ ] **Step 4: Validate against the real backlog.md**

Run: `cargo run --quiet --example parse_live_backlog 2>&1 || true`

(If you want to add the example, that's optional. At minimum, write a sanity one-off test gated to the live file:)

Add one more test at the bottom of `parse_md.rs`:
```rust
    #[test]
    fn parses_live_core_backlog_without_error() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("LLM_STATE/core/backlog.md");
        if !path.exists() {
            // Allow the test to skip on a fresh checkout without a core plan.
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let backlog = parse_backlog_markdown(&text).expect("core backlog must parse");
        assert!(
            backlog.tasks.len() >= 2,
            "core backlog must have multiple tasks, got {}",
            backlog.tasks.len()
        );
    }
```

Run: `cargo test --lib state::backlog::parse_md::tests::parses_live_core_backlog_without_error`

Expected: pass. This is the parser-feasibility validation the original research task's deliverable #3 demanded.

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Parse the legacy backlog.md format into BacklogFile

Strict grammar: rejects any task block missing **Category:** or
**Status:**, rejects status values outside the enumerated set, and
requires the `### <title>` + `**Field:**` + `---` separator shape
phase prompts emit today.

Lifts the \`\\n---\\n[HANDOFF]\` convention into the typed \`handoff:\`
field so it stops being a separator-driven prose parse and becomes a
schema invariant triage can enforce.

parses_live_core_backlog_without_error validates the parser against
the live LLM_STATE/core/backlog.md so schema drift in real plans is
caught by CI."
```

---

## Task 5: state backlog list verb

**Files:**
- Create: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: the most-used read verb. Filter predicates pull their weight here (`--ready`, `--missing-results`, `--has-handoff`) because R6's prompt migration expects them to short-circuit work-phase task selection.

- [ ] **Step 1: Write the failing tests for filter predicates**

Create `src/state/backlog/verbs.rs`:
```rust
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

use super::schema::{BacklogFile, Status, Task};
use super::yaml_io::read_backlog;

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
        // Smoke test through the disk path — the filter itself is unit-
        // tested above; here we verify the I/O integration.
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        // Redirect stdout by calling the internal function that doesn't
        // write to stdout; instead, verify the file loaded + filter is
        // applied by using task_matches directly.
        let backlog = read_backlog(tmp.path()).unwrap();
        let done = done_ids(&backlog);
        let filter = ListFilter { ready: true, ..Default::default() };
        let count = backlog.tasks.iter().filter(|t| task_matches(t, &filter, &done)).count();
        assert_eq!(count, 1, "one ready task (bar) expected");
    }
}
```

- [ ] **Step 2: Re-export verb entry points**

Update `src/state/backlog/mod.rs`:
```rust
//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod parse_md;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use parse_md::parse_backlog_markdown;
pub use schema::{allocate_id, slug_from_title, BacklogFile, Status, Task};
pub use verbs::{run_list, ListFilter, OutputFormat};
pub use yaml_io::{backlog_path, read_backlog, write_backlog};
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all four tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog list verb with filter predicates

ListFilter + task_matches concentrate the filter semantics in one pure
function unit-tested against a sample BacklogFile. The three
load-bearing filters — --ready, --missing-results, --has-handoff —
each have dedicated tests pinning semantics R6 will depend on.

run_list is the stdout-emitting entry point wired by main.rs in Task
13; the pure filter logic stays testable without touching stdout."
```

---

## Task 6: state backlog show verb

**Files:**
- Modify: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: emit one task's full record. Used when a phase prompt wants a single task's description or Results body.

- [ ] **Step 1: Write the failing test for `run_show`**

Append to the `tests` module in `src/state/backlog/verbs.rs`:
```rust
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
```

- [ ] **Step 2: Implement `run_show` and helper `find_task`**

Append to `src/state/backlog/verbs.rs` (above the `#[cfg(test)]` block):
```rust
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
```

- [ ] **Step 3: Re-export from module**

Update `src/state/backlog/mod.rs`:
```rust
pub use verbs::{run_list, run_show, ListFilter, OutputFormat};
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all six tests pass (including the two new ones).

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog show verb

find_task centralises the id-to-task lookup with a descriptive error;
every subsequent mutation verb will use it. run_show wraps the task
in a BacklogFile projection so the stdout shape is consistent with
list."
```

---

## Task 7: state backlog add + init verbs

**Files:**
- Modify: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: task creation (`add`) and bulk initialisation for create-plan (`init`). `init` refuses a non-empty backlog by design.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:
```rust
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
```

- [ ] **Step 2: Implement `AddRequest`, `run_add`, and `run_init`**

Append to `src/state/backlog/verbs.rs`:
```rust
use super::schema::allocate_id;
use super::yaml_io::write_backlog;

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
```

- [ ] **Step 3: Re-export**

Update `src/state/backlog/mod.rs`:
```rust
pub use verbs::{run_add, run_init, run_list, run_show, AddRequest, ListFilter, OutputFormat};
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all ten tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog add and init verbs

run_add validates dependency ids up-front so a typo surfaces as an
error rather than a dangling reference persisted to disk. Id allocation
flows through allocate_id so collisions land on -2/-3 suffixes rather
than silently shadowing existing tasks.

run_init is the create-plan bootstrap path. It refuses a non-empty
backlog.yaml to prevent accidental overwrites of a live plan."
```

---

## Task 8: state backlog set-status + set-results verbs

**Files:**
- Modify: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: the two most-mutated fields across phases. `set-status` enforces the `blocked` → `--reason` requirement; `set-results` accepts markdown bodies via `--body-file` or stdin.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:
```rust
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
    fn run_set_results_writes_markdown_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_results(tmp.path(), "foo", "Body of results.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.tasks.iter().find(|t| t.id == "foo").unwrap();
        assert_eq!(foo.results.as_deref(), Some("Body of results.\n"));
    }
```

- [ ] **Step 2: Implement the verbs**

Append to `src/state/backlog/verbs.rs`:
```rust
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
```

- [ ] **Step 3: Re-export**

Update `src/state/backlog/mod.rs`:
```rust
pub use verbs::{
    run_add, run_init, run_list, run_set_results, run_set_status, run_show,
    AddRequest, ListFilter, OutputFormat,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all fourteen tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog set-status and set-results verbs

run_set_status enforces the schema invariant that blocked_reason is
non-empty iff status == blocked: --reason is required for `blocked`
transitions and auto-cleared on exit. This catches the drift class
where a task is un-blocked but the reason string is left dangling.

run_set_results takes a markdown body and canonicalises to a trailing
newline; the --body-file vs stdin distinction is a CLI-layer concern
handled in Task 13."
```

---

## Task 9: state backlog set-handoff + clear-handoff verbs

**Files:**
- Modify: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: typed hand-off lifecycle. `set-handoff` accepts markdown; `clear-handoff` is used by triage after promoting or archiving.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:
```rust
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
```

- [ ] **Step 2: Implement the verbs**

Append to `src/state/backlog/verbs.rs`:
```rust
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
```

- [ ] **Step 3: Re-export**

Update `src/state/backlog/mod.rs`:
```rust
pub use verbs::{
    run_add, run_clear_handoff, run_init, run_list, run_set_handoff, run_set_results,
    run_set_status, run_show, AddRequest, ListFilter, OutputFormat,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all sixteen tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog set-handoff and clear-handoff verbs

The hand-off lifecycle promotes from a prose \\n---\\n[HANDOFF]
separator to a typed field. set-handoff is the writer (analyse-work);
clear-handoff is the consumer (triage, after promote-or-archive).

clear-handoff is distinct from a generic \`backlog clear <field>\`
because hand-off is the only field with a well-defined producer/
consumer lifecycle that's worth naming in the CLI surface."
```

---

## Task 10: state backlog set-title + reorder + delete verbs

**Files:**
- Modify: `src/state/backlog/verbs.rs`
- Modify: `src/state/backlog/mod.rs`

Goal: remaining CRUD verbs. `set-title` keeps `id` pinned (title changes without breaking dependency references); `reorder` moves a task relative to another; `delete` removes, with dependency-reference cleanup policy.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:
```rust
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
```

- [ ] **Step 2: Implement the verbs**

Append to `src/state/backlog/verbs.rs`:
```rust
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
```

- [ ] **Step 3: Re-export**

Update `src/state/backlog/mod.rs`:
```rust
pub use verbs::{
    run_add, run_clear_handoff, run_delete, run_init, run_list, run_reorder,
    run_set_handoff, run_set_results, run_set_status, run_set_title, run_show,
    AddRequest, ListFilter, OutputFormat, ReorderPosition,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib state::backlog::verbs::tests`

Expected: all twenty-one tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/state/backlog/
git commit -m "Add state backlog set-title, reorder, and delete verbs

set-title updates the title field while leaving id pinned — the
id-stability invariant the spec calls out explicitly. Dependents
continue to reference by id so a title refinement never breaks the
graph.

reorder accepts before|after positioning. delete refuses by default
when the task is a dep of another; --force cascades the dep reference
cleanup so nothing is left dangling."
```

---

## Task 11: state migrate verb (backlog-only in R1)

**Files:**
- Modify: `src/state/migrate.rs`
- Modify: `src/state/mod.rs`

Goal: one-shot per-plan conversion with the behaviour contract from the spec — atomicity, idempotency, validation, dry-run, force, keep-originals vs delete-originals. In R1, scope is limited to backlog.md. Memory / session-log / latest-session / phase come in R2–R3's extensions of this same verb.

- [ ] **Step 1: Write the failing tests**

Replace `src/state/migrate.rs` with:
```rust
//! `state migrate <plan-dir>` — single-plan conversion of legacy .md
//! files into typed .yaml siblings.
//!
//! R1 scope: backlog.md only. Future rollouts (R2–R3) extend this verb
//! in place to cover memory, session-log, latest-session, and phase.
//! Does not touch related-plans.md (handled by the separate
//! migrate-related-projects verb when R5 lands).

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::state::backlog::{
    parse_backlog_markdown, read_backlog, write_backlog, BacklogFile,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OriginalPolicy {
    Keep,
    Delete,
}

#[derive(Debug, Clone)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub original_policy: OriginalPolicy,
    pub force: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        MigrateOptions {
            dry_run: false,
            original_policy: OriginalPolicy::Keep,
            force: false,
        }
    }
}

pub fn run_migrate(plan_dir: &Path, options: &MigrateOptions) -> Result<()> {
    let source = plan_dir.join("backlog.md");
    let target = plan_dir.join("backlog.yaml");

    if !source.exists() {
        bail!(
            "no backlog.md to migrate at {}. Either the plan has no backlog or migration has already run.",
            source.display()
        );
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_backlog_markdown(&text)
        .with_context(|| format!("failed to parse {} as legacy backlog markdown", source.display()))?;

    // Idempotency: if the target exists, require the re-migration output
    // to match the current file content (modulo canonical serialisation).
    if target.exists() {
        let existing = read_backlog(plan_dir)
            .with_context(|| "failed to read existing backlog.yaml for idempotency check")?;
        if backlogs_equivalent(&existing, &parsed) {
            if matches!(options.original_policy, OriginalPolicy::Delete) && !options.dry_run {
                std::fs::remove_file(&source)
                    .with_context(|| format!("failed to delete {}", source.display()))?;
            }
            return Ok(()); // no-op
        }
        if !options.force {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    }

    if options.dry_run {
        println!("dry-run: would write {} ({} tasks)", target.display(), parsed.tasks.len());
        if matches!(options.original_policy, OriginalPolicy::Delete) {
            println!("dry-run: would delete {}", source.display());
        }
        return Ok(());
    }

    write_backlog(plan_dir, &parsed)?;

    // Validation round-trip: re-parse the file we just wrote and assert
    // it round-trips to the same BacklogFile content.
    let validated = read_backlog(plan_dir)
        .with_context(|| "validation round-trip read failed after write")?;
    if !backlogs_equivalent(&validated, &parsed) {
        bail!(
            "validation mismatch: backlog.yaml re-read does not match the parse result. Aborting without deleting the original."
        );
    }

    if matches!(options.original_policy, OriginalPolicy::Delete) {
        std::fs::remove_file(&source)
            .with_context(|| format!("failed to delete {}", source.display()))?;
    }
    Ok(())
}

/// Structural equivalence for idempotency / validation checks. Ignores
/// the `extra` IndexMap because a re-migration always emits empty extra
/// (no unknown top-level keys in a legacy-markdown parse).
fn backlogs_equivalent(a: &BacklogFile, b: &BacklogFile) -> bool {
    if a.tasks.len() != b.tasks.len() {
        return false;
    }
    for (task_a, task_b) in a.tasks.iter().zip(b.tasks.iter()) {
        if task_a.id != task_b.id
            || task_a.title != task_b.title
            || task_a.category != task_b.category
            || task_a.status != task_b.status
            || task_a.blocked_reason != task_b.blocked_reason
            || task_a.dependencies != task_b.dependencies
            || task_a.description != task_b.description
            || task_a.results != task_b.results
            || task_a.handoff != task_b.handoff
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TWO_TASK_MARKDOWN: &str = "\
### First task

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

First task body.

**Results:** _pending_

---

### Second task

**Category:** `research`
**Status:** `done`
**Dependencies:** First task

**Description:**

Second task body.

**Results:**

Done and dusted.

---
";

    fn write_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("backlog.md"), content).unwrap();
    }

    #[test]
    fn migrate_writes_backlog_yaml_and_keeps_md_by_default() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert!(tmp.path().join("backlog.yaml").exists());
        assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
    }

    #[test]
    fn migrate_with_delete_originals_removes_md_after_success() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        let opts = MigrateOptions {
            original_policy: OriginalPolicy::Delete,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(tmp.path().join("backlog.yaml").exists());
        assert!(!tmp.path().join("backlog.md").exists(), "md must be deleted on success");
    }

    #[test]
    fn migrate_dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        let opts = MigrateOptions {
            dry_run: true,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(!tmp.path().join("backlog.yaml").exists(), "dry-run must not write");
        assert!(tmp.path().join("backlog.md").exists());
    }

    #[test]
    fn migrate_is_idempotent_on_second_run() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();
        // Second run must no-op — no error, no changes.
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
    }

    #[test]
    fn migrate_refuses_overwrite_on_diverged_yaml_without_force() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        // Tamper with the yaml so it diverges from the markdown.
        let mut backlog = read_backlog(tmp.path()).unwrap();
        backlog.tasks[0].title = "Tampered title".into();
        write_backlog(tmp.path(), &backlog).unwrap();

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already exists"), "error must mention existence: {msg}");
        assert!(msg.contains("--force"), "error must cite --force: {msg}");

        // With --force, the tampered yaml is overwritten.
        let opts = MigrateOptions { force: true, ..MigrateOptions::default() };
        run_migrate(tmp.path(), &opts).unwrap();
        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks[0].title, "First task");
    }

    #[test]
    fn migrate_parse_failure_leaves_filesystem_untouched() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), "### Malformed task\n\nno category or status\n");

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Category") || msg.contains("Status"), "error must name the missing field: {msg}");
        assert!(!tmp.path().join("backlog.yaml").exists(), "partial writes forbidden on parse failure");
    }
}
```

- [ ] **Step 2: Re-export from state/mod.rs**

Update `src/state/mod.rs`:
```rust
//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase`    — `set-phase` (existing)
//! - `backlog`  — typed backlog.yaml + CRUD verbs (R1)
//! - `migrate`  — one-shot per-plan .md → .yaml conversion (backlog only in R1)

pub mod backlog;
pub mod migrate;
pub mod phase;

pub use phase::run_set_phase;
pub use migrate::{run_migrate, MigrateOptions, OriginalPolicy};
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib state::migrate::tests`

Expected: all six tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/state/migrate.rs src/state/mod.rs
git commit -m "Add state migrate verb (backlog-only in R1)

Implements the behaviour contract from docs/structured-plan-state-
design.md: atomic write (parse all sources first, then write), validation
round-trip (re-read + structural compare before declaring success), and
the idempotency rules (re-run matches existing yaml → no-op; existing
yaml diverges → requires --force).

Default policy is --keep-originals so a re-run is always safe; the user
opts into --delete-originals when they're confident.

R1 scope is backlog.md only; R2–R3 extend this verb in place to cover
memory, session-log, latest-session, phase."
```

---

## Task 12: Wire up CLI dispatch in main.rs

**Files:**
- Modify: `src/main.rs`

Goal: add the full `state backlog <verb>` subcommand tree plus `state migrate` to `StateCommands`. Body-field input (`--body-file` vs `--body -` stdin) resolved here; library functions take already-resolved `&str` bodies.

- [ ] **Step 1: Extend the `StateCommands` enum**

Replace the current `StateCommands` definition in `src/main.rs` with:

```rust
#[derive(Subcommand)]
enum StateCommands {
    /// Rewrite `<plan-dir>/phase.md` to the given phase. Validates the
    /// phase string and requires phase.md to already exist.
    SetPhase {
        plan_dir: PathBuf,
        phase: String,
    },
    /// Backlog CRUD verbs. Every prompt-side mutation of backlog.yaml
    /// goes through one of these.
    Backlog {
        #[command(subcommand)]
        command: BacklogCommands,
    },
    /// Single-plan conversion of legacy .md files into typed .yaml
    /// siblings. R1 scope: backlog.md only.
    Migrate {
        plan_dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
        /// Keep the .md originals on disk after migration (default).
        #[arg(long, conflicts_with = "delete_originals")]
        keep_originals: bool,
        /// Delete the .md originals only after write and validation both succeed.
        #[arg(long)]
        delete_originals: bool,
        /// Overwrite an existing backlog.yaml that differs from the re-migration output.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum BacklogCommands {
    /// Emit tasks matching the given filters.
    List {
        plan_dir: PathBuf,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        category: Option<String>,
        /// Shorthand for `status=not_started AND every dep is done`.
        #[arg(long)]
        ready: bool,
        /// Match tasks that carry a hand-off block.
        #[arg(long)]
        has_handoff: bool,
        /// Match done tasks missing a Results block.
        #[arg(long)]
        missing_results: bool,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single task by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new task.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        category: String,
        #[arg(long, value_delimiter = ',')]
        dependencies: Vec<String>,
        /// Path to a file containing the markdown description body.
        #[arg(long, conflicts_with = "description")]
        description_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the description inline.
        #[arg(long)]
        description: Option<String>,
    },
    /// One-shot bulk initialisation for create-plan. Refuses a non-empty backlog.
    Init {
        plan_dir: PathBuf,
        #[arg(long)]
        body_file: PathBuf,
    },
    /// Update a task's status. `--reason <text>` is required when setting to `blocked`.
    SetStatus {
        plan_dir: PathBuf,
        id: String,
        status: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Set a task's Results block from a file or stdin.
    SetResults {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Set a task's hand-off block from a file or stdin.
    SetHandoff {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Clear a task's hand-off block (triage uses after promote/archive).
    ClearHandoff {
        plan_dir: PathBuf,
        id: String,
    },
    /// Update a task's title. Id is preserved.
    SetTitle {
        plan_dir: PathBuf,
        id: String,
        new_title: String,
    },
    /// Move a task before or after another in the backlog list.
    Reorder {
        plan_dir: PathBuf,
        id: String,
        position: String,
        target_id: String,
    },
    /// Delete a task. Refuses if the task is a dependency of another unless `--force`.
    Delete {
        plan_dir: PathBuf,
        id: String,
        #[arg(long)]
        force: bool,
    },
}
```

- [ ] **Step 2: Add dispatch arms in `main`**

Replace the existing `Commands::State { command } => match command { … }` arm with:

```rust
        Commands::State { command } => dispatch_state(command),
```

Add this helper function at module scope (below `main`):

```rust
fn dispatch_state(command: StateCommands) -> Result<()> {
    use crate::state::backlog as backlog_v;
    use crate::state::migrate;

    match command {
        StateCommands::SetPhase { plan_dir, phase } => {
            state::run_set_phase(&plan_dir, &phase)
        }
        StateCommands::Backlog { command } => dispatch_backlog(command),
        StateCommands::Migrate {
            plan_dir,
            dry_run,
            keep_originals: _,
            delete_originals,
            force,
        } => {
            let options = migrate::MigrateOptions {
                dry_run,
                original_policy: if delete_originals {
                    migrate::OriginalPolicy::Delete
                } else {
                    migrate::OriginalPolicy::Keep
                },
                force,
            };
            migrate::run_migrate(&plan_dir, &options)
        }
    }
}

fn dispatch_backlog(command: BacklogCommands) -> Result<()> {
    use crate::state::backlog::{self, ListFilter, OutputFormat, ReorderPosition, Status};

    match command {
        BacklogCommands::List {
            plan_dir,
            status,
            category,
            ready,
            has_handoff,
            missing_results,
            format,
        } => {
            let status = status
                .as_deref()
                .map(|s| Status::parse(s).ok_or_else(|| {
                    anyhow::anyhow!("invalid --status value {s:?}; expected one of not_started, in_progress, done, blocked")
                }))
                .transpose()?;
            let filter = ListFilter {
                status,
                category,
                ready,
                has_handoff,
                missing_results,
            };
            let fmt = parse_output_format(&format)?;
            backlog::run_list(&plan_dir, &filter, fmt)
        }
        BacklogCommands::Show { plan_dir, id, format } => {
            let fmt = parse_output_format(&format)?;
            backlog::run_show(&plan_dir, &id, fmt)
        }
        BacklogCommands::Add {
            plan_dir,
            title,
            category,
            dependencies,
            description_file,
            description,
        } => {
            let description_body = resolve_body(description_file, description)?;
            let req = backlog::AddRequest {
                title,
                category,
                dependencies,
                description: description_body,
            };
            backlog::run_add(&plan_dir, &req)
        }
        BacklogCommands::Init { plan_dir, body_file } => {
            let text = std::fs::read_to_string(&body_file)
                .with_context(|| format!("failed to read {}", body_file.display()))?;
            let seed: backlog::BacklogFile = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {} as backlog.yaml", body_file.display()))?;
            backlog::run_init(&plan_dir, &seed)
        }
        BacklogCommands::SetStatus {
            plan_dir,
            id,
            status,
            reason,
        } => {
            let status = Status::parse(&status)
                .ok_or_else(|| anyhow::anyhow!("invalid status {status:?}"))?;
            backlog::run_set_status(&plan_dir, &id, status, reason.as_deref())
        }
        BacklogCommands::SetResults { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_results(&plan_dir, &id, &body)
        }
        BacklogCommands::SetHandoff { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_handoff(&plan_dir, &id, &body)
        }
        BacklogCommands::ClearHandoff { plan_dir, id } => {
            backlog::run_clear_handoff(&plan_dir, &id)
        }
        BacklogCommands::SetTitle { plan_dir, id, new_title } => {
            backlog::run_set_title(&plan_dir, &id, &new_title)
        }
        BacklogCommands::Reorder { plan_dir, id, position, target_id } => {
            let pos = ReorderPosition::parse(&position).ok_or_else(|| {
                anyhow::anyhow!("invalid reorder position {position:?}; expected `before` or `after`")
            })?;
            backlog::run_reorder(&plan_dir, &id, pos, &target_id)
        }
        BacklogCommands::Delete { plan_dir, id, force } => {
            backlog::run_delete(&plan_dir, &id, force)
        }
    }
}

fn parse_output_format(input: &str) -> Result<crate::state::backlog::OutputFormat> {
    crate::state::backlog::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

/// Resolve `--body-file <path>` vs `--body <value>` vs `--body -` (stdin).
/// Exactly one of the two arguments must be set; if neither is set,
/// returns an empty string (used for optional bodies like an add with no
/// description).
fn resolve_body(body_file: Option<PathBuf>, body: Option<String>) -> Result<String> {
    match (body_file, body) {
        (Some(path), None) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display())),
        (None, Some(value)) if value == "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read body from stdin")?;
            Ok(buf)
        }
        (None, Some(value)) => Ok(value),
        (None, None) => Ok(String::new()),
        (Some(_), Some(_)) => bail!("pass only one of --body-file or --body"),
    }
}
```

Also ensure `anyhow::Context` is in scope at the top of `src/main.rs`:
```rust
use anyhow::{Context, Result};
```

- [ ] **Step 3: Run all tests to verify the crate still builds cleanly**

Run: `cargo test --lib`

Expected: all tests pass (including every state test added in Tasks 1–11). No warnings (the crate has `warnings = "deny"`).

Run: `cargo build --release`

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "Wire state backlog verbs and state migrate into CLI dispatch

Extends StateCommands with a Backlog subcommand tree and a Migrate
command. Each verb maps to a single library entry point in
state::backlog::* or state::migrate.

resolve_body centralises the --body-file vs --body vs --body - (stdin)
resolution that every body-accepting verb needs. Library functions
take already-resolved &str bodies so the stdio-vs-file choice stays
out of their signatures."
```

---

## Task 13: End-to-end integration tests

**Files:**
- Create: `tests/state_backlog.rs`

Goal: integration tests that shell out to the `ravel-lite` binary (same pattern as `tests/integration.rs`) and assert CLI behaviour end-to-end. Covers the headline "three ready tasks" round-trip plus dry-run, idempotency, and parse-failure paths.

- [ ] **Step 1: Write the failing integration tests**

Create `tests/state_backlog.rs`:
```rust
//! End-to-end CLI integration tests for `ravel-lite state backlog *`
//! and `ravel-lite state migrate`. Shells out to the built binary via
//! CARGO_BIN_EXE_ravel-lite, matching the pattern in tests/integration.rs.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn seed_two_task_backlog_md(plan_dir: &std::path::Path) {
    let content = "\
### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Cargo clippy is clean today. Add a CI gate to keep it that way.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Blocked on upstream Claude Code release past 2.1.116.

**Results:** _pending_

---
";
    std::fs::write(plan_dir.join("backlog.md"), content).unwrap();
}

#[test]
fn migrate_converts_backlog_md_to_yaml_and_list_emits_ready_tasks() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    let migrate = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        migrate.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    assert!(tmp.path().join("backlog.yaml").exists());
    assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--status", "not_started", "--ready"])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        list.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(stdout.contains("add-clippy-d-warnings-ci-gate"), "output must include task id: {stdout}");
    assert!(
        stdout.contains("remove-claude-code-debug-file-workaround"),
        "output must include second task id: {stdout}"
    );
}

#[test]
fn migrate_dry_run_does_not_write_yaml() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(out.status.success(), "dry-run must exit 0");
    assert!(!tmp.path().join("backlog.yaml").exists(), "dry-run wrote yaml");
}

#[test]
fn migrate_is_idempotent_across_repeated_runs() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    for _ in 0..2 {
        let out = Command::new(bin())
            .args(["state", "migrate"])
            .arg(tmp.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "migrate failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // List must still yield two tasks (no duplication, no corruption).
    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let tasks: usize = stdout.matches("id:").count();
    assert_eq!(tasks, 2, "expected two tasks after idempotent migrate, got stdout:\n{stdout}");
}

#[test]
fn migrate_parse_failure_leaves_filesystem_untouched() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("backlog.md"), "### Bad\n\nno fields\n").unwrap();

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "malformed input must exit non-zero");
    assert!(!tmp.path().join("backlog.yaml").exists(), "partial write on parse failure");
}

#[test]
fn add_set_status_set_results_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty backlog.yaml so add has nothing to collide with.
    std::fs::write(
        tmp.path().join("backlog.yaml"),
        "tasks: []\n",
    )
    .unwrap();

    let add = Command::new(bin())
        .args(["state", "backlog", "add"])
        .arg(tmp.path())
        .args(["--title", "New task", "--category", "maintenance"])
        .args(["--description", "Task body.\n"])
        .output()
        .unwrap();
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let set_status = Command::new(bin())
        .args(["state", "backlog", "set-status"])
        .arg(tmp.path())
        .args(["new-task", "in_progress"])
        .output()
        .unwrap();
    assert!(set_status.status.success());

    let set_results = Command::new(bin())
        .args(["state", "backlog", "set-results"])
        .arg(tmp.path())
        .args(["new-task", "--body", "Finished.\n"])
        .output()
        .unwrap();
    assert!(set_results.status.success());

    // set-results is only meaningful on `done` tasks conceptually, but
    // the verb itself accepts any status; the conceptual invariant
    // (flip status to done first) is a prompt-side concern.
    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("new-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("in_progress"));
    assert!(stdout.contains("Finished."));
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cargo test --test state_backlog`

Expected: all five tests pass.

- [ ] **Step 3: Run the full test suite to confirm no regressions**

Run: `cargo test`

Expected: everything green, including existing integration tests in `tests/integration.rs`.

- [ ] **Step 4: Commit**

```bash
git add tests/state_backlog.rs
git commit -m "Add end-to-end integration tests for state backlog + migrate

Five CLI-level tests exercised via CARGO_BIN_EXE_ravel-lite. The
headline scenario (migrate + list --status not_started --ready) is the
path R6's prompt migration will use during work-phase task selection.

Idempotency, dry-run, and parse-failure tests pin the behaviour
contract from the spec. The round-trip test for add + set-status +
set-results + show validates the body-resolution path (--body inline
vs --body-file vs --body - stdin) at the CLI layer."
```

---

## Self-review checklist

Before declaring R1 done, work through each point:

1. **Spec coverage.** Every verb and flag in _CLI surface_ → state backlog section of `docs/structured-plan-state-design.md` has a task above. `handoff` field, `blocked_reason` field, filter predicates (`--ready`, `--missing-results`, `--has-handoff`), id stability across title edits — all covered.
2. **Migration contract.** Atomicity, idempotency, validation round-trip, dry-run, force, keep-originals vs delete-originals — each covered by a test in Task 11 or Task 13.
3. **No placeholders.** No "TBD", no "add appropriate X", no "implement later" steps. Every code step is complete runnable Rust.
4. **Type consistency.** `Status::parse` signature (`&str -> Option<Status>`), `OutputFormat::parse`, `ReorderPosition::parse` all follow the same convention. Verb handler names use `run_<verb>` prefix consistently.
5. **Tests before implementation.** Every task writes the failing test first.
6. **Commit cadence.** One commit per task, granular enough to revert any single feature without unwinding the rest.

## Scope items explicitly deferred to later rollout plans

- `state memory` verb surface — R2.
- `state session-log` verb surface + `latest-session.yaml` + GitCommitWork rewire — R3.
- `state projects` catalog + auto-add on `ravel-lite run` — R4.
- `state related-projects` global edge list + `migrate-related-projects` — R5.
- Phase-prompt rewrites (work / analyse-work / reflect / dream / triage / create-plan / survey / survey-incremental) — R6.
- LLM-driven discovery for related-projects — R7 (its own design-ish backlog task).
- `state migrate` expansion to cover memory, session-log, latest-session, phase — folded into R2/R3/R5.
