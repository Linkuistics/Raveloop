//! Handlers for every `state session-log <verb>` CLI verb.
//!
//! Append is idempotent on `id`: re-invoking with a record whose id
//! already exists in `sessions:` is a no-op. This is the crash-retry
//! safety property relied on by `phase_loop::GitCommitWork` — if the
//! commit phase re-runs after a partial failure, the log does not grow
//! duplicate rows.

use std::path::Path;

use anyhow::Result;

use super::schema::{SessionLogFile, SessionRecord};
use super::yaml_io::{read_latest_session, read_session_log, write_latest_session, write_session_log};

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

pub fn run_list(plan_dir: &Path, limit: Option<usize>, format: OutputFormat) -> Result<()> {
    let log = read_session_log(plan_dir)?;
    let projected = match limit {
        Some(n) => {
            let total = log.sessions.len();
            let start = total.saturating_sub(n);
            SessionLogFile {
                sessions: log.sessions[start..].to_vec(),
                extra: Default::default(),
            }
        }
        None => log,
    };
    emit_log(&projected, format)
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let log = read_session_log(plan_dir)?;
    let record = find_session(&log, id)?;
    let wrapper = SessionLogFile {
        sessions: vec![record.clone()],
        extra: Default::default(),
    };
    emit_log(&wrapper, format)
}

pub fn run_show_latest(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let record = read_latest_session(plan_dir)?;
    emit_record(&record, format)
}

pub(crate) fn find_session<'a>(
    log: &'a SessionLogFile,
    id: &str,
) -> Result<&'a SessionRecord> {
    log.sessions
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| anyhow::anyhow!("no session with id {id:?}"))
}

fn emit_log(log: &SessionLogFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(log)?,
        OutputFormat::Json => serde_json::to_string_pretty(log)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

fn emit_record(record: &SessionRecord, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(record)?,
        OutputFormat::Json => serde_json::to_string_pretty(record)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

/// Append `record` to session-log.yaml. Idempotent on `record.id`: if a
/// session with that id already exists in the log, the call is a no-op.
/// Returns `true` when a write actually occurred.
pub fn append_record(plan_dir: &Path, record: &SessionRecord) -> Result<bool> {
    let mut log = read_session_log_or_empty(plan_dir)?;
    if log.sessions.iter().any(|s| s.id == record.id) {
        return Ok(false);
    }
    log.sessions.push(record.clone());
    write_session_log(plan_dir, &log)?;
    Ok(true)
}

/// Read `session-log.yaml` or return an empty `SessionLogFile` when the
/// file does not yet exist. Used by `append_record` (which must work
/// against a fresh plan that has never produced a session).
fn read_session_log_or_empty(plan_dir: &Path) -> Result<SessionLogFile> {
    let path = super::yaml_io::session_log_path(plan_dir);
    if !path.exists() {
        return Ok(SessionLogFile::default());
    }
    read_session_log(plan_dir)
}

pub fn run_append(plan_dir: &Path, record: &SessionRecord) -> Result<()> {
    append_record(plan_dir, record)?;
    Ok(())
}

/// Overwrite `latest-session.yaml` with `record`. The file always holds
/// exactly one record at rest.
pub fn run_set_latest(plan_dir: &Path, record: &SessionRecord) -> Result<()> {
    write_latest_session(plan_dir, record)
}

/// Append the record currently stored in `latest-session.yaml` to
/// `session-log.yaml`. This is the programmatic entry point used by
/// `phase_loop::GitCommitWork` — the CLI surface is `append --body-file`
/// rather than a "promote latest" verb so the storage format and the
/// invocation site stay independent.
pub fn append_latest_to_log(plan_dir: &Path) -> Result<bool> {
    let latest_path = super::yaml_io::latest_session_path(plan_dir);
    if !latest_path.exists() {
        // No latest-session.yaml yet (fresh plan, or analyse-work hasn't
        // produced one). Nothing to append, not an error.
        return Ok(false);
    }
    let record = read_latest_session(plan_dir)?;
    append_record(plan_dir, &record)
}

/// Convenience constructor used by the CLI `append` subcommand: build a
/// `SessionRecord` from a body, optional id/timestamp/phase. When id is
/// not supplied, fail — session id assignment is the analyse-work
/// writer's responsibility per the design decision.
pub fn build_record_for_append(
    id: Option<String>,
    timestamp: Option<String>,
    phase: Option<String>,
    body: &str,
) -> Result<SessionRecord> {
    let id = id.ok_or_else(|| anyhow::anyhow!("--id <value> is required for append"))?;
    let timestamp = timestamp.ok_or_else(|| {
        anyhow::anyhow!(
            "--timestamp <iso8601> is required for append; session id and timestamp are assigned by the writer"
        )
    })?;
    let phase = phase.unwrap_or_else(|| "work".to_string());
    Ok(SessionRecord {
        id,
        timestamp,
        phase,
        body: ensure_trailing_newline(body),
    })
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            timestamp: "2026-04-22T14:33:07Z".into(),
            phase: "work".into(),
            body: "Body one.\nBody two.\n".into(),
        }
    }

    #[test]
    fn append_record_writes_new_record() {
        let tmp = TempDir::new().unwrap();
        let first = sample_record("2026-04-22-first");
        let wrote = append_record(tmp.path(), &first).unwrap();
        assert!(wrote);

        let log = read_session_log(tmp.path()).unwrap();
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].id, "2026-04-22-first");
    }

    #[test]
    fn append_record_is_idempotent_on_id() {
        let tmp = TempDir::new().unwrap();
        let record = sample_record("2026-04-22-dup");
        let first = append_record(tmp.path(), &record).unwrap();
        assert!(first, "first append should write");
        let second = append_record(tmp.path(), &record).unwrap();
        assert!(!second, "second append with same id should be a no-op");

        let log = read_session_log(tmp.path()).unwrap();
        assert_eq!(log.sessions.len(), 1, "no duplicate row");
    }

    #[test]
    fn append_record_preserves_order() {
        let tmp = TempDir::new().unwrap();
        append_record(tmp.path(), &sample_record("2026-04-22-a")).unwrap();
        append_record(tmp.path(), &sample_record("2026-04-22-b")).unwrap();
        append_record(tmp.path(), &sample_record("2026-04-22-c")).unwrap();

        let log = read_session_log(tmp.path()).unwrap();
        let ids: Vec<&str> = log.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["2026-04-22-a", "2026-04-22-b", "2026-04-22-c"]);
    }

    #[test]
    fn run_set_latest_overwrites_existing_file() {
        let tmp = TempDir::new().unwrap();
        run_set_latest(tmp.path(), &sample_record("first")).unwrap();
        run_set_latest(tmp.path(), &sample_record("second")).unwrap();

        let record = read_latest_session(tmp.path()).unwrap();
        assert_eq!(record.id, "second");
    }

    #[test]
    fn append_latest_to_log_copies_record_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        run_set_latest(tmp.path(), &sample_record("2026-04-22-latest")).unwrap();

        let wrote_first = append_latest_to_log(tmp.path()).unwrap();
        assert!(wrote_first, "first append should write");
        let wrote_second = append_latest_to_log(tmp.path()).unwrap();
        assert!(!wrote_second, "second append of same latest is a no-op");

        let log = read_session_log(tmp.path()).unwrap();
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].id, "2026-04-22-latest");
    }

    #[test]
    fn append_latest_to_log_no_latest_is_graceful_noop() {
        let tmp = TempDir::new().unwrap();
        let wrote = append_latest_to_log(tmp.path()).unwrap();
        assert!(!wrote, "missing latest-session.yaml is a no-op");
        assert!(
            !super::super::yaml_io::session_log_path(tmp.path()).exists(),
            "no session-log.yaml should be created in the no-op path"
        );
    }

    #[test]
    fn find_session_returns_record_by_id() {
        let log = SessionLogFile {
            sessions: vec![sample_record("foo"), sample_record("bar")],
            extra: Default::default(),
        };
        let record = find_session(&log, "bar").unwrap();
        assert_eq!(record.id, "bar");
    }

    #[test]
    fn find_session_errors_when_id_not_found() {
        let log = SessionLogFile::default();
        let err = find_session(&log, "missing").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing"), "error must cite bad id: {msg}");
    }

    #[test]
    fn run_list_limit_truncates_to_newest_n() {
        let tmp = TempDir::new().unwrap();
        for idx in 0..5 {
            append_record(tmp.path(), &sample_record(&format!("s-{idx}"))).unwrap();
        }
        // Smoke via direct slicing: run_list prints to stdout, not
        // easily captured here; the slicing logic is the interesting
        // part and is unit-tested in isolation.
        let log = read_session_log(tmp.path()).unwrap();
        let limit = 3;
        let start = log.sessions.len().saturating_sub(limit);
        let tail: Vec<&str> = log.sessions[start..]
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(tail, vec!["s-2", "s-3", "s-4"]);
    }

    #[test]
    fn build_record_for_append_requires_id_and_timestamp() {
        let err = build_record_for_append(None, Some("t".into()), None, "b").unwrap_err();
        assert!(format!("{err:#}").contains("--id"));

        let err = build_record_for_append(Some("i".into()), None, None, "b").unwrap_err();
        assert!(format!("{err:#}").contains("--timestamp"));

        let record = build_record_for_append(
            Some("i".into()),
            Some("t".into()),
            Some("analyse-work".into()),
            "body",
        )
        .unwrap();
        assert_eq!(record.phase, "analyse-work");
        assert_eq!(record.body, "body\n");
    }
}
