// The bin (`main.rs`) does not yet exercise ontology-yaml parsing —
// `parse_embedded` and the entry types feed only the drift tests in
// `mod.rs`, plus the next backlog task's prompt-rendering code. The
// allow keeps the binary compilation unit clean without hiding real
// dead code: removing usages from the lib's drift tests would still
// leave these functions without coverage to flag.
#![allow(dead_code)]

//! Parsed form of `defaults/ontology.yaml`.
//!
//! The data form of docs/component-ontology.md §5 + §6 + §3.2. Its
//! purpose is twofold (§8):
//!
//! 1. **Drift guard.** A test in `mod.rs` parses the embedded YAML
//!    shipped with the binary and asserts bijection with the Rust
//!    enum surface in `schema.rs`. Adding a kind in one place without
//!    the other fails the build.
//! 2. **Prompt input.** Later tasks will substitute the kind list into
//!    `defaults/discover-stage2.md` via a `{{ONTOLOGY_KINDS}}` token.
//!    The data stays in one place; the prompt renders from it.
//!
//! The file's `schema_version` is independent of
//! `related-components.yaml`'s `schema_version` — this one versions
//! the ontology definition, the other versions the on-disk edge graph.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const ONTOLOGY_FILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyYaml {
    pub schema_version: u32,
    #[serde(default)]
    pub kinds: Vec<KindEntry>,
    #[serde(default)]
    pub lifecycles: Vec<LifecycleEntry>,
    #[serde(default)]
    pub evidence_grades: Vec<EvidenceGradeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindEntry {
    pub name: String,
    pub family: String,
    pub directed: bool,
    #[serde(default)]
    pub lifecycles: Vec<String>,
    #[serde(default)]
    pub spdx: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEntry {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceGradeEntry {
    pub name: String,
    pub criterion: String,
}

/// Content of the shipped `defaults/ontology.yaml`, embedded at
/// compile time. `include_str!` means a drift test does not need a
/// runtime dependency on the filesystem.
pub const EMBEDDED_ONTOLOGY_YAML: &str = include_str!("../../defaults/ontology.yaml");

pub fn parse(yaml: &str) -> Result<OntologyYaml> {
    let parsed: OntologyYaml = serde_yaml::from_str(yaml)
        .context("failed to parse ontology YAML")?;
    if parsed.schema_version != ONTOLOGY_FILE_SCHEMA_VERSION {
        anyhow::bail!(
            "ontology YAML schema_version is {}, expected {}",
            parsed.schema_version,
            ONTOLOGY_FILE_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

/// Parse the shipped `defaults/ontology.yaml` embedded in the binary.
/// Used by the drift test in `mod.rs` and by future prompt-rendering
/// code that substitutes the kind list into Stage 2.
pub fn parse_embedded() -> Result<OntologyYaml> {
    parse(EMBEDDED_ONTOLOGY_YAML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_ontology_yaml_parses() {
        parse_embedded().unwrap();
    }

    #[test]
    fn shipped_ontology_yaml_declares_expected_schema_version() {
        let ontology = parse_embedded().unwrap();
        assert_eq!(ontology.schema_version, ONTOLOGY_FILE_SCHEMA_VERSION);
    }

    #[test]
    fn kind_entry_round_trips_through_yaml() {
        let entry = KindEntry {
            name: "depends-on".into(),
            family: "dependency".into(),
            directed: true,
            lifecycles: vec!["build".into(), "runtime".into()],
            spdx: Some("dependsOn".into()),
            description: "one-line body\n".into(),
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: KindEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn kind_entry_accepts_null_spdx() {
        let yaml = "\
name: scaffolds
family: generation
directed: true
lifecycles: [dev-workflow]
spdx: null
description: |
  body
";
        let parsed: KindEntry = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.spdx, None);
    }
}
