//! Atomic read/write of `<plan>/session-log.yaml` and
//! `<plan>/latest-session.yaml`.
//!
//! `latest-session.yaml` stores a single `SessionRecord` at rest; the
//! reader/writer functions take/return the record directly rather than
//! wrapping it in a one-element list. `session-log.yaml` stores the
//! `SessionLogFile` container (the `sessions:` list).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::{SessionLogFile, SessionRecord};
use crate::state::filenames::{LATEST_SESSION_FILENAME, SESSION_LOG_FILENAME};

pub fn session_log_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(SESSION_LOG_FILENAME)
}

pub fn latest_session_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(LATEST_SESSION_FILENAME)
}

pub fn read_session_log(plan_dir: &Path) -> Result<SessionLogFile> {
    let path = session_log_path(plan_dir);
    if !path.exists() {
        bail!(
            "{SESSION_LOG_FILENAME} not found at {}. Run `ravel-lite state migrate` to convert an existing session-log.md.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: SessionLogFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {SESSION_LOG_FILENAME} schema", path.display()))?;
    Ok(parsed)
}

pub fn write_session_log(plan_dir: &Path, log: &SessionLogFile) -> Result<()> {
    let path = session_log_path(plan_dir);
    let yaml = serde_yaml::to_string(log)
        .with_context(|| format!("Failed to serialise {SESSION_LOG_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
}

pub fn read_latest_session(plan_dir: &Path) -> Result<SessionRecord> {
    let path = latest_session_path(plan_dir);
    if !path.exists() {
        bail!(
            "{LATEST_SESSION_FILENAME} not found at {}. analyse-work is expected to have produced it.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: SessionRecord = serde_yaml::from_str(&text).with_context(|| {
        format!(
            "Failed to parse {} as a session record",
            path.display()
        )
    })?;
    Ok(parsed)
}

pub fn write_latest_session(plan_dir: &Path, record: &SessionRecord) -> Result<()> {
    let path = latest_session_path(plan_dir);
    let yaml = serde_yaml::to_string(record)
        .with_context(|| format!("Failed to serialise {LATEST_SESSION_FILENAME}"))?;
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
    use tempfile::TempDir;

    fn sample_record() -> SessionRecord {
        SessionRecord {
            id: "2026-04-22-sample-session".into(),
            timestamp: "2026-04-22T14:33:07Z".into(),
            phase: "work".into(),
            body: "Paragraph one.\n\nParagraph two.\n".into(),
        }
    }

    #[test]
    fn write_then_read_round_trips_session_log() {
        let tmp = TempDir::new().unwrap();
        let log = SessionLogFile {
            sessions: vec![sample_record()],
            extra: Default::default(),
        };
        write_session_log(tmp.path(), &log).unwrap();

        let round_tripped = read_session_log(tmp.path()).unwrap();
        assert_eq!(round_tripped.sessions.len(), 1);
        assert_eq!(round_tripped.sessions[0].id, "2026-04-22-sample-session");
        assert_eq!(round_tripped.sessions[0].body, sample_record().body);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_body() {
        let tmp = TempDir::new().unwrap();
        let log = SessionLogFile {
            sessions: vec![sample_record()],
            extra: Default::default(),
        };
        write_session_log(tmp.path(), &log).unwrap();

        let raw = std::fs::read_to_string(session_log_path(tmp.path())).unwrap();
        assert!(
            raw.contains("body: |") || raw.contains("body: |-"),
            "multi-line body must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_session_log_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_session_log(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(SESSION_LOG_FILENAME),
            "error must name {SESSION_LOG_FILENAME}: {msg}"
        );
        assert!(msg.contains("state migrate"), "error must suggest migrate: {msg}");
    }

    #[test]
    fn write_then_read_round_trips_latest_session() {
        let tmp = TempDir::new().unwrap();
        let record = sample_record();
        write_latest_session(tmp.path(), &record).unwrap();

        let round_tripped = read_latest_session(tmp.path()).unwrap();
        assert_eq!(round_tripped.id, record.id);
        assert_eq!(round_tripped.phase, record.phase);
        assert_eq!(round_tripped.body, record.body);
    }

    #[test]
    fn read_latest_errors_when_file_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_latest_session(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(LATEST_SESSION_FILENAME),
            "error must name {LATEST_SESSION_FILENAME}: {msg}"
        );
    }
}
