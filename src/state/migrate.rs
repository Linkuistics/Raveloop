//! `state migrate <plan-dir>` — single-plan conversion of legacy .md
//! files into typed .yaml siblings.
//!
//! Each supported file has its own atomic migration pathway (backlog,
//! memory). The top-level verb runs every migrator that has a source
//! file, parses all source files first, then writes all targets second:
//! a parse failure on any file aborts before any write touches disk.
//! Does not touch related-plans.md — that legacy markdown migrator
//! was retired with the move to component-ontology v2 (the
//! component-relationship graph at `<config-dir>/related-components.yaml`
//! is now populated exclusively by the discover pipeline).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::state::backlog::{
    parse_backlog_markdown, read_backlog, write_backlog, BacklogFile,
};
use crate::state::filenames::{
    BACKLOG_FILENAME, LATEST_SESSION_FILENAME, MEMORY_FILENAME, SESSION_LOG_FILENAME,
};
use crate::state::memory::{
    parse_memory_markdown, read_memory, write_memory, MemoryFile,
};
use crate::state::session_log::{
    parse_latest_session_markdown, parse_session_log_markdown, read_latest_session,
    read_session_log, write_latest_session, write_session_log, SessionLogFile, SessionRecord,
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

/// One parsed-but-not-yet-written migration candidate. Holds the source
/// and target paths, the parsed-in-memory value, and whether writing is
/// actually required (skipped when target already matches).
enum PendingMigration {
    Backlog {
        source: PathBuf,
        target: PathBuf,
        parsed: BacklogFile,
        needs_write: bool,
    },
    Memory {
        source: PathBuf,
        target: PathBuf,
        parsed: MemoryFile,
        needs_write: bool,
    },
    SessionLog {
        source: PathBuf,
        target: PathBuf,
        parsed: SessionLogFile,
        needs_write: bool,
    },
    LatestSession {
        source: PathBuf,
        target: PathBuf,
        parsed: SessionRecord,
        needs_write: bool,
    },
}

impl PendingMigration {
    fn source(&self) -> &Path {
        match self {
            PendingMigration::Backlog { source, .. } => source,
            PendingMigration::Memory { source, .. } => source,
            PendingMigration::SessionLog { source, .. } => source,
            PendingMigration::LatestSession { source, .. } => source,
        }
    }

    fn target(&self) -> &Path {
        match self {
            PendingMigration::Backlog { target, .. } => target,
            PendingMigration::Memory { target, .. } => target,
            PendingMigration::SessionLog { target, .. } => target,
            PendingMigration::LatestSession { target, .. } => target,
        }
    }

    fn needs_write(&self) -> bool {
        match self {
            PendingMigration::Backlog { needs_write, .. } => *needs_write,
            PendingMigration::Memory { needs_write, .. } => *needs_write,
            PendingMigration::SessionLog { needs_write, .. } => *needs_write,
            PendingMigration::LatestSession { needs_write, .. } => *needs_write,
        }
    }

    fn record_count(&self) -> usize {
        match self {
            PendingMigration::Backlog { parsed, .. } => parsed.tasks.len(),
            PendingMigration::Memory { parsed, .. } => parsed.entries.len(),
            PendingMigration::SessionLog { parsed, .. } => parsed.sessions.len(),
            PendingMigration::LatestSession { .. } => 1,
        }
    }
}

pub fn run_migrate(plan_dir: &Path, options: &MigrateOptions) -> Result<()> {
    let mut pending: Vec<PendingMigration> = Vec::new();

    if let Some(mig) = plan_backlog_migration(plan_dir, options)? {
        pending.push(mig);
    }
    if let Some(mig) = plan_memory_migration(plan_dir, options)? {
        pending.push(mig);
    }
    if let Some(mig) = plan_session_log_migration(plan_dir, options)? {
        pending.push(mig);
    }
    if let Some(mig) = plan_latest_session_migration(plan_dir, options)? {
        pending.push(mig);
    }

    if pending.is_empty() {
        bail!(
            "no migratable .md files found at {}. Either the plan has no state to migrate or migration has already run.",
            plan_dir.display()
        );
    }

    if options.dry_run {
        for mig in &pending {
            println!(
                "dry-run: would write {} ({} records)",
                mig.target().display(),
                mig.record_count()
            );
            if matches!(options.original_policy, OriginalPolicy::Delete) {
                println!("dry-run: would delete {}", mig.source().display());
            }
        }
        return Ok(());
    }

    for mig in &pending {
        if !mig.needs_write() {
            // Idempotent no-op: source parse equals existing target.
            continue;
        }
        match mig {
            PendingMigration::Backlog { parsed, .. } => {
                write_backlog(plan_dir, parsed)?;
                let validated = read_backlog(plan_dir)
                    .with_context(|| "validation round-trip read failed after backlog write")?;
                if !backlogs_equivalent(&validated, parsed) {
                    bail!(
                        "validation mismatch: backlog.yaml re-read does not match parse result."
                    );
                }
            }
            PendingMigration::Memory { parsed, .. } => {
                write_memory(plan_dir, parsed)?;
                let validated = read_memory(plan_dir)
                    .with_context(|| "validation round-trip read failed after memory write")?;
                if !memories_equivalent(&validated, parsed) {
                    bail!(
                        "validation mismatch: memory.yaml re-read does not match parse result."
                    );
                }
            }
            PendingMigration::SessionLog { parsed, .. } => {
                write_session_log(plan_dir, parsed)?;
                let validated = read_session_log(plan_dir).with_context(|| {
                    "validation round-trip read failed after session-log write"
                })?;
                if !session_logs_equivalent(&validated, parsed) {
                    bail!(
                        "validation mismatch: session-log.yaml re-read does not match parse result."
                    );
                }
            }
            PendingMigration::LatestSession { parsed, .. } => {
                write_latest_session(plan_dir, parsed)?;
                let validated = read_latest_session(plan_dir).with_context(|| {
                    "validation round-trip read failed after latest-session write"
                })?;
                if !session_records_equivalent(&validated, parsed) {
                    bail!(
                        "validation mismatch: latest-session.yaml re-read does not match parse result."
                    );
                }
            }
        }
    }

    if matches!(options.original_policy, OriginalPolicy::Delete) {
        for mig in &pending {
            let source = mig.source();
            if source.exists() {
                std::fs::remove_file(source)
                    .with_context(|| format!("failed to delete {}", source.display()))?;
            }
        }
    }
    Ok(())
}

fn plan_backlog_migration(
    plan_dir: &Path,
    options: &MigrateOptions,
) -> Result<Option<PendingMigration>> {
    let source = plan_dir.join("backlog.md");
    let target = plan_dir.join(BACKLOG_FILENAME);
    if !source.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_backlog_markdown(&text)
        .with_context(|| format!("failed to parse {} as legacy backlog markdown", source.display()))?;

    let needs_write = if target.exists() {
        let existing = read_backlog(plan_dir)
            .with_context(|| "failed to read existing backlog.yaml for idempotency check")?;
        if backlogs_equivalent(&existing, &parsed) {
            false
        } else if options.force {
            true
        } else {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    } else {
        true
    };

    Ok(Some(PendingMigration::Backlog {
        source,
        target,
        parsed,
        needs_write,
    }))
}

fn plan_memory_migration(
    plan_dir: &Path,
    options: &MigrateOptions,
) -> Result<Option<PendingMigration>> {
    let source = plan_dir.join("memory.md");
    let target = plan_dir.join(MEMORY_FILENAME);
    if !source.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_memory_markdown(&text)
        .with_context(|| format!("failed to parse {} as legacy memory markdown", source.display()))?;

    let needs_write = if target.exists() {
        let existing = read_memory(plan_dir)
            .with_context(|| "failed to read existing memory.yaml for idempotency check")?;
        if memories_equivalent(&existing, &parsed) {
            false
        } else if options.force {
            true
        } else {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    } else {
        true
    };

    Ok(Some(PendingMigration::Memory {
        source,
        target,
        parsed,
        needs_write,
    }))
}

fn plan_session_log_migration(
    plan_dir: &Path,
    options: &MigrateOptions,
) -> Result<Option<PendingMigration>> {
    let source = plan_dir.join("session-log.md");
    let target = plan_dir.join(SESSION_LOG_FILENAME);
    if !source.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_session_log_markdown(&text).with_context(|| {
        format!(
            "failed to parse {} as legacy session-log markdown",
            source.display()
        )
    })?;

    let needs_write = if target.exists() {
        let existing = read_session_log(plan_dir)
            .with_context(|| "failed to read existing session-log.yaml for idempotency check")?;
        if session_logs_equivalent(&existing, &parsed) {
            false
        } else if options.force {
            true
        } else {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    } else {
        true
    };

    Ok(Some(PendingMigration::SessionLog {
        source,
        target,
        parsed,
        needs_write,
    }))
}

fn plan_latest_session_migration(
    plan_dir: &Path,
    options: &MigrateOptions,
) -> Result<Option<PendingMigration>> {
    let source = plan_dir.join("latest-session.md");
    let target = plan_dir.join(LATEST_SESSION_FILENAME);
    if !source.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_latest_session_markdown(&text).with_context(|| {
        format!(
            "failed to parse {} as legacy latest-session markdown",
            source.display()
        )
    })?;

    let needs_write = if target.exists() {
        let existing = read_latest_session(plan_dir).with_context(|| {
            "failed to read existing latest-session.yaml for idempotency check"
        })?;
        if session_records_equivalent(&existing, &parsed) {
            false
        } else if options.force {
            true
        } else {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    } else {
        true
    };

    Ok(Some(PendingMigration::LatestSession {
        source,
        target,
        parsed,
        needs_write,
    }))
}

fn session_logs_equivalent(a: &SessionLogFile, b: &SessionLogFile) -> bool {
    if a.sessions.len() != b.sessions.len() {
        return false;
    }
    for (sa, sb) in a.sessions.iter().zip(b.sessions.iter()) {
        if !session_records_equivalent(sa, sb) {
            return false;
        }
    }
    true
}

fn session_records_equivalent(a: &SessionRecord, b: &SessionRecord) -> bool {
    a.id == b.id && a.timestamp == b.timestamp && a.phase == b.phase && a.body == b.body
}

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

fn memories_equivalent(a: &MemoryFile, b: &MemoryFile) -> bool {
    if a.entries.len() != b.entries.len() {
        return false;
    }
    for (entry_a, entry_b) in a.entries.iter().zip(b.entries.iter()) {
        if entry_a.id != entry_b.id
            || entry_a.title != entry_b.title
            || entry_a.body != entry_b.body
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

    const TWO_ENTRY_MEMORY: &str = "\
# Memory

## Alpha entry
Alpha body with `code`.

## Beta entry
Beta body.

Multi-paragraph.
";

    const TWO_SESSION_LOG: &str = "\
# Session Log

### Session 1 (2026-04-21T08:03:01Z) — First migrated session

- Bullet one.
- Bullet two.

### Session 2 (2026-04-22T06:14:36Z) — Second migrated session

Paragraph body.
";

    const LATEST_SESSION_MD: &str = "\
### Session 11 (2026-04-22T12:00:00Z) — Fresh latest session

- Body bullet.
";

    fn write_backlog_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("backlog.md"), content).unwrap();
    }

    fn write_memory_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("memory.md"), content).unwrap();
    }

    fn write_session_log_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("session-log.md"), content).unwrap();
    }

    fn write_latest_session_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("latest-session.md"), content).unwrap();
    }

    #[test]
    fn migrate_writes_backlog_yaml_and_keeps_md_by_default() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert!(tmp.path().join(BACKLOG_FILENAME).exists());
        assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
    }

    #[test]
    fn migrate_writes_memory_yaml_when_only_memory_md_present() {
        let tmp = TempDir::new().unwrap();
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert!(tmp.path().join(MEMORY_FILENAME).exists());
        assert!(tmp.path().join("memory.md").exists());

        let memory = read_memory(tmp.path()).unwrap();
        assert_eq!(memory.entries.len(), 2);
        assert_eq!(memory.entries[0].title, "Alpha entry");
    }

    #[test]
    fn migrate_converts_both_files_in_one_run() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert_eq!(read_backlog(tmp.path()).unwrap().tasks.len(), 2);
        assert_eq!(read_memory(tmp.path()).unwrap().entries.len(), 2);
    }

    #[test]
    fn migrate_with_delete_originals_removes_both_md_files() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);

        let opts = MigrateOptions {
            original_policy: OriginalPolicy::Delete,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(!tmp.path().join("backlog.md").exists());
        assert!(!tmp.path().join("memory.md").exists());
    }

    #[test]
    fn migrate_dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);

        let opts = MigrateOptions {
            dry_run: true,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(!tmp.path().join(BACKLOG_FILENAME).exists());
        assert!(!tmp.path().join(MEMORY_FILENAME).exists());
    }

    #[test]
    fn migrate_is_idempotent_on_second_run_with_both_files() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert_eq!(read_backlog(tmp.path()).unwrap().tasks.len(), 2);
        assert_eq!(read_memory(tmp.path()).unwrap().entries.len(), 2);
    }

    #[test]
    fn migrate_refuses_overwrite_on_diverged_memory_yaml_without_force() {
        let tmp = TempDir::new().unwrap();
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        let mut memory = read_memory(tmp.path()).unwrap();
        memory.entries[0].title = "Tampered".into();
        write_memory(tmp.path(), &memory).unwrap();

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already exists"), "error must mention existence: {msg}");
        assert!(msg.contains("--force"), "error must cite --force: {msg}");

        let opts = MigrateOptions { force: true, ..MigrateOptions::default() };
        run_migrate(tmp.path(), &opts).unwrap();
        let memory = read_memory(tmp.path()).unwrap();
        assert_eq!(memory.entries[0].title, "Alpha entry");
    }

    #[test]
    fn migrate_parse_failure_on_one_file_leaves_everything_untouched() {
        let tmp = TempDir::new().unwrap();
        // Valid backlog, malformed memory — the whole run must abort before
        // writing backlog.yaml, honouring the parse-all-then-write-all contract.
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), "## Empty body entry\n\n## Another\n\n");

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("memory") || msg.contains("no body"),
            "error must name the memory failure: {msg}"
        );
        assert!(
            !tmp.path().join(BACKLOG_FILENAME).exists(),
            "no write must occur when any parse fails"
        );
        assert!(
            !tmp.path().join(MEMORY_FILENAME).exists(),
            "no write must occur when any parse fails"
        );
    }

    #[test]
    fn migrate_errors_when_no_md_files_exist() {
        let tmp = TempDir::new().unwrap();
        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no migratable"), "error must explain the empty-plan case: {msg}");
    }

    #[test]
    fn migrate_writes_session_log_yaml_from_session_log_md() {
        let tmp = TempDir::new().unwrap();
        write_session_log_md(tmp.path(), TWO_SESSION_LOG);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert!(tmp.path().join(SESSION_LOG_FILENAME).exists());
        assert!(tmp.path().join("session-log.md").exists(), "default is keep-originals");

        let log = read_session_log(tmp.path()).unwrap();
        assert_eq!(log.sessions.len(), 2);
        assert_eq!(log.sessions[0].id, "2026-04-21-first-migrated-session");
    }

    #[test]
    fn migrate_writes_latest_session_yaml_from_latest_session_md() {
        let tmp = TempDir::new().unwrap();
        write_latest_session_md(tmp.path(), LATEST_SESSION_MD);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        let latest = read_latest_session(tmp.path()).unwrap();
        assert_eq!(latest.id, "2026-04-22-fresh-latest-session");
        assert_eq!(latest.timestamp, "2026-04-22T12:00:00Z");
        assert!(latest.body.contains("Body bullet."));
    }

    #[test]
    fn migrate_converts_all_four_files_in_one_run() {
        let tmp = TempDir::new().unwrap();
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_memory_md(tmp.path(), TWO_ENTRY_MEMORY);
        write_session_log_md(tmp.path(), TWO_SESSION_LOG);
        write_latest_session_md(tmp.path(), LATEST_SESSION_MD);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert_eq!(read_backlog(tmp.path()).unwrap().tasks.len(), 2);
        assert_eq!(read_memory(tmp.path()).unwrap().entries.len(), 2);
        assert_eq!(read_session_log(tmp.path()).unwrap().sessions.len(), 2);
        assert_eq!(
            read_latest_session(tmp.path()).unwrap().id,
            "2026-04-22-fresh-latest-session"
        );
    }

    #[test]
    fn migrate_session_log_is_idempotent_on_second_run() {
        let tmp = TempDir::new().unwrap();
        write_session_log_md(tmp.path(), TWO_SESSION_LOG);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert_eq!(read_session_log(tmp.path()).unwrap().sessions.len(), 2);
    }

    #[test]
    fn migrate_session_log_refuses_overwrite_without_force() {
        let tmp = TempDir::new().unwrap();
        write_session_log_md(tmp.path(), TWO_SESSION_LOG);
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        let mut log = read_session_log(tmp.path()).unwrap();
        log.sessions[0].phase = "tampered".into();
        write_session_log(tmp.path(), &log).unwrap();

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already exists"), "error must mention existence: {msg}");
        assert!(msg.contains("--force"), "error must cite --force: {msg}");

        let opts = MigrateOptions { force: true, ..MigrateOptions::default() };
        run_migrate(tmp.path(), &opts).unwrap();
        let log = read_session_log(tmp.path()).unwrap();
        assert_eq!(log.sessions[0].phase, "work");
    }

    #[test]
    fn migrate_session_log_parse_failure_leaves_nothing_untouched() {
        let tmp = TempDir::new().unwrap();
        // Valid backlog, malformed session-log — the whole run must abort
        // before writing any target, honouring parse-all-then-write-all.
        write_backlog_md(tmp.path(), TWO_TASK_MARKDOWN);
        write_session_log_md(
            tmp.path(),
            "### Session 1 (2026-04-21T08:03:01Z) — Empty body\n",
        );

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("session") || msg.contains("no body"),
            "error must name the session-log failure: {msg}"
        );
        assert!(
            !tmp.path().join(BACKLOG_FILENAME).exists(),
            "no write must occur when any parse fails"
        );
        assert!(
            !tmp.path().join(SESSION_LOG_FILENAME).exists(),
            "no write must occur when any parse fails"
        );
    }

    #[test]
    fn migrate_with_delete_originals_removes_session_log_md_files() {
        let tmp = TempDir::new().unwrap();
        write_session_log_md(tmp.path(), TWO_SESSION_LOG);
        write_latest_session_md(tmp.path(), LATEST_SESSION_MD);

        let opts = MigrateOptions {
            original_policy: OriginalPolicy::Delete,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(!tmp.path().join("session-log.md").exists());
        assert!(!tmp.path().join("latest-session.md").exists());
        assert!(tmp.path().join(SESSION_LOG_FILENAME).exists());
        assert!(tmp.path().join(LATEST_SESSION_FILENAME).exists());
    }
}
