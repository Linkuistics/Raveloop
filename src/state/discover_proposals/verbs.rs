//! `add-proposal` verb — validates a Stage 2 proposal at CLI-parse time
//! and appends it to `<config_root>/discover-proposals.yaml`.
//!
//! Validation pipeline, in order of cheapness → expense:
//!
//! 1. `clap`'s `ValueEnum` parse rejects unknown `--kind`, `--lifecycle`,
//!    `--evidence-grade` values at arg-parse time, listing the valid
//!    vocabulary in the error message. The LLM sees this and retries.
//! 2. Exactly two `--participant` flags are required; anything else is
//!    a structural error.
//! 3. Both participants must be in the projects catalog — Stage 2 can
//!    only propose edges between known components.
//! 4. `Edge::validate()` catches self-loops, empty rationale, and
//!    missing `evidence_fields` when the grade is not `weak`.
//! 5. Exact-duplicate detection against the already-accumulated
//!    proposals — a re-run or a near-miss retry never lands the same
//!    edge twice.

use std::path::Path;

use anyhow::{bail, Result};

use crate::discover::schema::{ProposalRecord, ProposalsFile, PROPOSALS_SCHEMA_VERSION};
use crate::discover::{load_proposals, proposals_path, save_proposals_atomic};
use crate::ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
use crate::projects::{self, ProjectsCatalog};

pub struct AddProposalRequest<'a> {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub participants: &'a [String],
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

pub fn run_add_proposal(config_root: &Path, req: &AddProposalRequest<'_>) -> Result<()> {
    if req.participants.len() != 2 {
        bail!(
            "--participant must be supplied exactly twice (got {}); \
             pass two `--participant <NAME>` flags",
            req.participants.len()
        );
    }
    let a = &req.participants[0];
    let b = &req.participants[1];

    let catalog = projects::load_or_empty(config_root)?;
    require_component_known(&catalog, a)?;
    require_component_known(&catalog, b)?;

    let participants = canonicalise_participants_for_kind(req.kind, a, b);
    let edge = Edge {
        kind: req.kind,
        lifecycle: req.lifecycle,
        participants: participants.clone(),
        evidence_grade: req.evidence_grade,
        evidence_fields: req.evidence_fields.clone(),
        rationale: req.rationale.clone(),
    };
    edge.validate()?;

    let mut file = load_or_default_proposals_file(config_root)?;

    if proposal_already_present(&file, req.kind, req.lifecycle, &participants) {
        eprintln!(
            "proposal already present (kind={}, lifecycle={}, {} / {}); no change.",
            req.kind.as_str(),
            req.lifecycle.as_str(),
            a,
            b
        );
        return Ok(());
    }

    file.proposals.push(ProposalRecord {
        kind: edge.kind,
        lifecycle: edge.lifecycle,
        participants: edge.participants,
        evidence_grade: edge.evidence_grade,
        evidence_fields: edge.evidence_fields,
        rationale: edge.rationale,
    });
    save_proposals_atomic(config_root, &file)
}

fn load_or_default_proposals_file(config_root: &Path) -> Result<ProposalsFile> {
    if proposals_path(config_root).exists() {
        load_proposals(config_root)
    } else {
        Ok(ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: String::new(),
            source_project_states: Default::default(),
            proposals: Vec::new(),
            failures: Vec::new(),
        })
    }
}

fn proposal_already_present(
    file: &ProposalsFile,
    kind: EdgeKind,
    lifecycle: LifecycleScope,
    participants: &[String],
) -> bool {
    file.proposals.iter().any(|p| {
        p.kind == kind && p.lifecycle == lifecycle && p.participants == participants
    })
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
            "component '{}' is not in the projects catalog; \
             only catalogued components may be Stage 2 participants. \
             Known components: {}",
            name,
            known_component_names(catalog),
        );
    }
    Ok(())
}

fn known_component_names(catalog: &ProjectsCatalog) -> String {
    if catalog.projects.is_empty() {
        return "(empty)".to_string();
    }
    catalog
        .projects
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::try_add_named;
    use tempfile::TempDir;

    fn scaffold_cfg_with_catalog(components: &[&str]) -> (TempDir, std::path::PathBuf) {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let mut catalog = projects::ProjectsCatalog::default();
        for name in components {
            let path = tmp.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            try_add_named(&mut catalog, name, &path).unwrap();
        }
        projects::save_atomic(&cfg, &catalog).unwrap();
        (tmp, cfg)
    }

    fn sample_request<'a>(
        kind: EdgeKind,
        lifecycle: LifecycleScope,
        participants: &'a [String],
    ) -> AddProposalRequest<'a> {
        // Derive evidence_fields from whatever participants were actually
        // passed — tests that exercise count-validation supply fewer than
        // two, and must not crash the helper before the verb sees the
        // bad input.
        let evidence_fields = participants
            .iter()
            .map(|p| format!("{p}.produces_files"))
            .collect();
        AddProposalRequest {
            kind,
            lifecycle,
            participants,
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields,
            rationale: "because tests said so".to_string(),
        }
    }

    #[test]
    fn add_proposal_appends_to_empty_file() {
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha", "Beta"]);
        let participants = vec!["Alpha".to_string(), "Beta".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        run_add_proposal(&cfg, &req).unwrap();

        let loaded = load_proposals(&cfg).unwrap();
        assert_eq!(loaded.proposals.len(), 1);
        assert_eq!(loaded.proposals[0].kind, EdgeKind::Generates);
        assert_eq!(loaded.proposals[0].participants, participants);
    }

    #[test]
    fn add_proposal_is_idempotent_on_duplicate() {
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha", "Beta"]);
        let participants = vec!["Alpha".to_string(), "Beta".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        run_add_proposal(&cfg, &req).unwrap();
        run_add_proposal(&cfg, &req).unwrap();

        let loaded = load_proposals(&cfg).unwrap();
        assert_eq!(loaded.proposals.len(), 1, "duplicate must be dropped");
    }

    #[test]
    fn add_proposal_canonicalises_symmetric_participants() {
        // co-implements is symmetric → participants stored sorted.
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha", "Beta"]);
        let reversed = vec!["Beta".to_string(), "Alpha".to_string()];
        let req = sample_request(EdgeKind::CoImplements, LifecycleScope::Design, &reversed);
        run_add_proposal(&cfg, &req).unwrap();

        let loaded = load_proposals(&cfg).unwrap();
        assert_eq!(
            loaded.proposals[0].participants,
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
    }

    #[test]
    fn add_proposal_rejects_unknown_participant() {
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha"]);
        let participants = vec!["Alpha".to_string(), "Ghost".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        let err = run_add_proposal(&cfg, &req).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Ghost"), "error must name the unknown component: {msg}");
    }

    #[test]
    fn add_proposal_rejects_self_loop_via_edge_validate() {
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha"]);
        let participants = vec!["Alpha".to_string(), "Alpha".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        let err = run_add_proposal(&cfg, &req).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("distinct"), "self-loop must be rejected: {msg}");
    }

    #[test]
    fn add_proposal_rejects_wrong_participant_count() {
        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha"]);
        let participants = vec!["Alpha".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        let err = run_add_proposal(&cfg, &req).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("--participant"), "error must instruct on --participant: {msg}");
    }

    #[test]
    fn add_proposal_preserves_failures_from_initial_file() {
        use crate::discover::schema::Stage1Failure;

        let (_tmp, cfg) = scaffold_cfg_with_catalog(&["Alpha", "Beta"]);
        // Pre-seed proposals file with a failure (as Stage 2 will).
        let initial = ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "2026-04-24T00:00:00Z".to_string(),
            source_project_states: Default::default(),
            proposals: Vec::new(),
            failures: vec![Stage1Failure {
                project: "Gamma".to_string(),
                error: "not yet analysed".to_string(),
            }],
        };
        save_proposals_atomic(&cfg, &initial).unwrap();

        let participants = vec!["Alpha".to_string(), "Beta".to_string()];
        let req = sample_request(EdgeKind::Generates, LifecycleScope::Codegen, &participants);
        run_add_proposal(&cfg, &req).unwrap();

        let loaded = load_proposals(&cfg).unwrap();
        assert_eq!(loaded.failures.len(), 1, "pre-existing failure must survive add-proposal");
        assert_eq!(loaded.proposals.len(), 1);
        assert_eq!(loaded.generated_at, "2026-04-24T00:00:00Z");
    }
}
