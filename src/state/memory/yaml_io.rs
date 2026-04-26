//! Atomic read/write of `<plan>/memory.yaml`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::MemoryFile;
use crate::state::filenames::MEMORY_FILENAME;

pub fn memory_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(MEMORY_FILENAME)
}

pub fn read_memory(plan_dir: &Path) -> Result<MemoryFile> {
    let path = memory_path(plan_dir);
    if !path.exists() {
        bail!(
            "{MEMORY_FILENAME} not found at {}. Run `ravel-lite state migrate` to convert an existing memory.md.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: MemoryFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {MEMORY_FILENAME} schema", path.display()))?;
    Ok(parsed)
}

pub fn write_memory(plan_dir: &Path, memory: &MemoryFile) -> Result<()> {
    let path = memory_path(plan_dir);
    let yaml = serde_yaml::to_string(memory)
        .with_context(|| format!("Failed to serialise {MEMORY_FILENAME}"))?;
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
    use crate::state::memory::schema::MemoryEntry;
    use tempfile::TempDir;

    fn sample_entry() -> MemoryEntry {
        MemoryEntry {
            id: "sample".into(),
            title: "Sample entry".into(),
            body: "Paragraph one.\n\nParagraph two, with `code`.\n".into(),
        }
    }

    #[test]
    fn write_then_read_round_trips_entry_fields() {
        let tmp = TempDir::new().unwrap();
        let memory = MemoryFile {
            entries: vec![sample_entry()],
            extra: Default::default(),
        };
        write_memory(tmp.path(), &memory).unwrap();

        let round_tripped = read_memory(tmp.path()).unwrap();
        assert_eq!(round_tripped.entries.len(), 1);
        assert_eq!(round_tripped.entries[0].id, "sample");
        assert_eq!(round_tripped.entries[0].body, sample_entry().body);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_body() {
        let tmp = TempDir::new().unwrap();
        let memory = MemoryFile {
            entries: vec![sample_entry()],
            extra: Default::default(),
        };
        write_memory(tmp.path(), &memory).unwrap();

        let raw = std::fs::read_to_string(memory_path(tmp.path())).unwrap();
        assert!(
            raw.contains("body: |") || raw.contains("body: |-"),
            "multi-line body must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_memory_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_memory(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(MEMORY_FILENAME), "error must name {MEMORY_FILENAME}: {msg}");
        assert!(msg.contains("state migrate"), "error must suggest migrate: {msg}");
    }
}
