//! Schema types for the discovery pipeline.
//!
//! `SurfaceFile` is the Stage 1 per-project output (`<config-dir>/discover-cache/<name>.yaml`).
//! `SurfaceRecord` is the surface section authored by the LLM; identity
//! fields on `SurfaceFile` are injected by Rust post-parse so the subagent
//! cannot claim a different project name or stale tree SHA.

use serde::{Deserialize, Serialize};

pub const SURFACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFile {
    pub schema_version: u32,
    pub project: String,
    pub tree_sha: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_file_round_trips_via_yaml() {
        let original = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Alpha".to_string(),
            tree_sha: "deadbeef".to_string(),
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
}
