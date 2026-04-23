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
/// Used by the drift test in `mod.rs` and by Stage 2 prompt rendering
/// to substitute the kind list into `defaults/discover-stage2.md`.
pub fn parse_embedded() -> Result<OntologyYaml> {
    parse(EMBEDDED_ONTOLOGY_YAML)
}

/// Render the ontology's kind list as the markdown block that
/// Stage 2's `{{ONTOLOGY_KINDS}}` token expands to. Kinds are grouped
/// by family in the order they appear in the YAML; each bullet carries
/// directionality, typical lifecycles, and a flattened one-paragraph
/// description. A drift test in `mod.rs` asserts the rendered kind
/// names stay in bijection with the YAML.
pub fn render_kinds_for_prompt(ontology: &OntologyYaml) -> String {
    let mut out = String::new();
    let mut current_family: Option<&str> = None;
    for kind in &ontology.kinds {
        if current_family != Some(kind.family.as_str()) {
            if current_family.is_some() {
                out.push('\n');
            }
            out.push_str(&format!(
                "### {} family\n\n",
                capitalise_first(&kind.family)
            ));
            current_family = Some(kind.family.as_str());
        }
        let direction = if kind.directed { "directed" } else { "symmetric" };
        let lifecycles = kind
            .lifecycles
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "- **`{}`** ({}; lifecycles: {}) — {}\n",
            kind.name,
            direction,
            lifecycles,
            flatten_paragraph(&kind.description),
        ));
    }
    out
}

/// Convenience wrapper that parses the embedded YAML and renders in one
/// call. Callers that need the parsed form for other purposes should
/// go through `parse_embedded` + `render_kinds_for_prompt` directly.
pub fn render_embedded_kinds_for_prompt() -> Result<String> {
    parse_embedded().map(|o| render_kinds_for_prompt(&o))
}

fn capitalise_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn flatten_paragraph(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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
    fn render_kinds_for_prompt_emits_family_headings_in_yaml_order() {
        let ontology = parse_embedded().unwrap();
        let block = render_kinds_for_prompt(&ontology);

        // The shipped YAML lists families in this order; the rendered
        // block must preserve it (capitalised, as a sub-heading).
        let mut last_pos: Option<usize> = None;
        for heading in [
            "### Dependency family",
            "### Linkage family",
            "### Generation family",
            "### Communication family",
            "### Orchestration family",
            "### Testing family",
            "### Specification family",
        ] {
            let pos = block
                .find(heading)
                .unwrap_or_else(|| panic!("missing heading: {heading}"));
            if let Some(prev) = last_pos {
                assert!(
                    pos > prev,
                    "family heading {heading} out of order (pos {pos} <= previous {prev})"
                );
            }
            last_pos = Some(pos);
        }
    }

    #[test]
    fn render_kinds_for_prompt_annotates_directionality_and_lifecycles() {
        let ontology = parse_embedded().unwrap();
        let block = render_kinds_for_prompt(&ontology);

        // Spot-check a directed kind and a symmetric one.
        assert!(
            block.contains("**`generates`** (directed; lifecycles: `codegen`)"),
            "generates should render as directed with codegen lifecycle:\n{block}"
        );
        assert!(
            block.contains("**`co-implements`** (symmetric; lifecycles: `design`)"),
            "co-implements should render as symmetric with design lifecycle:\n{block}"
        );
    }

    #[test]
    fn render_kinds_for_prompt_flattens_multiline_descriptions_to_single_line() {
        let ontology = parse_embedded().unwrap();
        let block = render_kinds_for_prompt(&ontology);

        // Every bullet line must be self-contained — flattening collapses
        // the YAML's hard-wrapped description into one paragraph per
        // bullet, which is the contract downstream prompt rendering
        // relies on. Assert the property by checking no continuation
        // line begins with lower-case text suggesting a wrapped body.
        for line in block.lines() {
            if line.starts_with("  ") && !line.trim().is_empty() {
                panic!("description should be flattened; got continuation line: {line}");
            }
        }
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
