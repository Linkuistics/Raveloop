//! Merge `discover-proposals.yaml` into `related-projects.yaml`.

use std::path::Path;

use anyhow::{Context, Result};

use crate::related_projects::{self, Edge, RelatedProjectsFile};

use super::load_proposals;

pub struct ApplyReport {
    pub added: usize,
    pub already_present: usize,
    pub kind_conflicts: Vec<KindConflict>,
}

pub struct KindConflict {
    pub proposed: Edge,
    pub existing: Edge,
}

pub fn run_discover_apply(config_root: &Path) -> Result<()> {
    let report = apply_proposals(config_root)?;
    eprintln!(
        "discover-apply: added {} edge(s), {} already present, {} kind-conflict(s)",
        report.added, report.already_present, report.kind_conflicts.len()
    );
    for c in &report.kind_conflicts {
        eprintln!(
            "  conflict: proposed {:?} on {:?} but existing {:?} blocks it",
            c.proposed.kind, c.proposed.participants, c.existing.kind
        );
    }
    Ok(())
}

pub fn apply_proposals(config_root: &Path) -> Result<ApplyReport> {
    let proposals = load_proposals(config_root)?;
    let mut file = related_projects::load_or_empty(config_root)?;
    let mut added = 0usize;
    let mut already_present = 0usize;
    let mut kind_conflicts = Vec::new();

    for p in proposals.proposals {
        let proposed = Edge {
            kind: p.kind,
            participants: p.participants,
        };
        if let Err(e) = proposed.validate() {
            eprintln!("  skipping invalid proposal: {e:#}");
            continue;
        }
        if let Some(existing) = find_conflicting_kind(&file, &proposed) {
            kind_conflicts.push(KindConflict {
                proposed: proposed.clone(),
                existing: existing.clone(),
            });
            continue;
        }
        match file.add_edge(proposed.clone())? {
            true => added += 1,
            false => already_present += 1,
        }
    }

    if added > 0 {
        related_projects::save_atomic(config_root, &file)
            .context("save related-projects.yaml after applying proposals")?;
    }

    Ok(ApplyReport {
        added,
        already_present,
        kind_conflicts,
    })
}

/// Look for an existing edge on the same participant pair with a
/// *different* kind than `proposed`. Sibling/parent-of on the same
/// unordered pair are mutually exclusive at the apply layer even though
/// the underlying schema would technically allow both to coexist —
/// this check is the policy knob.
fn find_conflicting_kind<'a>(
    file: &'a RelatedProjectsFile,
    proposed: &Edge,
) -> Option<&'a Edge> {
    let pair_sorted = {
        let mut v = proposed.participants.clone();
        v.sort();
        v
    };
    file.edges.iter().find(|e| {
        let mut ev = e.participants.clone();
        ev.sort();
        ev == pair_sorted && e.kind != proposed.kind
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::{ProposalRecord, ProposalsFile, PROPOSALS_SCHEMA_VERSION};
    use super::super::save_proposals_atomic;
    use crate::projects;
    use crate::related_projects::EdgeKind;
    use tempfile::TempDir;

    fn seed_two_projects(cfg: &std::path::Path) -> std::path::PathBuf {
        let mut catalog = projects::ProjectsCatalog::default();
        let a = cfg.parent().unwrap().join("A");
        let b = cfg.parent().unwrap().join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        projects::try_add_named(&mut catalog, "A", &a).unwrap();
        projects::try_add_named(&mut catalog, "B", &b).unwrap();
        projects::save_atomic(cfg, &catalog).unwrap();
        a
    }

    fn mk_proposals(pairs: &[(EdgeKind, &str, &str)]) -> ProposalsFile {
        ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "t".to_string(),
            source_tree_shas: Default::default(),
            proposals: pairs
                .iter()
                .map(|(k, a, b)| ProposalRecord {
                    kind: *k,
                    participants: vec![a.to_string(), b.to_string()],
                    rationale: "test".to_string(),
                    supporting_surface_fields: vec![],
                })
                .collect(),
            failures: vec![],
        }
    }

    #[test]
    fn apply_adds_new_edges() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);

        save_proposals_atomic(&cfg, &mk_proposals(&[(EdgeKind::Sibling, "A", "B")])).unwrap();
        let report = apply_proposals(&cfg).unwrap();

        assert_eq!(report.added, 1);
        assert_eq!(report.already_present, 0);
        assert!(report.kind_conflicts.is_empty());
        let loaded = related_projects::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn apply_is_idempotent_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        save_proposals_atomic(&cfg, &mk_proposals(&[(EdgeKind::Sibling, "A", "B")])).unwrap();

        let first = apply_proposals(&cfg).unwrap();
        assert_eq!(first.added, 1);
        let second = apply_proposals(&cfg).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.already_present, 1);
    }

    #[test]
    fn apply_rejects_kind_conflict_and_continues() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        // Seed existing parent-of(A, B).
        let mut file = related_projects::RelatedProjectsFile::default();
        file.add_edge(Edge::parent_of("A", "B")).unwrap();
        related_projects::save_atomic(&cfg, &file).unwrap();

        // Propose sibling(A, B) — must be rejected. Also include a
        // harmless sibling(B, "C") to prove "continues" works; seed C first.
        let mut catalog = projects::load_or_empty(&cfg).unwrap();
        let c_path = tmp.path().join("C");
        std::fs::create_dir_all(&c_path).unwrap();
        projects::try_add_named(&mut catalog, "C", &c_path).unwrap();
        projects::save_atomic(&cfg, &catalog).unwrap();

        save_proposals_atomic(
            &cfg,
            &mk_proposals(&[
                (EdgeKind::Sibling, "A", "B"),   // conflicts
                (EdgeKind::Sibling, "B", "C"),   // OK
            ]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 1, "second proposal should still land");
        assert_eq!(report.kind_conflicts.len(), 1);

        let loaded = related_projects::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
        assert!(loaded.edges.iter().any(|e| e.kind == EdgeKind::ParentOf));
        assert!(loaded.edges.iter().any(|e| e.kind == EdgeKind::Sibling));
    }
}
