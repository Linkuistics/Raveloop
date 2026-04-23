//! Schema types for the discovery pipeline.
//!
//! `SurfaceFile` is the Stage 1 per-project output (`<config-dir>/discover-cache/<name>.yaml`).
//! `SurfaceRecord` is the surface section authored by the LLM; identity
//! fields on `SurfaceFile` are injected by Rust post-parse so the subagent
//! cannot claim a different project name or stale tree SHA.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::tree_sha::ProjectState;
use crate::related_projects::EdgeKind;

pub const SURFACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFile {
    pub schema_version: u32,
    pub project: String,
    pub tree_sha: String,
    /// Hash of uncommitted subtree state (diff + untracked contents).
    /// Empty string when the subtree was clean at analysis time. Cache
    /// hit requires both `tree_sha` AND `dirty_hash` to match the
    /// project's current state. Serde-default preserves read-compat with
    /// pre-existing cache files that predate this field.
    #[serde(default)]
    pub dirty_hash: String,
    pub analysed_at: String,
    pub surface: SurfaceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SurfaceRecord {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub consumes_files: Vec<String>,
    #[serde(default)]
    pub produces_files: Vec<String>,
    #[serde(default)]
    pub network_endpoints: Vec<String>,
    #[serde(default)]
    pub data_formats: Vec<String>,
    #[serde(default)]
    pub external_tools_spawned: Vec<String>,
    #[serde(default)]
    pub explicit_cross_project_mentions: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

pub const PROPOSALS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalsFile {
    pub schema_version: u32,
    pub generated_at: String,
    /// Map from project name to the `ProjectState` (tree_sha + dirty_hash)
    /// captured at proposal-generation time. The pair is the actual cache
    /// key used by Stage 1; storing only `tree_sha` here silently omitted
    /// the dirty_hash half and misrepresented the cache state.
    #[serde(default)]
    pub source_project_states: BTreeMap<String, ProjectState>,
    #[serde(default)]
    pub proposals: Vec<ProposalRecord>,
    #[serde(default)]
    pub failures: Vec<Stage1Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub kind: EdgeKind,
    pub participants: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub supporting_surface_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage1Failure {
    pub project: String,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_file_round_trips_via_yaml() {
        let original = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Alpha".to_string(),
            tree_sha: "deadbeef".to_string(),
            dirty_hash: "feedface".to_string(),
            analysed_at: "2026-04-22T12:00:00Z".to_string(),
            surface: SurfaceRecord {
                purpose: "Does the alpha thing.".to_string(),
                consumes_files: vec!["~/.config/alpha/*.yaml".to_string()],
                produces_files: vec!["/tmp/alpha-output/*.json".to_string()],
                network_endpoints: vec!["grpc://alpha-service:50051".to_string()],
                data_formats: vec!["AlphaRecord".to_string()],
                external_tools_spawned: vec!["git".to_string()],
                explicit_cross_project_mentions: vec!["Beta".to_string()],
                notes: String::new(),
            },
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: SurfaceFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn surface_record_empty_fields_round_trip_as_defaults() {
        let yaml = "purpose: hello\n";
        let parsed: SurfaceRecord = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.purpose, "hello");
        assert!(parsed.consumes_files.is_empty());
        assert!(parsed.produces_files.is_empty());
        assert!(parsed.network_endpoints.is_empty());
        assert!(parsed.data_formats.is_empty());
        assert!(parsed.external_tools_spawned.is_empty());
        assert!(parsed.explicit_cross_project_mentions.is_empty());
        assert!(parsed.notes.is_empty());
    }

    #[test]
    fn proposals_file_round_trips_via_yaml() {
        let original = ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "2026-04-22T12:05:00Z".to_string(),
            source_project_states: [
                (
                    "Alpha".to_string(),
                    ProjectState {
                        tree_sha: "abc123".to_string(),
                        dirty_hash: "dirty-alpha".to_string(),
                    },
                ),
                (
                    "Beta".to_string(),
                    ProjectState {
                        tree_sha: "def456".to_string(),
                        dirty_hash: String::new(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            proposals: vec![
                ProposalRecord {
                    kind: EdgeKind::Sibling,
                    participants: vec!["Alpha".to_string(), "Beta".to_string()],
                    rationale: "Both speak the same gRPC protocol.".to_string(),
                    supporting_surface_fields: vec![
                        "Alpha.surface.network_endpoints".to_string(),
                        "Beta.surface.network_endpoints".to_string(),
                    ],
                },
            ],
            failures: vec![],
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: ProposalsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn proposals_file_reads_legacy_source_tree_shas_as_empty_states() {
        // A proposals file written before the rename will have
        // `source_tree_shas:` as its field name. The `source_project_states`
        // field is purely informational (never consulted for cache
        // correctness), so the acceptable behaviour is: read succeeds with
        // an empty map, legacy field silently ignored.
        let yaml = "schema_version: 1\n\
                    generated_at: \"2026-04-22T12:05:00Z\"\n\
                    source_tree_shas:\n\
                      Alpha: abc123\n\
                    proposals: []\n";
        let parsed: ProposalsFile = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.source_project_states.is_empty());
    }

    #[test]
    fn project_state_reads_without_dirty_hash_as_default() {
        // Forward-compat for files emitted before dirty_hash existed in
        // ProjectState's serialised form — deserialise must succeed with
        // `dirty_hash: ""`.
        let yaml = "tree_sha: abc123\n";
        let parsed: ProjectState = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.tree_sha, "abc123");
        assert!(parsed.dirty_hash.is_empty());
    }
}
