//! Types for the component-relationship ontology (schema v2).
//!
//! Canonical spec: `docs/component-ontology.md` §5 (edge-kind reference),
//! §6 (direction table), §3.2 (lifecycle scopes), §9 (library surface).
//!
//! The library is deliberately host-agnostic: no references to
//! Ravel-Lite's phases, plans, backlog, or state directories. Its
//! universe is edges, kinds, lifecycles, components, and evidence. That
//! constraint is what makes the module promotable to a standalone
//! workspace crate in Phase B without carrying host concepts.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// `schema_version` value for `related-components.yaml`. Independent of
/// the schema version of `defaults/ontology.yaml` (which versions the
/// ontology definition itself — see `defaults.rs`).
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    // Dependency family.
    DependsOn,
    HasOptionalDependency,
    ProvidedByHost,
    // Linkage family.
    LinksStatically,
    LinksDynamically,
    // Generation family.
    Generates,
    Scaffolds,
    // Communication family.
    CommunicatesWith,
    Calls,
    // Orchestration family.
    Invokes,
    Orchestrates,
    Embeds,
    // Testing family.
    Tests,
    ProvidesFixturesFor,
    // Specification family.
    ConformsTo,
    CoImplements,
    Describes,
}

impl EdgeKind {
    /// Symmetric kinds are order-insensitive (participants sorted);
    /// directed kinds preserve the semantic order defined in §6.
    /// Matches the `directed` column in `defaults/ontology.yaml`.
    pub fn is_directed(self) -> bool {
        !matches!(self, EdgeKind::CommunicatesWith | EdgeKind::CoImplements)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::DependsOn => "depends-on",
            EdgeKind::HasOptionalDependency => "has-optional-dependency",
            EdgeKind::ProvidedByHost => "provided-by-host",
            EdgeKind::LinksStatically => "links-statically",
            EdgeKind::LinksDynamically => "links-dynamically",
            EdgeKind::Generates => "generates",
            EdgeKind::Scaffolds => "scaffolds",
            EdgeKind::CommunicatesWith => "communicates-with",
            EdgeKind::Calls => "calls",
            EdgeKind::Invokes => "invokes",
            EdgeKind::Orchestrates => "orchestrates",
            EdgeKind::Embeds => "embeds",
            EdgeKind::Tests => "tests",
            EdgeKind::ProvidesFixturesFor => "provides-fixtures-for",
            EdgeKind::ConformsTo => "conforms-to",
            EdgeKind::CoImplements => "co-implements",
            EdgeKind::Describes => "describes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "depends-on" => EdgeKind::DependsOn,
            "has-optional-dependency" => EdgeKind::HasOptionalDependency,
            "provided-by-host" => EdgeKind::ProvidedByHost,
            "links-statically" => EdgeKind::LinksStatically,
            "links-dynamically" => EdgeKind::LinksDynamically,
            "generates" => EdgeKind::Generates,
            "scaffolds" => EdgeKind::Scaffolds,
            "communicates-with" => EdgeKind::CommunicatesWith,
            "calls" => EdgeKind::Calls,
            "invokes" => EdgeKind::Invokes,
            "orchestrates" => EdgeKind::Orchestrates,
            "embeds" => EdgeKind::Embeds,
            "tests" => EdgeKind::Tests,
            "provides-fixtures-for" => EdgeKind::ProvidesFixturesFor,
            "conforms-to" => EdgeKind::ConformsTo,
            "co-implements" => EdgeKind::CoImplements,
            "describes" => EdgeKind::Describes,
            _ => return None,
        })
    }

    pub fn all() -> &'static [EdgeKind] {
        &[
            EdgeKind::DependsOn,
            EdgeKind::HasOptionalDependency,
            EdgeKind::ProvidedByHost,
            EdgeKind::LinksStatically,
            EdgeKind::LinksDynamically,
            EdgeKind::Generates,
            EdgeKind::Scaffolds,
            EdgeKind::CommunicatesWith,
            EdgeKind::Calls,
            EdgeKind::Invokes,
            EdgeKind::Orchestrates,
            EdgeKind::Embeds,
            EdgeKind::Tests,
            EdgeKind::ProvidesFixturesFor,
            EdgeKind::ConformsTo,
            EdgeKind::CoImplements,
            EdgeKind::Describes,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleScope {
    Design,
    Codegen,
    Build,
    Test,
    Deploy,
    Runtime,
    DevWorkflow,
}

impl LifecycleScope {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleScope::Design => "design",
            LifecycleScope::Codegen => "codegen",
            LifecycleScope::Build => "build",
            LifecycleScope::Test => "test",
            LifecycleScope::Deploy => "deploy",
            LifecycleScope::Runtime => "runtime",
            LifecycleScope::DevWorkflow => "dev-workflow",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "design" => LifecycleScope::Design,
            "codegen" => LifecycleScope::Codegen,
            "build" => LifecycleScope::Build,
            "test" => LifecycleScope::Test,
            "deploy" => LifecycleScope::Deploy,
            "runtime" => LifecycleScope::Runtime,
            "dev-workflow" => LifecycleScope::DevWorkflow,
            _ => return None,
        })
    }

    pub fn all() -> &'static [LifecycleScope] {
        &[
            LifecycleScope::Design,
            LifecycleScope::Codegen,
            LifecycleScope::Build,
            LifecycleScope::Test,
            LifecycleScope::Deploy,
            LifecycleScope::Runtime,
            LifecycleScope::DevWorkflow,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade {
    Strong,
    Medium,
    Weak,
}

impl EvidenceGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceGrade::Strong => "strong",
            EvidenceGrade::Medium => "medium",
            EvidenceGrade::Weak => "weak",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "strong" => EvidenceGrade::Strong,
            "medium" => EvidenceGrade::Medium,
            "weak" => EvidenceGrade::Weak,
            _ => return None,
        })
    }

    pub fn all() -> &'static [EvidenceGrade] {
        &[EvidenceGrade::Strong, EvidenceGrade::Medium, EvidenceGrade::Weak]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub participants: Vec<String>,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

impl Edge {
    /// Enforce §9.5 invariants: exactly two distinct participants;
    /// directed kinds keep caller-supplied order, symmetric kinds must
    /// be stored sorted; `evidence_fields` non-empty unless `Weak`;
    /// `rationale` non-empty.
    pub fn validate(&self) -> Result<()> {
        if self.participants.len() != 2 {
            bail!(
                "edge must have exactly 2 participants, got {} ({:?})",
                self.participants.len(),
                self.participants
            );
        }
        if self.participants[0] == self.participants[1] {
            bail!(
                "edge participants must be distinct; '{}' appears twice",
                self.participants[0]
            );
        }
        if !self.kind.is_directed() {
            let mut sorted = self.participants.clone();
            sorted.sort();
            if sorted != self.participants {
                bail!(
                    "symmetric kind '{}' requires participants stored in sorted order; got {:?}",
                    self.kind.as_str(),
                    self.participants
                );
            }
        }
        if self.rationale.trim().is_empty() {
            bail!("edge rationale must be non-empty");
        }
        if self.evidence_grade != EvidenceGrade::Weak && self.evidence_fields.is_empty() {
            bail!(
                "edge with evidence_grade={} must carry at least one evidence_field; only `weak` may omit",
                self.evidence_grade.as_str()
            );
        }
        Ok(())
    }

    /// Dedup key per §7.3: for symmetric kinds the participants are
    /// sorted before comparison; for directed kinds participant order
    /// is semantic and part of the identity. Two edges with equal keys
    /// are the same edge for idempotent-insert purposes.
    pub fn canonical_key(&self) -> (EdgeKind, LifecycleScope, Vec<String>) {
        let participants = if self.kind.is_directed() {
            self.participants.clone()
        } else {
            let mut sorted = self.participants.clone();
            sorted.sort();
            sorted
        };
        (self.kind, self.lifecycle, participants)
    }

    pub fn involves(&self, component: &str) -> bool {
        self.participants.iter().any(|p| p == component)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedComponentsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl Default for RelatedComponentsFile {
    fn default() -> Self {
        RelatedComponentsFile {
            schema_version: SCHEMA_VERSION,
            edges: Vec::new(),
        }
    }
}

impl RelatedComponentsFile {
    /// Append `edge` if no existing edge shares its canonical key.
    /// Returns true on insert, false on dedup no-op. Validates before
    /// either path so an invalid edge never lands in the file.
    pub fn add_edge(&mut self, edge: Edge) -> Result<bool> {
        edge.validate()?;
        let key = edge.canonical_key();
        if self.edges.iter().any(|e| e.canonical_key() == key) {
            return Ok(false);
        }
        self.edges.push(edge);
        Ok(true)
    }

    /// Rewrite every participant name matching `old` to `new`. Cascades
    /// from an external component-rename at the catalog layer. The
    /// caller is responsible for ensuring `new` does not collide with
    /// an existing component name; under that invariant no self-loop
    /// or duplicate can emerge from the substitution.
    pub fn rename_component_in_edges(&mut self, old: &str, new: &str) -> bool {
        let mut changed = false;
        for edge in &mut self.edges {
            for participant in &mut edge.participants {
                if participant == old {
                    *participant = new.to_string();
                    changed = true;
                }
            }
            // Symmetric kinds must remain sorted after rename; directed
            // kinds keep their caller-supplied order.
            if !edge.kind.is_directed() {
                edge.participants.sort();
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strong_edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle,
            participants: vec![a.into(), b.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![format!("{a}.produces_files"), format!("{b}.consumes_files")],
            rationale: "example".into(),
        }
    }

    #[test]
    fn every_kind_has_a_canonical_string_and_roundtrips() {
        for kind in EdgeKind::all() {
            let s = kind.as_str();
            assert_eq!(EdgeKind::parse(s), Some(*kind));
            let ser = serde_yaml::to_string(kind).unwrap();
            let parsed: EdgeKind = serde_yaml::from_str(&ser).unwrap();
            assert_eq!(parsed, *kind);
        }
    }

    #[test]
    fn every_lifecycle_roundtrips_through_str_and_yaml() {
        for lc in LifecycleScope::all() {
            assert_eq!(LifecycleScope::parse(lc.as_str()), Some(*lc));
            let ser = serde_yaml::to_string(lc).unwrap();
            let parsed: LifecycleScope = serde_yaml::from_str(&ser).unwrap();
            assert_eq!(parsed, *lc);
        }
    }

    #[test]
    fn every_evidence_grade_roundtrips_through_str_and_yaml() {
        for g in EvidenceGrade::all() {
            assert_eq!(EvidenceGrade::parse(g.as_str()), Some(*g));
            let ser = serde_yaml::to_string(g).unwrap();
            let parsed: EvidenceGrade = serde_yaml::from_str(&ser).unwrap();
            assert_eq!(parsed, *g);
        }
    }

    #[test]
    fn is_directed_agrees_with_spec_table() {
        // §6 lists exactly these two as symmetric; everything else is
        // directed. Pin that here so drift in the enum surfaces loudly.
        for kind in EdgeKind::all() {
            let expected = !matches!(
                kind,
                EdgeKind::CommunicatesWith | EdgeKind::CoImplements
            );
            assert_eq!(
                kind.is_directed(),
                expected,
                "direction for {} diverged from §6",
                kind.as_str()
            );
        }
    }

    #[test]
    fn edge_validate_rejects_self_loop() {
        let mut e = strong_edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "A");
        e.participants = vec!["A".into(), "A".into()];
        let err = e.validate().unwrap_err();
        assert!(format!("{err:#}").contains("distinct"));
    }

    #[test]
    fn edge_validate_rejects_wrong_participant_count() {
        let mut e = strong_edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B");
        e.participants = vec!["solo".into()];
        let err = e.validate().unwrap_err();
        assert!(format!("{err:#}").contains("exactly 2"));
    }

    #[test]
    fn edge_validate_rejects_empty_rationale() {
        let mut e = strong_edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B");
        e.rationale = "   \n".into();
        let err = e.validate().unwrap_err();
        assert!(format!("{err:#}").contains("rationale"));
    }

    #[test]
    fn edge_validate_requires_evidence_fields_unless_weak() {
        let mut e = strong_edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B");
        e.evidence_fields.clear();
        let err = e.validate().unwrap_err();
        assert!(format!("{err:#}").contains("evidence_field"));

        // Weak is permitted to have an empty evidence_fields list.
        e.evidence_grade = EvidenceGrade::Weak;
        e.validate().unwrap();
    }

    #[test]
    fn edge_validate_rejects_unsorted_symmetric_participants() {
        // communicates-with is symmetric → participants must be sorted.
        let e = Edge {
            kind: EdgeKind::CommunicatesWith,
            lifecycle: LifecycleScope::Runtime,
            participants: vec!["Zeta".into(), "Alpha".into()], // reversed
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Alpha.network_endpoints".into()],
            rationale: "example".into(),
        };
        let err = e.validate().unwrap_err();
        assert!(
            format!("{err:#}").contains("sorted"),
            "symmetric-kind unsorted participants must be rejected: {err:#}"
        );
    }

    #[test]
    fn directed_edge_preserves_caller_supplied_order() {
        let e = strong_edge(EdgeKind::Generates, LifecycleScope::Codegen, "Gen", "Out");
        e.validate().unwrap();
        // Reversing directed participants is a distinct edge, not an error.
        let reversed = strong_edge(EdgeKind::Generates, LifecycleScope::Codegen, "Out", "Gen");
        reversed.validate().unwrap();
        assert_ne!(e.canonical_key(), reversed.canonical_key());
    }

    #[test]
    fn canonical_key_for_symmetric_kinds_sorts_participants() {
        let a = Edge {
            kind: EdgeKind::CoImplements,
            lifecycle: LifecycleScope::Design,
            participants: vec!["Alpha".into(), "Beta".into()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["Alpha.purpose".into()],
            rationale: "rfc".into(),
        };
        // After validation succeeds the canonical key's participants are
        // always sorted; a `Beta, Alpha` edge would fail validate.
        let key = a.canonical_key();
        assert_eq!(key.2, vec!["Alpha".to_string(), "Beta".to_string()]);
    }

    #[test]
    fn canonical_key_segregates_by_lifecycle() {
        let build = strong_edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B");
        let runtime = strong_edge(EdgeKind::DependsOn, LifecycleScope::Runtime, "A", "B");
        // Same kind, same participants, different lifecycle → two edges.
        assert_ne!(build.canonical_key(), runtime.canonical_key());
    }

    #[test]
    fn add_edge_is_idempotent_on_directed_kinds() {
        let mut file = RelatedComponentsFile::default();
        let e = strong_edge(EdgeKind::Generates, LifecycleScope::Codegen, "Gen", "Out");
        assert!(file.add_edge(e.clone()).unwrap());
        assert!(!file.add_edge(e).unwrap());
        assert_eq!(file.edges.len(), 1);
    }

    #[test]
    fn add_edge_dedups_symmetric_by_sorted_participants() {
        let mut file = RelatedComponentsFile::default();
        let ab = Edge {
            kind: EdgeKind::CoImplements,
            lifecycle: LifecycleScope::Design,
            participants: vec!["Alpha".into(), "Beta".into()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["Alpha.purpose".into()],
            rationale: "rfc".into(),
        };
        assert!(file.add_edge(ab.clone()).unwrap());
        // Add_edge validate will reject the reversed form directly, so
        // construct a second edge with sorted participants (same key).
        let ba = Edge {
            participants: vec!["Alpha".into(), "Beta".into()],
            ..ab
        };
        assert!(!file.add_edge(ba).unwrap());
        assert_eq!(file.edges.len(), 1);
    }

    #[test]
    fn add_edge_accepts_multiple_kinds_on_same_pair() {
        // §3.5: one pair, two kinds, two scopes — expected.
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "Ravel-Lite",
            "Ravel",
        ))
        .unwrap();
        file.add_edge(strong_edge(
            EdgeKind::Orchestrates,
            LifecycleScope::DevWorkflow,
            "Ravel-Lite",
            "Ravel",
        ))
        .unwrap();
        assert_eq!(file.edges.len(), 2);
    }

    #[test]
    fn rename_component_in_edges_rewrites_every_occurrence() {
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "OldName",
            "Peer",
        ))
        .unwrap();
        let changed = file.rename_component_in_edges("OldName", "NewName");
        assert!(changed);
        assert_eq!(file.edges[0].participants, vec!["NewName", "Peer"]);
    }

    #[test]
    fn rename_component_in_edges_resorts_symmetric_participants() {
        let mut file = RelatedComponentsFile::default();
        let edge = Edge {
            kind: EdgeKind::CoImplements,
            lifecycle: LifecycleScope::Design,
            // After rename Mango < Zebra → must re-sort.
            participants: vec!["Apple".into(), "Zebra".into()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["Apple.purpose".into()],
            rationale: "rfc".into(),
        };
        file.add_edge(edge).unwrap();
        file.rename_component_in_edges("Apple", "Mango");
        assert_eq!(
            file.edges[0].participants,
            vec!["Mango".to_string(), "Zebra".to_string()]
        );
    }

    #[test]
    fn rename_component_in_edges_no_op_on_missing_name() {
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        ))
        .unwrap();
        assert!(!file.rename_component_in_edges("Nonexistent", "Other"));
    }

    #[test]
    fn edge_round_trips_through_yaml() {
        let original = strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "Ravel-Lite",
            "Ravel",
        );
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: Edge = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn related_components_file_round_trips_through_yaml() {
        let mut file = RelatedComponentsFile::default();
        file.add_edge(strong_edge(
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        ))
        .unwrap();
        let yaml = serde_yaml::to_string(&file).unwrap();
        let parsed: RelatedComponentsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
    }
}
