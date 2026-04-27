//! Host adapter for the component-ontology graph at
//! `<config_root>/related-components.yaml`.
//!
//! The on-disk types and validation rules live in `crate::ontology` —
//! that module is host-agnostic so it can graduate to a workspace crate.
//! Everything host-specific (the filename, the `<config-root>` join, the
//! per-user `projects.yaml` resolver, the CLI verbs) lives here.
//!
//! Schema is v2: every edge carries `(kind, lifecycle, participants,
//! evidence_grade, evidence_fields, rationale)`. The loader rejects any
//! file whose `schema_version` is not 2 (enforced in
//! `crate::ontology::yaml_io::load_or_default`). There is no in-memory
//! v1 → v2 upgrader — the file is a generated artifact, so
//! delete-and-regenerate is the supported upgrade path
//! (`docs/component-ontology.md` §12).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::ontology::{self, Edge, EvidenceGrade, RelatedComponentsFile};
use crate::projects::{self, ProjectsCatalog};
use crate::state::filenames::RELATED_COMPONENTS_FILENAME;

// Re-export the v2 ontology surface that the host needs to construct
// edges through this adapter — the binary crate (`main.rs`) and tests
// route through `crate::related_components::*` rather than touching
// `crate::ontology` directly.
pub use crate::ontology::{EdgeKind, LifecycleScope};

/// Filter set for `run_list`. An empty filter emits every edge; each
/// populated field narrows the match set by AND-composition.
pub struct ListFilter<'a> {
    pub plan: Option<&'a Path>,
    pub kind: Option<EdgeKind>,
    pub lifecycle: Option<LifecycleScope>,
}

/// Full-field request for `run_add_edge`. Participants are borrowed
/// because their lifetime is trivially shorter than the caller's stack
/// frame; the owned fields (`evidence_fields`, `rationale`) move into
/// the constructed `Edge`.
pub struct AddEdgeRequest<'a> {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub a: &'a str,
    pub b: &'a str,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

pub fn load_or_empty(config_root: &Path) -> Result<RelatedComponentsFile> {
    ontology::load_or_default(&config_root.join(RELATED_COMPONENTS_FILENAME))
}

pub fn save_atomic(config_root: &Path, file: &RelatedComponentsFile) -> Result<()> {
    ontology::save_atomic(&config_root.join(RELATED_COMPONENTS_FILENAME), file)
}

/// Cascade for `projects::run_rename`. Loads, rewrites every participant
/// reference, saves. No-op when the file is absent (a catalog without
/// any edge file is valid). Symmetric kinds are re-sorted internally by
/// the ontology layer.
pub fn rename_component_in_edges(config_root: &Path, old: &str, new: &str) -> Result<()> {
    let path = config_root.join(RELATED_COMPONENTS_FILENAME);
    if !path.exists() {
        return Ok(());
    }
    let mut file = load_or_empty(config_root)?;
    if file.rename_component_in_edges(old, new) {
        save_atomic(config_root, &file)?;
    }
    Ok(())
}

/// Canonical read of a plan's `related-plans.md` prose, used by the
/// phase-loop entry points in `main.rs` and `multi_plan.rs` to seed
/// `PlanContext::related_plans` (the `{{RELATED_PLANS}}` macro).
/// Returns an empty string when the file is absent — graceful default.
pub fn read_related_plans_markdown(plan_dir: &Path) -> String {
    std::fs::read_to_string(plan_dir.join("related-plans.md")).unwrap_or_default()
}

// ---------- CLI handlers ----------

pub fn run_list(config_root: &Path, filter: &ListFilter<'_>) -> Result<()> {
    let file = load_or_empty(config_root)?;

    // Plan-derived component filter is resolved once; kind/lifecycle
    // are direct value comparisons against each edge.
    let plan_component = match filter.plan {
        None => None,
        Some(plan) => {
            let catalog = projects::load_or_empty(config_root)?;
            Some(resolve_plan_component_name(&catalog, plan)?)
        }
    };

    let filtered = RelatedComponentsFile {
        schema_version: file.schema_version,
        edges: file
            .edges
            .into_iter()
            .filter(|e| plan_component.as_deref().is_none_or(|name| e.involves(name)))
            .filter(|e| filter.kind.is_none_or(|k| e.kind == k))
            .filter(|e| filter.lifecycle.is_none_or(|l| e.lifecycle == l))
            .collect(),
    };

    let yaml = serde_yaml::to_string(&filtered)
        .context("failed to serialise related-components to YAML")?;
    print!("{yaml}");
    Ok(())
}

/// Add an edge with the full ontology v2 field set supplied by the
/// caller. Validation happens inside `Edge::validate` via
/// `RelatedComponentsFile::add_edge` — non-empty rationale,
/// `evidence_fields` non-empty unless `evidence_grade=weak`, symmetric
/// kinds stored in sorted order, distinct participants.
pub fn run_add_edge(config_root: &Path, req: &AddEdgeRequest<'_>) -> Result<()> {
    let catalog = projects::load_or_empty(config_root)?;
    require_component_known(&catalog, req.a)?;
    require_component_known(&catalog, req.b)?;

    let participants = canonicalise_participants_for_kind(req.kind, req.a, req.b);
    let edge = Edge {
        kind: req.kind,
        lifecycle: req.lifecycle,
        participants,
        evidence_grade: req.evidence_grade,
        evidence_fields: req.evidence_fields.clone(),
        rationale: req.rationale.clone(),
    };

    let mut file = load_or_empty(config_root)?;
    let added = file.add_edge(edge)?;
    if !added {
        eprintln!(
            "edge already present (kind={}, lifecycle={}, {} / {}); no change.",
            req.kind.as_str(),
            req.lifecycle.as_str(),
            req.a,
            req.b
        );
        return Ok(());
    }
    save_atomic(config_root, &file)
}

/// Remove the unique edge matching `(kind, lifecycle, canonicalised
/// participants)`. Errors when the search finds nothing so scripted
/// cleanups don't silently no-op.
pub fn run_remove_edge(
    config_root: &Path,
    kind: EdgeKind,
    lifecycle: LifecycleScope,
    a: &str,
    b: &str,
) -> Result<()> {
    let mut file = load_or_empty(config_root)?;
    let want = canonicalise_participants_for_kind(kind, a, b);
    let before = file.edges.len();
    file.edges.retain(|e| {
        !(e.kind == kind && e.lifecycle == lifecycle && e.participants == want)
    });
    if file.edges.len() == before {
        bail!(
            "no matching edge to remove (kind={}, lifecycle={}, {} / {})",
            kind.as_str(),
            lifecycle.as_str(),
            a,
            b
        );
    }
    save_atomic(config_root, &file)
}

fn canonicalise_participants_for_kind(kind: EdgeKind, a: &str, b: &str) -> Vec<String> {
    let mut v = vec![a.to_string(), b.to_string()];
    if !kind.is_directed() {
        v.sort();
    }
    v
}

fn require_component_known(catalog: &ProjectsCatalog, name: &str) -> Result<()> {
    if catalog.find_by_name(name).is_none() {
        bail!(
            "component '{}' is not in the projects catalog; add it with \
             `ravel-lite state projects add --name {} --path <abs-path>`",
            name,
            name
        );
    }
    Ok(())
}

fn resolve_plan_component_name(catalog: &ProjectsCatalog, plan_dir: &Path) -> Result<String> {
    let project_path = plan_project_path(plan_dir)?;
    if let Some(entry) = catalog.find_by_path(&project_path) {
        return Ok(entry.name.clone());
    }
    bail!(
        "plan's project {} is not in the catalog; run `ravel-lite run` once or add it with `ravel-lite state projects add`",
        project_path.display()
    )
}

/// Derive `<plan>/../..` as the project path, anchored to absolute via
/// `std::path::absolute` (pure path math, matching the catalog's storage
/// convention; avoids `canonicalize`'s symlink-induced `/private/...`
/// drift on macOS).
fn plan_project_path(plan_dir: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(plan_dir).with_context(|| {
        format!(
            "failed to resolve plan dir {} to an absolute path",
            plan_dir.display()
        )
    })?;
    let parent = absolute
        .parent()
        .with_context(|| format!("plan dir {} has no parent", absolute.display()))?;
    let grandparent = parent.parent().with_context(|| {
        format!(
            "plan dir {} has no grandparent (expected <project>/<state-dir>/<plan>)",
            absolute.display()
        )
    })?;
    Ok(grandparent.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_catalog_with(config_root: &Path, names: &[&str]) -> Vec<PathBuf> {
        let mut catalog = ProjectsCatalog::default();
        let mut paths = Vec::new();
        for name in names {
            let p = config_root.join(name);
            std::fs::create_dir_all(&p).unwrap();
            projects::try_add_named(&mut catalog, name, &p).unwrap();
            paths.push(p);
        }
        projects::save_atomic(config_root, &catalog).unwrap();
        paths
    }

    /// Test helper: build a weak-evidence edge with a fixed lifecycle so
    /// unit tests don't replicate the add-edge flag plumbing. Mirrors
    /// what the CLI would produce for `add-edge <kind> <lifecycle> a b
    /// --evidence-grade weak --rationale test`.
    fn weak_edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle,
            participants: canonicalise_participants_for_kind(kind, a, b),
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "test".into(),
        }
    }

    fn req_weak<'a>(
        kind: EdgeKind,
        lifecycle: LifecycleScope,
        a: &'a str,
        b: &'a str,
    ) -> AddEdgeRequest<'a> {
        AddEdgeRequest {
            kind,
            lifecycle,
            a,
            b,
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "test".into(),
        }
    }

    #[test]
    fn load_or_empty_returns_empty_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_empty(tmp.path()).unwrap();
        assert_eq!(file.schema_version, ontology::SCHEMA_VERSION);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedComponentsFile::default();
        file.add_edge(weak_edge(EdgeKind::Generates, LifecycleScope::Codegen, "Alpha", "Beta"))
            .unwrap();
        save_atomic(tmp.path(), &file).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn rename_cascade_rewrites_participants() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedComponentsFile::default();
        file.add_edge(weak_edge(EdgeKind::Generates, LifecycleScope::Codegen, "OldName", "Peer"))
            .unwrap();
        save_atomic(tmp.path(), &file).unwrap();

        rename_component_in_edges(tmp.path(), "OldName", "NewName").unwrap();

        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded.edges[0].participants, vec!["NewName", "Peer"]);
    }

    #[test]
    fn rename_cascade_is_noop_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        rename_component_in_edges(tmp.path(), "Solo", "SoloRenamed").unwrap();
        assert!(!tmp.path().join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_rejects_unknown_component() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Known"]);

        let err = run_add_edge(
            &cfg,
            &req_weak(EdgeKind::Generates, LifecycleScope::Codegen, "Known", "Stranger"),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Stranger"));
        assert!(msg.contains("state projects add"));
        assert!(!cfg.join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_persists_caller_supplied_v2_fields() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["A.produces_files".into(), "B.consumes_files".into()],
            rationale: "A emits schemas B consumes".into(),
        };
        run_add_edge(&cfg, &req).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        let edge = &loaded.edges[0];
        assert_eq!(edge.kind, EdgeKind::Generates);
        assert_eq!(edge.lifecycle, LifecycleScope::Codegen);
        assert_eq!(edge.evidence_grade, EvidenceGrade::Strong);
        assert_eq!(
            edge.evidence_fields,
            vec!["A.produces_files".to_string(), "B.consumes_files".to_string()]
        );
        assert_eq!(edge.rationale, "A emits schemas B consumes");
    }

    #[test]
    fn add_edge_rejects_strong_grade_without_evidence_fields() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "A emits schemas B consumes".into(),
        };
        let err = run_add_edge(&cfg, &req).unwrap_err();
        assert!(format!("{err:#}").contains("evidence_field"));
        assert!(!cfg.join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_rejects_empty_rationale() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "   ".into(),
        };
        let err = run_add_edge(&cfg, &req).unwrap_err();
        assert!(format!("{err:#}").contains("rationale"));
    }

    #[test]
    fn add_edge_is_idempotent_on_directed_kind() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = req_weak(EdgeKind::Generates, LifecycleScope::Codegen, "A", "B");
        run_add_edge(&cfg, &req).unwrap();
        run_add_edge(&cfg, &req).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn add_edge_accepts_same_pair_at_distinct_lifecycles() {
        // §3.5: one pair, one kind, two lifecycles → two edges. The CLI
        // must preserve this — `lifecycle` participates in the dedup key.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Runtime, "A", "B"),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
    }

    #[test]
    fn add_edge_canonicalises_symmetric_participants() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        // Reverse order on a symmetric kind must still dedup with the
        // sorted form.
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Beta", "Alpha"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta"),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(
            loaded.edges[0].participants,
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
    }

    #[test]
    fn remove_edge_errors_when_absent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let err = run_remove_edge(
            &cfg,
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no matching edge"));
    }

    #[test]
    fn remove_edge_matches_only_specified_lifecycle() {
        // Adding a `depends-on` at two lifecycles then removing one must
        // leave the other in place — the lifecycle is a required part
        // of the match key, not a wildcard.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Runtime, "A", "B"),
        )
        .unwrap();

        run_remove_edge(
            &cfg,
            EdgeKind::DependsOn,
            LifecycleScope::Build,
            "A",
            "B",
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0].lifecycle, LifecycleScope::Runtime);
    }

    #[test]
    fn remove_edge_works_on_symmetric_kind_regardless_of_order() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta"),
        )
        .unwrap();
        run_remove_edge(
            &cfg,
            EdgeKind::CoImplements,
            LifecycleScope::Design,
            "Beta",
            "Alpha",
        )
        .unwrap();

        assert!(load_or_empty(&cfg).unwrap().edges.is_empty());
    }

    #[test]
    fn plan_project_path_derives_grandparent() {
        let path = plan_project_path(Path::new("/a/b/c")).unwrap();
        assert_eq!(path, PathBuf::from("/a"));
    }

    #[test]
    fn plan_project_path_resolves_relative_input_against_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let derived = plan_project_path(Path::new("a/b/c")).unwrap();
        let expected = std::path::absolute(cwd.join("a")).unwrap();
        assert_eq!(derived, expected);
    }
}
