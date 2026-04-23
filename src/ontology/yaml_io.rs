//! Load / save helpers for `related-components.yaml`.
//!
//! Host-agnostic: callers supply the full path to the file, not a
//! config root. The library has no opinion on filesystem conventions
//! — Ravel-Lite's adapter in `src/main.rs` owns that.
//!
//! `load_or_default` returns the schema-v2 default when the file is
//! absent. `load` is strict: absent file is an error. Both hard-error on
//! any `schema_version` other than 2 (§9.1 — no in-memory upgrade path).
//! The delete-and-regenerate upgrade strategy lives at the catalog
//! layer, not here.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::schema::{RelatedComponentsFile, SCHEMA_VERSION};

/// Return `RelatedComponentsFile::default()` when `path` is absent;
/// parse and validate otherwise. Every edge is `validate`d on load so
/// a malformed file fails fast with a contextual error.
pub fn load_or_default(path: &Path) -> Result<RelatedComponentsFile> {
    if !path.exists() {
        return Ok(RelatedComponentsFile::default());
    }
    load(path)
}

/// Strict load: absent file is an error. Use `load_or_default` for the
/// tolerant variant. Both paths hard-error on a `schema_version`
/// mismatch per §9.1.
pub fn load(path: &Path) -> Result<RelatedComponentsFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: RelatedComponentsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {} as related-components.yaml", path.display()))?;
    if file.schema_version != SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}. \
             Related-components files are a generated artifact; the \
             supported upgrade path is to delete this file and re-run \
             `ravel-lite state related-components discover --apply` \
             (see docs/component-ontology.md §12).",
            path.display(),
            file.schema_version,
            SCHEMA_VERSION
        );
    }
    for edge in &file.edges {
        edge.validate()
            .with_context(|| format!("invalid edge in {}", path.display()))?;
    }
    Ok(file)
}

/// Write via temp-file-then-rename so a crash mid-write cannot leave a
/// half-serialised file at `path`. Temp file sits alongside the target
/// so the rename is on the same filesystem.
pub fn save_atomic(path: &Path, file: &RelatedComponentsFile) -> Result<()> {
    let yaml = serde_yaml::to_string(file)
        .context("failed to serialise related-components to YAML")?;
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name component", path.display()))?
        .to_string_lossy()
        .into_owned();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::schema::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
    use super::*;
    use tempfile::TempDir;

    fn strong_edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle,
            participants: vec![a.into(), b.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![format!("{a}.produces_files")],
            rationale: "example".into(),
        }
    }

    #[test]
    fn load_or_default_returns_empty_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_default(&tmp.path().join("related-components.yaml")).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("related-components.yaml");
        let mut original = RelatedComponentsFile::default();
        original
            .add_edge(strong_edge(
                EdgeKind::Generates,
                LifecycleScope::Codegen,
                "Ravel-Lite",
                "Ravel",
            ))
            .unwrap();
        save_atomic(&path, &original).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_rejects_non_matching_schema_version() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("related-components.yaml");
        std::fs::write(&path, "schema_version: 1\nedges: []\n").unwrap();
        let err = load(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must mention the version key: {msg}");
        assert!(msg.contains("discover --apply"), "error must point at the regenerate path: {msg}");
    }

    #[test]
    fn load_rejects_invalid_edge() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("related-components.yaml");
        // Self-loop — fails `Edge::validate` on load.
        let body = "\
schema_version: 2
edges:
  - kind: depends-on
    lifecycle: build
    participants: [A, A]
    evidence_grade: strong
    evidence_fields: [A.consumes_files]
    rationale: self-loop
";
        std::fs::write(&path, body).unwrap();
        let err = load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("distinct"));
    }

    #[test]
    fn save_atomic_never_leaves_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("related-components.yaml");
        save_atomic(&path, &RelatedComponentsFile::default()).unwrap();
        let leftover: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(
            leftover.is_empty(),
            "atomic save left a temp file: {:?}",
            leftover.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
    }
}
