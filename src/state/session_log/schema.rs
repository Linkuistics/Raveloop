//! Typed schema for `<plan>/session-log.yaml` and `<plan>/latest-session.yaml`.
//!
//! `SessionRecord` is the shared entry shape. `session-log.yaml` wraps it
//! in a `sessions:` list; `latest-session.yaml` stores exactly one record
//! at rest (written by analyse-work, consumed by git-commit-work).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub timestamp: String,
    pub phase: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionLogFile {
    #[serde(default)]
    pub sessions: Vec<SessionRecord>,
    /// Preserve unknown top-level keys (e.g. `schema_version`) across a
    /// read/write cycle so future schema extensions are not dropped by an
    /// older reader.
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_yaml::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trips_through_yaml() {
        let record = SessionRecord {
            id: "2026-04-22-example".into(),
            timestamp: "2026-04-22T14:33:07Z".into(),
            phase: "work".into(),
            body: "Paragraph one.\n\nParagraph two.\n".into(),
        };
        let yaml = serde_yaml::to_string(&record).unwrap();
        let decoded: SessionRecord = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.id, record.id);
        assert_eq!(decoded.timestamp, record.timestamp);
        assert_eq!(decoded.phase, record.phase);
        assert_eq!(decoded.body, record.body);
    }

    #[test]
    fn session_log_file_preserves_unknown_top_level_keys() {
        let input = r#"
sessions: []
schema_version: 1
"#;
        let parsed: SessionLogFile = serde_yaml::from_str(input).unwrap();
        assert!(parsed.extra.contains_key("schema_version"));
        let re_emitted = serde_yaml::to_string(&parsed).unwrap();
        assert!(
            re_emitted.contains("schema_version"),
            "extra keys must round-trip: {re_emitted}"
        );
    }
}
