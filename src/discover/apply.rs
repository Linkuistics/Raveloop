//! Merge `discover-proposals.yaml` into `related-components.yaml`.

use std::path::Path;

use anyhow::{Context, Result};

use crate::ontology::{Edge, EdgeKind, RelatedComponentsFile};
use crate::related_components;

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
        "discover-apply: added {} edge(s), {} already present, {} directional conflict(s)",
        report.added, report.already_present, report.kind_conflicts.len()
    );
    for c in &report.kind_conflicts {
        eprintln!(
            "  conflict: proposed {}@{} on {:?} but existing reverses participants ({:?})",
            c.proposed.kind.as_str(),
            c.proposed.lifecycle.as_str(),
            c.proposed.participants,
            c.existing.participants,
        );
    }
    Ok(())
}

pub fn apply_proposals(config_root: &Path) -> Result<ApplyReport> {
    let proposals = load_proposals(config_root)?;
    let mut file = related_components::load_or_empty(config_root)?;
    let mut added = 0usize;
    let mut already_present = 0usize;
    let mut kind_conflicts = Vec::new();

    for p in proposals.proposals {
        let participants = canonicalise_for_kind(p.kind, p.participants);
        let proposed = Edge {
            kind: p.kind,
            lifecycle: p.lifecycle,
            participants,
            evidence_grade: p.evidence_grade,
            evidence_fields: p.evidence_fields,
            rationale: p.rationale,
        };
        if let Err(e) = proposed.validate() {
            eprintln!("  skipping invalid proposal: {e:#}");
            continue;
        }
        if let Some(existing) = find_directional_conflict(&file, &proposed) {
            kind_conflicts.push(KindConflict {
                proposed: proposed.clone(),
                existing: existing.clone(),
            });
            continue;
        }
        match file.add_edge(proposed)? {
            true => added += 1,
            false => already_present += 1,
        }
    }

    if added > 0 {
        related_components::save_atomic(config_root, &file)
            .context("save related-components.yaml after applying proposals")?;
    }

    Ok(ApplyReport {
        added,
        already_present,
        kind_conflicts,
    })
}

/// Spec §7.4: the only modelling conflict the apply layer detects is a
/// directed edge proposed in the reverse direction of an existing one
/// at the same `(kind, lifecycle)`. Cross-kind on the same pair is
/// expected (§3.5), so it is not a conflict. Exact-duplicate edges
/// route through `add_edge` as `already_present`, never as conflicts.
fn find_directional_conflict<'a>(
    file: &'a RelatedComponentsFile,
    proposed: &Edge,
) -> Option<&'a Edge> {
    if !proposed.kind.is_directed() {
        return None;
    }
    file.edges.iter().find(|e| {
        e.kind == proposed.kind
            && e.lifecycle == proposed.lifecycle
            && e.participants != proposed.participants
            && participants_reversed(&e.participants, &proposed.participants)
    })
}

fn participants_reversed(a: &[String], b: &[String]) -> bool {
    a.len() == 2 && b.len() == 2 && a[0] == b[1] && a[1] == b[0]
}

fn canonicalise_for_kind(kind: EdgeKind, mut participants: Vec<String>) -> Vec<String> {
    if !kind.is_directed() {
        participants.sort();
    }
    participants
}

#[cfg(test)]
mod tests {
    use super::super::save_proposals_atomic;
    use super::super::schema::{ProposalRecord, ProposalsFile, PROPOSALS_SCHEMA_VERSION};
    use super::*;
    use crate::ontology::{EvidenceGrade, LifecycleScope};
    use crate::projects;
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

    fn proposal(
        kind: EdgeKind,
        lifecycle: LifecycleScope,
        a: &str,
        b: &str,
    ) -> ProposalRecord {
        ProposalRecord {
            kind,
            lifecycle,
            participants: vec![a.to_string(), b.to_string()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![format!("{a}.produces_files"), format!("{b}.consumes_files")],
            rationale: "test".to_string(),
        }
    }

    fn mk_proposals(records: Vec<ProposalRecord>) -> ProposalsFile {
        ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "t".to_string(),
            source_project_states: Default::default(),
            proposals: records,
            failures: vec![],
        }
    }

    fn strong_edge(
        kind: EdgeKind,
        lifecycle: LifecycleScope,
        a: &str,
        b: &str,
    ) -> Edge {
        let participants = canonicalise_for_kind(kind, vec![a.to_string(), b.to_string()]);
        Edge {
            kind,
            lifecycle,
            participants,
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![format!("{a}.produces_files")],
            rationale: "seed".to_string(),
        }
    }

    #[test]
    fn apply_adds_new_edges() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);

        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![proposal(
                EdgeKind::Generates,
                LifecycleScope::Codegen,
                "A",
                "B",
            )]),
        )
        .unwrap();
        let report = apply_proposals(&cfg).unwrap();

        assert_eq!(report.added, 1);
        assert_eq!(report.already_present, 0);
        assert!(report.kind_conflicts.is_empty());
        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn apply_is_idempotent_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![proposal(
                EdgeKind::Generates,
                LifecycleScope::Codegen,
                "A",
                "B",
            )]),
        )
        .unwrap();

        let first = apply_proposals(&cfg).unwrap();
        assert_eq!(first.added, 1);
        let second = apply_proposals(&cfg).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.already_present, 1);
    }

    #[test]
    fn apply_rejects_reversed_directed_edge_at_same_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        // Seed existing generates(A, B) @ codegen.
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        ))
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        // Propose the reversed direction at the same lifecycle.
        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![proposal(
                EdgeKind::Generates,
                LifecycleScope::Codegen,
                "B",
                "A",
            )]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 0, "reversed directed edge must not be added");
        assert_eq!(report.kind_conflicts.len(), 1);

        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0].participants, vec!["A", "B"]);
    }

    #[test]
    fn apply_treats_duplicate_directed_as_already_present() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        ))
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![proposal(
                EdgeKind::Generates,
                LifecycleScope::Codegen,
                "A",
                "B",
            )]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 0);
        assert_eq!(report.already_present, 1);
        assert!(report.kind_conflicts.is_empty());
    }

    #[test]
    fn apply_accepts_multiple_kinds_on_same_pair() {
        // §3.5: two distinct (kind, lifecycle) tuples on one pair are
        // legal — one edge generates schemas, the other orchestrates a
        // dev-workflow loop. Both must land.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);

        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![
                proposal(EdgeKind::Generates, LifecycleScope::Codegen, "A", "B"),
                proposal(EdgeKind::Orchestrates, LifecycleScope::DevWorkflow, "A", "B"),
            ]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 2);
        assert!(report.kind_conflicts.is_empty());
    }

    #[test]
    fn apply_rejects_directional_conflict_and_continues() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        ))
        .unwrap();
        related_components::save_atomic(&cfg, &file).unwrap();

        // Catalogue C so the harmless second proposal can land.
        let mut catalog = projects::load_or_empty(&cfg).unwrap();
        let c_path = tmp.path().join("C");
        std::fs::create_dir_all(&c_path).unwrap();
        projects::try_add_named(&mut catalog, "C", &c_path).unwrap();
        projects::save_atomic(&cfg, &catalog).unwrap();

        save_proposals_atomic(
            &cfg,
            &mk_proposals(vec![
                proposal(EdgeKind::Generates, LifecycleScope::Codegen, "B", "A"), // conflicts
                proposal(EdgeKind::Generates, LifecycleScope::Codegen, "B", "C"), // OK
            ]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 1, "second proposal should still land");
        assert_eq!(report.kind_conflicts.len(), 1);

        let loaded = related_components::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
    }
}
