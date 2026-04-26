//! Atomic read/write of `<plan>/backlog.yaml`. Format preservation
//! note: serde_yaml 0.9 emits multi-line strings as `|` block scalars
//! automatically when they contain a newline, which renders Results /
//! description bodies readably without escaping.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::BacklogFile;
use crate::state::filenames::BACKLOG_FILENAME;

pub fn backlog_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(BACKLOG_FILENAME)
}

pub fn read_backlog(plan_dir: &Path) -> Result<BacklogFile> {
    let path = backlog_path(plan_dir);
    if !path.exists() {
        bail!(
            "{BACKLOG_FILENAME} not found at {}. Run `ravel-lite state migrate` to convert an existing backlog.md.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: BacklogFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {BACKLOG_FILENAME} schema", path.display()))?;
    Ok(parsed)
}

pub fn write_backlog(plan_dir: &Path, backlog: &BacklogFile) -> Result<()> {
    let path = backlog_path(plan_dir);
    let yaml = serde_yaml::to_string(backlog)
        .with_context(|| format!("Failed to serialise {BACKLOG_FILENAME}"))?;
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
        assert!(msg.contains(BACKLOG_FILENAME), "error must name {BACKLOG_FILENAME}: {msg}");
        assert!(msg.contains("state migrate"), "error must suggest migrate: {msg}");
    }
}
