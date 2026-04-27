//! Per-user projects catalog: `<config_root>/projects.yaml`.
//!
//! Maps project names to project directories. Shared-between-users edge
//! lists (R5) reference projects by name; this catalog is the per-user
//! name → path resolver.
//!
//! ## Path representation
//!
//! In memory, `ProjectEntry.path` is always an absolute, cleaned
//! `PathBuf`. Consumers may use it directly for filesystem operations.
//!
//! On disk, paths are written **relative to the directory containing
//! `projects.yaml`** (i.e. `<config_root>`). This makes `projects.yaml`
//! portable: the same file can be committed to a shared repository and
//! loaded correctly from different `<config_root>` absolute paths on
//! different machines, provided the project layout relative to the
//! catalog is the same.
//!
//! Loading accepts both relative and absolute path values for
//! backwards compatibility with older all-absolute catalogs. The first
//! state-mutating verb on a legacy catalog rewrites every entry to the
//! relative form — semantics are unchanged, only the on-disk shape.
//!
//! All read/write goes through `load_or_empty` / `save_atomic` so the
//! single `schema_version` field is always applied correctly and every
//! write is tmp-file-plus-rename.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};

pub const CATALOG_FILE: &str = "projects.yaml";

/// Only schema version in circulation; bump when the on-disk shape
/// changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectsCatalog {
    pub schema_version: u32,
    pub projects: Vec<ProjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub name: String,
    pub path: PathBuf,
}

/// On-disk shape: `path` may be absolute or relative to the directory
/// containing `projects.yaml`. The in-memory `ProjectEntry` is always
/// absolute; conversion happens in `load_or_empty` / `save_atomic`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawProjectsCatalog {
    schema_version: u32,
    #[serde(default)]
    projects: Vec<RawProjectEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawProjectEntry {
    name: String,
    path: PathBuf,
}

impl Default for ProjectsCatalog {
    fn default() -> Self {
        ProjectsCatalog {
            schema_version: SCHEMA_VERSION,
            projects: Vec::new(),
        }
    }
}

impl ProjectsCatalog {
    pub fn find_by_name(&self, name: &str) -> Option<&ProjectEntry> {
        self.projects.iter().find(|p| p.name == name)
    }

    pub fn find_by_path(&self, path: &Path) -> Option<&ProjectEntry> {
        let cleaned = path.to_path_buf().clean();
        self.projects.iter().find(|p| p.path == cleaned)
    }
}

/// Outcome of attempting to add a project by its directory basename.
/// Pure-logic: does not perform I/O or prompt. The caller decides what
/// to do on `NameCollision` (typically prompt and retry with a chosen
/// name via `try_add_named`).
#[derive(Debug, PartialEq, Eq)]
pub enum AutoAddOutcome {
    /// The path is already catalogued; `name` is the existing entry.
    AlreadyCatalogued { name: String },
    /// Added to the in-memory catalog under `name`. Caller must persist.
    Added { name: String },
    /// The basename is already used by a different path. Caller must
    /// resolve (e.g. prompt for an alternative name, then call
    /// `try_add_named`).
    NameCollision {
        attempted_name: String,
        existing_path: PathBuf,
    },
}

/// Extract a project's directory basename as a usable name. Used by
/// `auto_add` and by `run_add` when no explicit `--name` is supplied.
fn basename_as_name(project_path: &Path) -> Result<String> {
    project_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .with_context(|| {
            format!(
                "project path {} has no directory basename usable as a project name",
                project_path.display()
            )
        })
}

/// Attempt to add `project_path` to `catalog` using its directory
/// basename as the default name. Does not persist; caller saves on
/// `Added`.
pub fn auto_add(catalog: &mut ProjectsCatalog, project_path: &Path) -> Result<AutoAddOutcome> {
    let name = basename_as_name(project_path)?;

    if let Some(existing) = catalog.find_by_path(project_path) {
        return Ok(AutoAddOutcome::AlreadyCatalogued {
            name: existing.name.clone(),
        });
    }
    if let Some(existing) = catalog.find_by_name(&name) {
        return Ok(AutoAddOutcome::NameCollision {
            attempted_name: name,
            existing_path: existing.path.clone(),
        });
    }

    catalog.projects.push(ProjectEntry {
        name: name.clone(),
        path: project_path.to_path_buf().clean(),
    });
    Ok(AutoAddOutcome::Added { name })
}

/// Add `project_path` under an explicit `name`. Errors if the name
/// collides with a different path, or the path is already catalogued
/// under a different name. Does not persist; caller saves on `Ok`.
pub fn try_add_named(
    catalog: &mut ProjectsCatalog,
    name: &str,
    project_path: &Path,
) -> Result<()> {
    if let Some(existing) = catalog.find_by_path(project_path) {
        if existing.name == name {
            return Ok(());
        }
        bail!(
            "project path {} is already catalogued under name '{}'; refusing to re-add as '{}'",
            project_path.display(),
            existing.name,
            name
        );
    }
    if let Some(existing) = catalog.find_by_name(name) {
        bail!(
            "project name '{}' is already in use for path {}; pick a different name",
            name,
            existing.path.display()
        );
    }
    catalog.projects.push(ProjectEntry {
        name: name.to_string(),
        path: project_path.to_path_buf().clean(),
    });
    Ok(())
}

pub fn load_or_empty(config_root: &Path) -> Result<ProjectsCatalog> {
    let path = config_root.join(CATALOG_FILE);
    if !path.exists() {
        return Ok(ProjectsCatalog::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let raw: RawProjectsCatalog = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    if raw.schema_version != SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}; aborting to avoid data loss",
            path.display(),
            raw.schema_version,
            SCHEMA_VERSION
        );
    }
    let projects = raw
        .projects
        .into_iter()
        .map(|raw_entry| ProjectEntry {
            name: raw_entry.name,
            path: resolve_against_catalog_dir(&raw_entry.path, config_root),
        })
        .collect();
    Ok(ProjectsCatalog {
        schema_version: raw.schema_version,
        projects,
    })
}

pub fn save_atomic(config_root: &Path, catalog: &ProjectsCatalog) -> Result<()> {
    let path = config_root.join(CATALOG_FILE);
    let yaml = serialise_catalog(catalog, config_root)?;
    let tmp = config_root.join(format!(".{CATALOG_FILE}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Serialise the in-memory catalog into the on-disk YAML form, with
/// every path expressed relative to `<config_root>`. Shared by
/// `save_atomic` and `run_list` so the list command prints the exact
/// shape that is on (or would be on) disk.
fn serialise_catalog(catalog: &ProjectsCatalog, config_root: &Path) -> Result<String> {
    let projects = catalog
        .projects
        .iter()
        .map(|entry| {
            let on_disk = relativise_against_catalog_dir(&entry.path, config_root)?;
            Ok(RawProjectEntry {
                name: entry.name.clone(),
                path: on_disk,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let raw = RawProjectsCatalog {
        schema_version: catalog.schema_version,
        projects,
    };
    serde_yaml::to_string(&raw).context("Failed to serialise projects catalog to YAML")
}

/// Resolve a stored path against `<config_root>` and clean. Absolute
/// stored paths (legacy catalogs, or any future explicit absolute) are
/// returned cleaned but otherwise unchanged.
fn resolve_against_catalog_dir(stored: &Path, config_root: &Path) -> PathBuf {
    if stored.is_absolute() {
        stored.to_path_buf().clean()
    } else {
        config_root.join(stored).clean()
    }
}

/// Compute the on-disk relative form of an in-memory absolute path.
/// On Unix two absolutes always have a relative form (worst case
/// climbs to `/` then descends), so failure here implies a non-absolute
/// in-memory path — a contract violation worth surfacing as an error.
fn relativise_against_catalog_dir(absolute: &Path, config_root: &Path) -> Result<PathBuf> {
    pathdiff::diff_paths(absolute, config_root).with_context(|| {
        format!(
            "cannot compute relative path from {} to catalog dir {}",
            absolute.display(),
            config_root.display()
        )
    })
}

// ---------- CLI handlers ----------

pub fn run_list(config_root: &Path) -> Result<()> {
    let catalog = load_or_empty(config_root)?;
    let yaml = serialise_catalog(&catalog, config_root)?;
    print!("{yaml}");
    Ok(())
}

/// Add a project to the catalog. `project_path` may be relative — it is
/// resolved against the current working directory via `std::path::absolute`
/// (pure path math; no disk access, no symlink resolution) and then
/// cleaned (`.` / `..` segments collapsed). `name` is optional; when
/// `None`, the path's basename is used.
pub fn run_add(config_root: &Path, name: Option<&str>, project_path: &Path) -> Result<()> {
    let absolute_path = std::path::absolute(project_path)
        .with_context(|| {
            format!(
                "Failed to resolve project path {} to an absolute path",
                project_path.display()
            )
        })?
        .clean();
    let resolved_name = match name {
        Some(n) => n.to_string(),
        None => basename_as_name(&absolute_path)?,
    };
    let mut catalog = load_or_empty(config_root)?;
    try_add_named(&mut catalog, &resolved_name, &absolute_path)?;
    save_atomic(config_root, &catalog)
}

pub fn run_remove(config_root: &Path, name: &str) -> Result<()> {
    let mut catalog = load_or_empty(config_root)?;
    let before = catalog.projects.len();
    catalog.projects.retain(|p| p.name != name);
    if catalog.projects.len() == before {
        bail!("no project named '{}' in catalog at {}", name, config_root.join(CATALOG_FILE).display());
    }
    save_atomic(config_root, &catalog)
}

pub fn run_rename(config_root: &Path, old: &str, new: &str) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let mut catalog = load_or_empty(config_root)?;
    if catalog.find_by_name(new).is_some() {
        bail!("cannot rename to '{}': name already in use", new);
    }
    let entry = catalog
        .projects
        .iter_mut()
        .find(|p| p.name == old)
        .with_context(|| format!("no project named '{old}' in catalog"))?;
    entry.name = new.to_string();
    save_atomic(config_root, &catalog)?;
    crate::related_components::rename_component_in_edges(config_root, old, new)?;
    crate::discover::cache::rename(config_root, old, new)
}

/// Ensure `project_path` is catalogued. Pure-logic path returns the
/// resolved name on success; on basename collision, prompts on
/// `prompt_out`/`prompt_in` for an alternative name and retries. Empty
/// input at any prompt aborts with an actionable error.
///
/// Callers in non-interactive contexts (e.g. `state projects` CLI
/// verbs) should not use this — they should call `auto_add` or
/// `try_add_named` directly and surface `NameCollision` as an error.
pub fn ensure_in_catalog_interactive<R: std::io::BufRead, W: std::io::Write>(
    config_root: &Path,
    project_path: &Path,
    prompt_out: &mut W,
    prompt_in: &mut R,
) -> Result<String> {
    let mut catalog = load_or_empty(config_root)?;
    match auto_add(&mut catalog, project_path)? {
        AutoAddOutcome::AlreadyCatalogued { name } => Ok(name),
        AutoAddOutcome::Added { name } => {
            save_atomic(config_root, &catalog)?;
            Ok(name)
        }
        AutoAddOutcome::NameCollision {
            attempted_name,
            existing_path,
        } => {
            let resolved_name = prompt_for_alternative_name(
                &catalog,
                &attempted_name,
                &existing_path,
                project_path,
                prompt_out,
                prompt_in,
            )?;
            try_add_named(&mut catalog, &resolved_name, project_path)?;
            save_atomic(config_root, &catalog)?;
            Ok(resolved_name)
        }
    }
}

fn prompt_for_alternative_name<R: std::io::BufRead, W: std::io::Write>(
    catalog: &ProjectsCatalog,
    attempted_name: &str,
    existing_path: &Path,
    project_path: &Path,
    out: &mut W,
    input: &mut R,
) -> Result<String> {
    writeln!(
        out,
        "project name '{attempted_name}' is already used for a different path:\n  \
         existing: {}\n  adding:   {}\n\
         enter a different name (blank to abort): ",
        existing_path.display(),
        project_path.display()
    )?;
    out.flush()?;

    let mut line = String::new();
    input
        .read_line(&mut line)
        .context("failed to read name from stdin")?;
    let chosen = line.trim().to_string();
    if chosen.is_empty() {
        bail!(
            "project catalog add aborted by user; resolve manually with `ravel-lite state projects add --name <name> --path {}`",
            project_path.display()
        );
    }
    if catalog.find_by_name(&chosen).is_some() {
        bail!(
            "name '{chosen}' is also already in use; resolve manually with `ravel-lite state projects add --name <name> --path {}`",
            project_path.display()
        );
    }
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_project_dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn load_or_empty_returns_empty_catalog_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let catalog = load_or_empty(tmp.path()).unwrap();
        assert_eq!(catalog.schema_version, SCHEMA_VERSION);
        assert!(catalog.projects.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let project_path = mk_project_dir(tmp.path(), "some-proj");
        let catalog = ProjectsCatalog {
            schema_version: SCHEMA_VERSION,
            projects: vec![ProjectEntry {
                name: "some-proj".to_string(),
                path: project_path.clone(),
            }],
        };
        save_atomic(tmp.path(), &catalog).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, catalog);
    }

    #[test]
    fn load_rejects_unknown_schema_version() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(CATALOG_FILE),
            "schema_version: 99\nprojects: []\n",
        )
        .unwrap();
        let err = load_or_empty(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"));
        assert!(msg.contains("99"));
    }

    #[test]
    fn auto_add_adds_under_basename_when_empty() {
        let tmp = TempDir::new().unwrap();
        let project = mk_project_dir(tmp.path(), "Ravel-Lite");
        let mut catalog = ProjectsCatalog::default();

        let outcome = auto_add(&mut catalog, &project).unwrap();

        assert_eq!(
            outcome,
            AutoAddOutcome::Added {
                name: "Ravel-Lite".to_string()
            }
        );
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.projects[0].name, "Ravel-Lite");
        assert_eq!(catalog.projects[0].path, project);
    }

    #[test]
    fn auto_add_is_idempotent_by_path() {
        let tmp = TempDir::new().unwrap();
        let project = mk_project_dir(tmp.path(), "my-proj");
        let mut catalog = ProjectsCatalog::default();

        auto_add(&mut catalog, &project).unwrap();
        let outcome = auto_add(&mut catalog, &project).unwrap();

        assert_eq!(
            outcome,
            AutoAddOutcome::AlreadyCatalogued {
                name: "my-proj".to_string()
            }
        );
        assert_eq!(catalog.projects.len(), 1);
    }

    #[test]
    fn auto_add_reports_name_collision_for_same_basename_different_path() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        let project_a = mk_project_dir(tmp_a.path(), "shared-name");
        let project_b = mk_project_dir(tmp_b.path(), "shared-name");

        let mut catalog = ProjectsCatalog::default();
        auto_add(&mut catalog, &project_a).unwrap();
        let outcome = auto_add(&mut catalog, &project_b).unwrap();

        assert_eq!(
            outcome,
            AutoAddOutcome::NameCollision {
                attempted_name: "shared-name".to_string(),
                existing_path: project_a,
            }
        );
        assert_eq!(catalog.projects.len(), 1, "collision must not mutate catalog");
    }

    #[test]
    fn try_add_named_rejects_duplicate_name() {
        let tmp = TempDir::new().unwrap();
        let project_a = mk_project_dir(tmp.path(), "a");
        let project_b = mk_project_dir(tmp.path(), "b");

        let mut catalog = ProjectsCatalog::default();
        try_add_named(&mut catalog, "taken", &project_a).unwrap();
        let err = try_add_named(&mut catalog, "taken", &project_b).unwrap_err();
        assert!(format!("{err:#}").contains("already in use"));
    }

    #[test]
    fn try_add_named_is_noop_when_exact_same_entry_already_present() {
        let tmp = TempDir::new().unwrap();
        let project = mk_project_dir(tmp.path(), "a");
        let mut catalog = ProjectsCatalog::default();

        try_add_named(&mut catalog, "a", &project).unwrap();
        try_add_named(&mut catalog, "a", &project).unwrap();

        assert_eq!(catalog.projects.len(), 1);
    }

    #[test]
    fn try_add_named_rejects_same_path_under_different_name() {
        let tmp = TempDir::new().unwrap();
        let project = mk_project_dir(tmp.path(), "a");
        let mut catalog = ProjectsCatalog::default();

        try_add_named(&mut catalog, "a", &project).unwrap();
        let err = try_add_named(&mut catalog, "b", &project).unwrap_err();
        assert!(format!("{err:#}").contains("already catalogued"));
    }

    #[test]
    fn run_add_persists_to_disk() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "abs-proj");

        run_add(&cfg, Some("abs-proj"), &project).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "abs-proj");
    }

    #[test]
    fn run_add_canonicalises_relative_path_against_cwd() {
        // `std::path::absolute` resolves relative paths against the
        // process CWD. Test CWDs are unstable when running in parallel,
        // so we assert on the canonicalisation property rather than a
        // specific resolved path: stored path must be absolute.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        run_add(&cfg, Some("rel-proj"), Path::new("some/relative/path")).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert!(
            loaded.projects[0].path.is_absolute(),
            "stored path must be absolute, got {}",
            loaded.projects[0].path.display()
        );
        assert!(
            loaded.projects[0].path.ends_with("some/relative/path"),
            "absolute path must end with the relative input, got {}",
            loaded.projects[0].path.display()
        );
    }

    #[test]
    fn run_add_with_no_name_defaults_to_basename() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "SomeProject");

        run_add(&cfg, None, &project).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "SomeProject");
        assert_eq!(loaded.projects[0].path, project);
    }

    #[test]
    fn run_add_with_no_name_and_relative_path_derives_basename_from_resolved_path() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        run_add(&cfg, None, Path::new("parent/BasenameTarget")).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "BasenameTarget");
    }

    #[test]
    fn run_remove_deletes_entry() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "target");

        run_add(&cfg, Some("target"), &project).unwrap();
        run_remove(&cfg, "target").unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert!(loaded.projects.is_empty());
    }

    #[test]
    fn run_remove_unknown_name_errors() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        let err = run_remove(&cfg, "nope").unwrap_err();
        assert!(format!("{err:#}").contains("nope"));
    }

    #[test]
    fn run_rename_updates_entry() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "old-name");

        run_add(&cfg, Some("old-name"), &project).unwrap();
        run_rename(&cfg, "old-name", "new-name").unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects[0].name, "new-name");
        assert_eq!(loaded.projects[0].path, project);
    }

    #[test]
    fn run_rename_rejects_when_new_name_taken() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let a = mk_project_dir(tmp.path(), "a");
        let b = mk_project_dir(tmp.path(), "b");

        run_add(&cfg, Some("a"), &a).unwrap();
        run_add(&cfg, Some("b"), &b).unwrap();

        let err = run_rename(&cfg, "a", "b").unwrap_err();
        assert!(format!("{err:#}").contains("already in use"));
        // Catalog unchanged.
        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 2);
        assert!(loaded.find_by_name("a").is_some());
        assert!(loaded.find_by_name("b").is_some());
    }

    #[test]
    fn run_rename_cascades_into_symmetric_v2_edges() {
        use crate::ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
        use crate::related_components;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let a = mk_project_dir(tmp.path(), "OldName");
        let b = mk_project_dir(tmp.path(), "Other");
        run_add(&cfg, Some("OldName"), &a).unwrap();
        run_add(&cfg, Some("Other"), &b).unwrap();

        // Symmetric kind: participants must be sorted on construction.
        let mut file = crate::ontology::RelatedComponentsFile::default();
        file.add_edge(Edge {
            kind: EdgeKind::CoImplements,
            lifecycle: LifecycleScope::Design,
            participants: vec!["OldName".to_string(), "Other".to_string()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["OldName.purpose".into()],
            rationale: "shared spec".into(),
        })
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        run_rename(&cfg, "OldName", "NewName").unwrap();

        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0].kind, EdgeKind::CoImplements);
        // After rename to NewName, sort order is (NewName, Other).
        assert_eq!(
            loaded.edges[0].participants,
            vec!["NewName".to_string(), "Other".to_string()],
            "symmetric participants must be re-sorted after rename"
        );
    }

    #[test]
    fn run_rename_cascade_preserves_directed_kind_order() {
        use crate::ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
        use crate::related_components;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let parent = mk_project_dir(tmp.path(), "Parent");
        let child = mk_project_dir(tmp.path(), "Child");
        run_add(&cfg, Some("Parent"), &parent).unwrap();
        run_add(&cfg, Some("Child"), &child).unwrap();

        // Directed kind: order is semantic (Parent first as generator).
        let mut file = crate::ontology::RelatedComponentsFile::default();
        file.add_edge(Edge {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            participants: vec!["Parent".to_string(), "Child".to_string()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Parent.produces_files".into()],
            rationale: "generates schemas Child consumes".into(),
        })
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        run_rename(&cfg, "Parent", "NewParent").unwrap();

        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0].kind, EdgeKind::Generates);
        assert_eq!(
            loaded.edges[0].participants,
            vec!["NewParent".to_string(), "Child".to_string()],
            "directed kind order must be preserved across rename"
        );
    }

    #[test]
    fn run_rename_cascade_leaves_uninvolved_edges_untouched() {
        use crate::ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
        use crate::related_components;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let a = mk_project_dir(tmp.path(), "Alpha");
        let b = mk_project_dir(tmp.path(), "Beta");
        let c = mk_project_dir(tmp.path(), "Gamma");
        run_add(&cfg, Some("Alpha"), &a).unwrap();
        run_add(&cfg, Some("Beta"), &b).unwrap();
        run_add(&cfg, Some("Gamma"), &c).unwrap();

        let mut file = crate::ontology::RelatedComponentsFile::default();
        file.add_edge(Edge {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            participants: vec!["Beta".to_string(), "Gamma".to_string()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["Beta.produces_files".into()],
            rationale: "test".into(),
        })
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        run_rename(&cfg, "Alpha", "AlphaRenamed").unwrap();

        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert!(loaded.edges[0].participants.contains(&"Beta".to_string()));
        assert!(loaded.edges[0].participants.contains(&"Gamma".to_string()));
    }

    #[test]
    fn run_rename_cascade_is_noop_when_yaml_absent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let a = mk_project_dir(tmp.path(), "Solo");
        run_add(&cfg, Some("Solo"), &a).unwrap();

        // No related-components.yaml at all: rename must succeed.
        run_rename(&cfg, "Solo", "SoloRenamed").unwrap();

        assert!(
            !cfg.join(crate::state::filenames::RELATED_COMPONENTS_FILENAME).exists(),
            "cascade must not create the file when it wasn't there to begin with"
        );
    }

    #[test]
    fn run_rename_cascades_into_discover_cache() {
        use crate::discover::cache;
        use crate::discover::schema::{SurfaceFile, SurfaceRecord, SURFACE_SCHEMA_VERSION};

        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "OldName");
        run_add(&cfg, Some("OldName"), &project).unwrap();

        let surface = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "OldName".to_string(),
            tree_sha: "abc".to_string(),
            dirty_hash: String::new(),
            analysed_at: "2026-04-22T00:00:00Z".to_string(),
            surface: SurfaceRecord::default(),
        };
        cache::save_atomic(&cfg, &surface).unwrap();
        assert!(cache::cache_path(&cfg, "OldName").exists());

        run_rename(&cfg, "OldName", "NewName").unwrap();

        assert!(!cache::cache_path(&cfg, "OldName").exists());
        assert!(cache::cache_path(&cfg, "NewName").exists());
    }

    #[test]
    fn run_rename_cascade_is_noop_when_no_cache_file() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "Uncached");
        run_add(&cfg, Some("Uncached"), &project).unwrap();

        // No cache file: rename must still succeed.
        run_rename(&cfg, "Uncached", "UncachedRenamed").unwrap();

        assert!(!crate::discover::cache::cache_dir(&cfg).exists(),
            "cascade must not create the cache dir when no cache file existed");
    }

    #[test]
    fn ensure_in_catalog_interactive_no_collision_happy_path() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "auto");

        let mut out = Vec::<u8>::new();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());

        let name = ensure_in_catalog_interactive(&cfg, &project, &mut out, &mut input).unwrap();
        assert_eq!(name, "auto");

        // Second call on same path is a no-op; no prompt emitted.
        let mut out2 = Vec::<u8>::new();
        let name2 = ensure_in_catalog_interactive(&cfg, &project, &mut out2, &mut input).unwrap();
        assert_eq!(name2, "auto");
        assert!(out2.is_empty(), "idempotent path must not prompt");
    }

    #[test]
    fn ensure_in_catalog_interactive_collision_prompts_for_new_name() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        let cfg = tmp_a.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project_a = mk_project_dir(tmp_a.path(), "collide");
        let project_b = mk_project_dir(tmp_b.path(), "collide");

        // Pre-seed the collision.
        run_add(&cfg, Some("collide"), &project_a).unwrap();

        let mut out = Vec::<u8>::new();
        let mut input = std::io::Cursor::new(b"collide-two\n".to_vec());

        let name =
            ensure_in_catalog_interactive(&cfg, &project_b, &mut out, &mut input).unwrap();

        assert_eq!(name, "collide-two");
        let prompt = String::from_utf8(out).unwrap();
        assert!(prompt.contains("already used"));
        assert!(prompt.contains("collide"));

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 2);
        assert_eq!(loaded.find_by_path(&project_b).unwrap().name, "collide-two");
    }

    #[test]
    fn ensure_in_catalog_interactive_collision_blank_input_aborts() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        let cfg = tmp_a.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project_a = mk_project_dir(tmp_a.path(), "collide");
        let project_b = mk_project_dir(tmp_b.path(), "collide");
        run_add(&cfg, Some("collide"), &project_a).unwrap();

        let mut out = Vec::<u8>::new();
        let mut input = std::io::Cursor::new(b"\n".to_vec());

        let err =
            ensure_in_catalog_interactive(&cfg, &project_b, &mut out, &mut input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("aborted"));
        assert!(msg.contains("state projects add"));

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.projects.len(), 1, "abort must not mutate catalog");
    }

    // ---------- relative-path catalog tests ----------

    /// Read the on-disk YAML and return the `path` field of the first
    /// entry as it appears literally — used to assert on the on-disk
    /// shape, not the in-memory resolved form.
    fn first_stored_path_string(cfg: &Path) -> String {
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(cfg.join(CATALOG_FILE)).unwrap())
                .unwrap();
        yaml["projects"][0]["path"]
            .as_str()
            .expect("first entry must have a string path")
            .to_string()
    }

    #[test]
    fn save_writes_path_relative_to_catalog_dir_when_project_under_root() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        // Project nested under cfg so the relative form is a simple basename.
        let project = mk_project_dir(&cfg, "nested-proj");

        run_add(&cfg, Some("nested-proj"), &project).unwrap();

        let stored = first_stored_path_string(&cfg);
        assert_eq!(
            stored, "nested-proj",
            "project under catalog dir must serialise as a bare relative segment"
        );
    }

    #[test]
    fn save_writes_path_with_parent_segments_when_project_outside_root() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        // Project as a sibling of cfg — relative form must climb out.
        let project = mk_project_dir(tmp.path(), "sibling-proj");

        run_add(&cfg, Some("sibling-proj"), &project).unwrap();

        let stored = first_stored_path_string(&cfg);
        assert_eq!(
            stored, "../sibling-proj",
            "sibling project must serialise with one parent segment"
        );
    }

    #[test]
    fn load_resolves_relative_stored_path_against_catalog_dir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        // Hand-craft a relative-form yaml as if checked in from another machine.
        std::fs::write(
            cfg.join(CATALOG_FILE),
            "schema_version: 1\nprojects:\n- name: hand-written\n  path: ./local-proj\n",
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();

        assert_eq!(loaded.projects.len(), 1);
        assert!(
            loaded.projects[0].path.is_absolute(),
            "in-memory path must be absolute, got {}",
            loaded.projects[0].path.display()
        );
        assert_eq!(loaded.projects[0].path, cfg.join("local-proj"));
    }

    #[test]
    fn load_accepts_legacy_absolute_stored_paths() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "legacy-abs");
        // Hand-craft a legacy all-absolute yaml.
        std::fs::write(
            cfg.join(CATALOG_FILE),
            format!(
                "schema_version: 1\nprojects:\n- name: legacy-abs\n  path: {}\n",
                project.display()
            ),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();

        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].path, project);
    }

    #[test]
    fn save_after_load_migrates_legacy_absolute_entries_to_relative() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "legacy");
        // Seed a legacy absolute-form catalog.
        std::fs::write(
            cfg.join(CATALOG_FILE),
            format!(
                "schema_version: 1\nprojects:\n- name: legacy\n  path: {}\n",
                project.display()
            ),
        )
        .unwrap();

        // Load + add a sibling entry to trigger a save.
        let other = mk_project_dir(tmp.path(), "other");
        run_add(&cfg, Some("other"), &other).unwrap();

        // Both entries should now be relative on disk.
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(cfg.join(CATALOG_FILE)).unwrap())
                .unwrap();
        for entry in yaml["projects"].as_sequence().unwrap() {
            let p = entry["path"].as_str().unwrap();
            assert!(
                !p.starts_with('/'),
                "after save, every entry must be relative, got {p}"
            );
        }
    }

    #[test]
    fn same_yaml_loaded_from_two_locations_yields_distinct_absolute_paths() {
        // The portability proof: a single committed projects.yaml,
        // copied verbatim into two different config roots, resolves to
        // each user's local absolute path.
        let yaml_body =
            "schema_version: 1\nprojects:\n- name: shared\n  path: ../shared-proj\n";

        let user_a = TempDir::new().unwrap();
        let user_b = TempDir::new().unwrap();
        let cfg_a = user_a.path().join("cfg");
        let cfg_b = user_b.path().join("cfg");
        std::fs::create_dir_all(&cfg_a).unwrap();
        std::fs::create_dir_all(&cfg_b).unwrap();
        std::fs::write(cfg_a.join(CATALOG_FILE), yaml_body).unwrap();
        std::fs::write(cfg_b.join(CATALOG_FILE), yaml_body).unwrap();

        let loaded_a = load_or_empty(&cfg_a).unwrap();
        let loaded_b = load_or_empty(&cfg_b).unwrap();

        assert_eq!(loaded_a.projects[0].name, "shared");
        assert_eq!(loaded_b.projects[0].name, "shared");
        assert_ne!(
            loaded_a.projects[0].path, loaded_b.projects[0].path,
            "different config roots must produce different absolute paths"
        );
        assert_eq!(loaded_a.projects[0].path, user_a.path().join("shared-proj"));
        assert_eq!(loaded_b.projects[0].path, user_b.path().join("shared-proj"));
    }

    #[test]
    fn find_by_path_matches_unclean_search_argument() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let project = mk_project_dir(tmp.path(), "target");
        run_add(&cfg, Some("target"), &project).unwrap();
        let catalog = load_or_empty(&cfg).unwrap();

        // Reach the same dir via a path that contains a `..` segment.
        let detour = tmp.path().join("target").join("..").join("target");
        let found = catalog.find_by_path(&detour);

        assert!(
            found.is_some(),
            "find_by_path must clean its argument before comparing"
        );
        assert_eq!(found.unwrap().name, "target");
    }
}
