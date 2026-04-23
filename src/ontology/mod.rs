//! Component-relationship ontology (schema v2).
//!
//! Canonical spec: `docs/component-ontology.md`. This module is the
//! Rust realisation of the types in §9.4; callers that want to
//! serialise edges to disk use `yaml_io::{load, load_or_default,
//! save_atomic}`; the shipped `defaults/ontology.yaml` is available as
//! a parsed value via `defaults::parse_embedded` for prompt rendering
//! and drift detection.
//!
//! The module deliberately does not know about Ravel-Lite's
//! `<config-root>`, plans, or phases — that constraint is what lets it
//! migrate to a standalone workspace crate later (§11.2). It has no
//! production caller today; the file surface exists ahead of the
//! migration task that swaps internal callers onto it.

pub mod defaults;
pub mod schema;
pub mod yaml_io;

pub use defaults::{
    parse as parse_ontology_yaml, parse_embedded as parse_embedded_ontology_yaml,
    EMBEDDED_ONTOLOGY_YAML, ONTOLOGY_FILE_SCHEMA_VERSION,
};
pub use schema::{
    Edge, EdgeKind, EvidenceGrade, LifecycleScope, RelatedComponentsFile, SCHEMA_VERSION,
};
pub use yaml_io::{load, load_or_default, save_atomic};

#[cfg(test)]
mod drift_tests {
    //! Bijection tests between `defaults/ontology.yaml` and the Rust
    //! enum surface. Adding a kind / lifecycle / evidence grade in
    //! either location without the other fails these tests — the same
    //! enforcement shape as `every_default_coding_style_file_is_embedded`
    //! in `init.rs`.

    use super::defaults::parse_embedded;
    use super::schema::{EdgeKind, EvidenceGrade, LifecycleScope};
    use std::collections::BTreeSet;

    fn collect_yaml_kinds() -> BTreeSet<String> {
        let ontology = parse_embedded().expect("defaults/ontology.yaml must parse");
        ontology.kinds.into_iter().map(|k| k.name).collect()
    }

    fn collect_yaml_lifecycles() -> BTreeSet<String> {
        let ontology = parse_embedded().expect("defaults/ontology.yaml must parse");
        ontology.lifecycles.into_iter().map(|l| l.name).collect()
    }

    fn collect_yaml_evidence_grades() -> BTreeSet<String> {
        let ontology = parse_embedded().expect("defaults/ontology.yaml must parse");
        ontology
            .evidence_grades
            .into_iter()
            .map(|g| g.name)
            .collect()
    }

    #[test]
    fn edge_kinds_in_yaml_and_rust_are_bijective() {
        let yaml: BTreeSet<String> = collect_yaml_kinds();
        let rust: BTreeSet<String> = EdgeKind::all().iter().map(|k| k.as_str().to_string()).collect();

        let missing_from_rust: Vec<_> = yaml.difference(&rust).cloned().collect();
        let missing_from_yaml: Vec<_> = rust.difference(&yaml).cloned().collect();

        assert!(
            missing_from_rust.is_empty(),
            "kind(s) in defaults/ontology.yaml but not in EdgeKind enum: {missing_from_rust:?}"
        );
        assert!(
            missing_from_yaml.is_empty(),
            "kind(s) in EdgeKind enum but not in defaults/ontology.yaml: {missing_from_yaml:?}"
        );
    }

    #[test]
    fn lifecycles_in_yaml_and_rust_are_bijective() {
        let yaml = collect_yaml_lifecycles();
        let rust: BTreeSet<String> = LifecycleScope::all()
            .iter()
            .map(|l| l.as_str().to_string())
            .collect();
        assert_eq!(yaml, rust, "lifecycle set divergence");
    }

    #[test]
    fn evidence_grades_in_yaml_and_rust_are_bijective() {
        let yaml = collect_yaml_evidence_grades();
        let rust: BTreeSet<String> = EvidenceGrade::all()
            .iter()
            .map(|g| g.as_str().to_string())
            .collect();
        assert_eq!(yaml, rust, "evidence-grade set divergence");
    }

    #[test]
    fn directed_flag_in_yaml_matches_edgekind_is_directed() {
        let ontology = parse_embedded().unwrap();
        for entry in &ontology.kinds {
            let kind = EdgeKind::parse(&entry.name)
                .unwrap_or_else(|| panic!("yaml kind {} has no Rust variant", entry.name));
            assert_eq!(
                kind.is_directed(),
                entry.directed,
                "direction mismatch for kind {}: yaml says directed={}, Rust says {}",
                entry.name,
                entry.directed,
                kind.is_directed()
            );
        }
    }

    #[test]
    fn every_kind_declares_at_least_one_lifecycle() {
        let ontology = parse_embedded().unwrap();
        for entry in &ontology.kinds {
            assert!(
                !entry.lifecycles.is_empty(),
                "kind {} has no typical lifecycles; §5 requires at least one",
                entry.name
            );
            for lifecycle_name in &entry.lifecycles {
                assert!(
                    LifecycleScope::parse(lifecycle_name).is_some(),
                    "kind {} references unknown lifecycle {:?}",
                    entry.name,
                    lifecycle_name
                );
            }
        }
    }
}
