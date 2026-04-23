//! Host adapter for the component-ontology graph at
//! `<config_root>/related-components.yaml`.
//!
//! The on-disk types and validation rules live in `crate::ontology` —
//! that module is host-agnostic so it can graduate to a workspace crate.
//! Everything host-specific (the filename, the `<config-root>` join, the
//! per-user `projects.yaml` resolver, the CLI verbs) lives here.
//!
//! Schema is v2: every edge carries `(kind, lifecycle, participants,
//! evidence_grade, evidence_fields, rationale)`. The loader rejects v1
//! files at either the new path (with `schema_version: 1`) or the legacy
//! `related-projects.yaml` filename, and points the user at
//! `discover --apply` for regeneration. There is no in-memory v1 → v2
//! upgrader — the file is a generated artifact, so delete-and-regenerate
//! is the supported upgrade path (`docs/component-ontology.md` §12).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::ontology::{self, Edge, EvidenceGrade, RelatedComponentsFile};
use crate::projects::{self, ProjectsCatalog};

// Re-export the v2 ontology surface that the host needs to construct
// edges through this adapter — the binary crate (`main.rs`) and tests
// route through `crate::related_components::*` rather than touching
// `crate::ontology` directly.
pub use crate::ontology::{EdgeKind, LifecycleScope};

pub const RELATED_COMPONENTS_FILE: &str = "related-components.yaml";
pub const LEGACY_RELATED_PROJECTS_FILE: &str = "related-projects.yaml";

pub fn load_or_empty(config_root: &Path) -> Result<RelatedComponentsFile> {
    error_if_legacy_file_present(config_root)?;
    ontology::load_or_default(&config_root.join(RELATED_COMPONENTS_FILE))
}

pub fn save_atomic(config_root: &Path, file: &RelatedComponentsFile) -> Result<()> {
    ontology::save_atomic(&config_root.join(RELATED_COMPONENTS_FILE), file)
}

/// Cascade for `projects::run_rename`. Loads, rewrites every participant
/// reference, saves. No-op when the file is absent (a catalog without
/// any edge file is valid). Symmetric kinds are re-sorted internally by
/// the ontology layer.
pub fn rename_component_in_edges(config_root: &Path, old: &str, new: &str) -> Result<()> {
    let path = config_root.join(RELATED_COMPONENTS_FILE);
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

/// Hard-error if the legacy v1 file is still present at `config_root`.
/// The two filenames must not coexist, and the user must run the
/// documented upgrade path before any read against the new path can
/// succeed.
fn error_if_legacy_file_present(config_root: &Path) -> Result<()> {
    let legacy = config_root.join(LEGACY_RELATED_PROJECTS_FILE);
    if legacy.exists() {
        bail!(
            "legacy v1 file '{}' is still present. Ravel-Lite has moved to the \
             component-ontology v2 schema; v1 files are not auto-upgraded. \
             Delete '{}' (and '{}/discover-proposals.yaml' if present), then run \
             `ravel-lite state related-components discover --apply` to regenerate \
             v2 edges with direct evidence. See docs/component-ontology.md §12.",
            legacy.display(),
            legacy.display(),
            config_root.display(),
        );
    }
    Ok(())
}

// ---------- CLI handlers ----------

pub fn run_list(config_root: &Path, plan_dir: Option<&Path>) -> Result<()> {
    let file = load_or_empty(config_root)?;
    let filtered = match plan_dir {
        None => file,
        Some(plan) => {
            let catalog = projects::load_or_empty(config_root)?;
            let component_name = resolve_plan_component_name(&catalog, plan)?;
            RelatedComponentsFile {
                schema_version: file.schema_version,
                edges: file
                    .edges
                    .into_iter()
                    .filter(|e| e.involves(&component_name))
                    .collect(),
            }
        }
    };
    let yaml = serde_yaml::to_string(&filtered)
        .context("failed to serialise related-components to YAML")?;
    print!("{yaml}");
    Ok(())
}

/// Add an edge with synthesised defaults — the transitional CLI shape.
/// `kind` selects the v2 vocabulary; the `lifecycle` is chosen by
/// `default_lifecycle_for_kind`, `evidence_grade` is `weak`,
/// `evidence_fields` is empty, and `rationale` is a generic placeholder
/// that names the CLI provenance. The next backlog task replaces this
/// with explicit `--lifecycle` / `--evidence-grade` / `--rationale`
/// flags.
pub fn run_add_edge(config_root: &Path, kind: EdgeKind, a: &str, b: &str) -> Result<()> {
    let catalog = projects::load_or_empty(config_root)?;
    require_component_known(&catalog, a)?;
    require_component_known(&catalog, b)?;

    let participants = canonicalise_participants_for_kind(kind, a, b);
    let edge = Edge {
        kind,
        lifecycle: default_lifecycle_for_kind(kind),
        participants,
        evidence_grade: EvidenceGrade::Weak,
        evidence_fields: Vec::new(),
        rationale: format!(
            "added via `ravel-lite state related-components add-edge {} {} {}`; \
             lifecycle / evidence-grade / rationale flags pending v2 CLI extension",
            kind.as_str(),
            a,
            b,
        ),
    };

    let mut file = load_or_empty(config_root)?;
    let added = file.add_edge(edge)?;
    if !added {
        eprintln!(
            "edge already present (kind={}, lifecycle={}, {} / {}); no change.",
            kind.as_str(),
            default_lifecycle_for_kind(kind).as_str(),
            a,
            b
        );
        return Ok(());
    }
    save_atomic(config_root, &file)
}

/// Remove every edge matching `(kind, canonicalised participants)` —
/// across all lifecycles — until the next task's `--lifecycle` flag
/// makes lifecycle-targeted removal possible. Errors when the search
/// finds nothing so scripted cleanups don't silently no-op.
pub fn run_remove_edge(config_root: &Path, kind: EdgeKind, a: &str, b: &str) -> Result<()> {
    let mut file = load_or_empty(config_root)?;
    let want = canonicalise_participants_for_kind(kind, a, b);
    let before = file.edges.len();
    file.edges
        .retain(|e| !(e.kind == kind && e.participants == want));
    if file.edges.len() == before {
        bail!(
            "no matching edge to remove (kind={}, {} / {})",
            kind.as_str(),
            a,
            b
        );
    }
    save_atomic(config_root, &file)
}

/// Default lifecycle per kind, drawn from the typical-lifecycle column of
/// `docs/component-ontology.md` §5. When a kind has more than one
/// canonical lifecycle, the one most representative of the kind's
/// primary use is chosen. Only consulted by the transitional CLI
/// `add-edge`; once explicit `--lifecycle` lands this helper goes away.
fn default_lifecycle_for_kind(kind: EdgeKind) -> LifecycleScope {
    match kind {
        EdgeKind::DependsOn => LifecycleScope::Build,
        EdgeKind::HasOptionalDependency => LifecycleScope::Build,
        EdgeKind::ProvidedByHost => LifecycleScope::Runtime,
        EdgeKind::LinksStatically => LifecycleScope::Build,
        EdgeKind::LinksDynamically => LifecycleScope::Runtime,
        EdgeKind::Generates => LifecycleScope::Codegen,
        EdgeKind::Scaffolds => LifecycleScope::DevWorkflow,
        EdgeKind::CommunicatesWith => LifecycleScope::Runtime,
        EdgeKind::Calls => LifecycleScope::Runtime,
        EdgeKind::Invokes => LifecycleScope::DevWorkflow,
        EdgeKind::Orchestrates => LifecycleScope::DevWorkflow,
        EdgeKind::Embeds => LifecycleScope::Runtime,
        EdgeKind::Tests => LifecycleScope::Test,
        EdgeKind::ProvidesFixturesFor => LifecycleScope::Test,
        EdgeKind::ConformsTo => LifecycleScope::Design,
        EdgeKind::CoImplements => LifecycleScope::Design,
        EdgeKind::Describes => LifecycleScope::Design,
    }
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

    fn cli_edge(kind: EdgeKind, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle: default_lifecycle_for_kind(kind),
            participants: canonicalise_participants_for_kind(kind, a, b),
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
        file.add_edge(cli_edge(EdgeKind::Generates, "Alpha", "Beta"))
            .unwrap();
        save_atomic(tmp.path(), &file).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn load_rejects_legacy_filename() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(LEGACY_RELATED_PROJECTS_FILE),
            "schema_version: 1\nedges: []\n",
        )
        .unwrap();
        let err = load_or_empty(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("legacy v1 file"),
            "error must name the legacy file: {msg}"
        );
        assert!(
            msg.contains("discover --apply"),
            "error must point at the regenerate command: {msg}"
        );
    }

    #[test]
    fn rename_cascade_rewrites_participants() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedComponentsFile::default();
        file.add_edge(cli_edge(EdgeKind::Generates, "OldName", "Peer"))
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
        assert!(!tmp.path().join(RELATED_COMPONENTS_FILE).exists());
    }

    #[test]
    fn add_edge_rejects_unknown_component() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Known"]);

        let err = run_add_edge(&cfg, EdgeKind::Generates, "Known", "Stranger").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Stranger"));
        assert!(msg.contains("state projects add"));
        assert!(!cfg.join(RELATED_COMPONENTS_FILE).exists());
    }

    #[test]
    fn add_edge_synthesises_v2_defaults_and_persists() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(&cfg, EdgeKind::Generates, "A", "B").unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        let edge = &loaded.edges[0];
        assert_eq!(edge.kind, EdgeKind::Generates);
        assert_eq!(edge.lifecycle, LifecycleScope::Codegen);
        assert_eq!(edge.evidence_grade, EvidenceGrade::Weak);
        assert!(edge.evidence_fields.is_empty());
        assert!(edge.rationale.contains("added via"));
    }

    #[test]
    fn add_edge_is_idempotent_on_directed_kind() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(&cfg, EdgeKind::Generates, "A", "B").unwrap();
        run_add_edge(&cfg, EdgeKind::Generates, "A", "B").unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn add_edge_canonicalises_symmetric_participants() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        // Reverse order on a symmetric kind must still dedup with the
        // sorted form.
        run_add_edge(&cfg, EdgeKind::CoImplements, "Beta", "Alpha").unwrap();
        run_add_edge(&cfg, EdgeKind::CoImplements, "Alpha", "Beta").unwrap();

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
        let err = run_remove_edge(&cfg, EdgeKind::Generates, "A", "B").unwrap_err();
        assert!(format!("{err:#}").contains("no matching edge"));
    }

    #[test]
    fn remove_edge_works_on_symmetric_kind_regardless_of_order() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        run_add_edge(&cfg, EdgeKind::CoImplements, "Alpha", "Beta").unwrap();
        run_remove_edge(&cfg, EdgeKind::CoImplements, "Beta", "Alpha").unwrap();

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
