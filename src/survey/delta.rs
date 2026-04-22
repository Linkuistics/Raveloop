// src/survey/delta.rs
//
// Delta classification and merge for incremental surveys. Given a
// prior `SurveyResponse` loaded from disk and a freshly-loaded list
// of `PlanSnapshot`s, partition current plans into unchanged /
// changed / added, and separately record keys removed since the
// prior. Merge an LLM-produced delta back with unchanged rows into
// a canonical `SurveyResponse`.
//
// All mechanical work lives here: hash comparison, set arithmetic,
// row merging. The LLM sees only the changed+added payload.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};

use super::discover::PlanSnapshot;
use super::schema::{plan_key, PlanRow, SurveyResponse};

/// Outcome of diffing current plan snapshots against a prior survey.
/// Borrows from `current` (the freshly-loaded snapshots the caller
/// owns) and clones `PlanRow`s out of `prior` so the classification
/// can be handed off to a merge step that no longer needs the prior
/// in hand.
#[derive(Debug)]
pub struct PlanClassification<'a> {
    /// Rows whose `input_hash` matched the prior — carried forward
    /// verbatim into the merged survey.
    pub unchanged_rows: Vec<PlanRow>,
    /// Plans whose `input_hash` differs from the prior and must be
    /// re-analysed by the LLM.
    pub changed: Vec<&'a PlanSnapshot>,
    /// Plans present now but not in the prior — must be analysed.
    pub added: Vec<&'a PlanSnapshot>,
    /// `project/plan` keys present in the prior but no longer
    /// discovered. Pruned from the merged survey; surfaced to the LLM
    /// as context so it can update cross-plan blockers and streams.
    pub removed_keys: Vec<String>,
}

impl<'a> PlanClassification<'a> {
    /// Compare `current` plan snapshots against `prior` to classify
    /// each as unchanged / changed / added, and record any prior keys
    /// missing from `current` as removed.
    pub fn classify(prior: &SurveyResponse, current: &'a [PlanSnapshot]) -> Self {
        let prior_by_key: HashMap<String, &PlanRow> = prior
            .plans
            .iter()
            .map(|row| (plan_key(&row.project, &row.plan), row))
            .collect();
        let current_keys: HashSet<String> = current
            .iter()
            .map(|snap| plan_key(&snap.project, &snap.plan))
            .collect();

        let mut unchanged_rows = Vec::new();
        let mut changed = Vec::new();
        let mut added = Vec::new();
        for snapshot in current {
            let key = plan_key(&snapshot.project, &snapshot.plan);
            match prior_by_key.get(&key) {
                Some(prior_row) if prior_row.input_hash == snapshot.input_hash => {
                    unchanged_rows.push((*prior_row).clone());
                }
                Some(_) => changed.push(snapshot),
                None => added.push(snapshot),
            }
        }

        let removed_keys: Vec<String> = prior
            .plans
            .iter()
            .map(|row| plan_key(&row.project, &row.plan))
            .filter(|key| !current_keys.contains(key))
            .collect();

        Self { unchanged_rows, changed, added, removed_keys }
    }

    /// Plans that must appear in the LLM payload and thus in the LLM
    /// response. Caller feeds these to `render_survey_input_incremental`.
    pub fn plans_to_analyse(&self) -> Vec<&'a PlanSnapshot> {
        self.changed.iter().copied().chain(self.added.iter().copied()).collect()
    }

    /// `(project, plan)` keys that the LLM delta response is REQUIRED
    /// to cover — no more, no less. Mirrors `inject_input_hashes`'s
    /// hard-error contract: drift in either direction is a loud bail.
    pub fn expected_delta_keys(&self) -> HashSet<String> {
        self.changed
            .iter()
            .chain(self.added.iter())
            .map(|s| plan_key(&s.project, &s.plan))
            .collect()
    }

    /// True when nothing changed relative to prior — no changed, no
    /// added, no removed. Caller may elide the LLM call entirely and
    /// write the prior through unchanged.
    pub fn is_noop(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed_keys.is_empty()
    }
}

/// Combine the LLM's delta response with the classification's
/// unchanged rows into a full `SurveyResponse`. Annotation sections
/// (`cross_plan_blockers`, `parallel_streams`,
/// `recommended_invocation_order`) come from the LLM delta verbatim,
/// since the LLM was shown the full prior as context when producing
/// them.
///
/// Hard-errors if the LLM delta mutates a plan outside the declared
/// changed+added set, or omits any declared plan. Either direction is
/// the same class of failure as `inject_input_hashes`'s invariants.
pub fn merge_delta(
    classification: PlanClassification<'_>,
    llm_delta: SurveyResponse,
) -> Result<SurveyResponse> {
    let expected = classification.expected_delta_keys();
    let got: HashSet<String> = llm_delta
        .plans
        .iter()
        .map(|row| plan_key(&row.project, &row.plan))
        .collect();

    let extra: Vec<&String> = got.difference(&expected).collect();
    if !extra.is_empty() {
        bail!(
            "incremental survey response contains {} plan(s) outside the \
             declared changed+added set — the LLM returned rows for plans it \
             was not asked to analyse. Extra: {:?}",
            extra.len(),
            extra
        );
    }
    let missing: Vec<&String> = expected.difference(&got).collect();
    if !missing.is_empty() {
        bail!(
            "incremental survey response is missing {} of the {} plan(s) the \
             LLM was asked to analyse. Missing: {:?}",
            missing.len(),
            expected.len(),
            missing
        );
    }

    let mut plans = classification.unchanged_rows;
    plans.extend(llm_delta.plans);
    plans.sort_by(|a, b| (a.project.as_str(), a.plan.as_str()).cmp(&(b.project.as_str(), b.plan.as_str())));

    Ok(SurveyResponse {
        schema_version: llm_delta.schema_version,
        plans,
        cross_plan_blockers: llm_delta.cross_plan_blockers,
        parallel_streams: llm_delta.parallel_streams,
        recommended_invocation_order: llm_delta.recommended_invocation_order,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::SCHEMA_VERSION;

    fn snapshot(project: &str, plan: &str, hash: &str) -> PlanSnapshot {
        PlanSnapshot {
            project: project.into(),
            plan: plan.into(),
            phase: "work".into(),
            backlog: None,
            memory: None,
            input_hash: hash.into(),
            task_counts: None,
        }
    }

    fn row(project: &str, plan: &str, hash: &str) -> PlanRow {
        PlanRow {
            project: project.into(),
            plan: plan.into(),
            phase: "work".into(),
            unblocked: 1,
            blocked: 0,
            done: 0,
            received: 0,
            notes: String::new(),
            input_hash: hash.into(),
            task_counts: None,
        }
    }

    fn prior_with(plans: Vec<PlanRow>) -> SurveyResponse {
        SurveyResponse {
            schema_version: SCHEMA_VERSION,
            plans,
            cross_plan_blockers: vec![],
            parallel_streams: vec![],
            recommended_invocation_order: vec![],
        }
    }

    #[test]
    fn classify_treats_matching_hash_as_unchanged() {
        let prior = prior_with(vec![row("P", "a", "hash-a")]);
        let current = vec![snapshot("P", "a", "hash-a")];
        let c = PlanClassification::classify(&prior, &current);
        assert_eq!(c.unchanged_rows.len(), 1);
        assert!(c.changed.is_empty());
        assert!(c.added.is_empty());
        assert!(c.removed_keys.is_empty());
        assert!(c.is_noop());
    }

    #[test]
    fn classify_treats_different_hash_as_changed() {
        let prior = prior_with(vec![row("P", "a", "hash-old")]);
        let current = vec![snapshot("P", "a", "hash-new")];
        let c = PlanClassification::classify(&prior, &current);
        assert!(c.unchanged_rows.is_empty());
        assert_eq!(c.changed.len(), 1);
        assert!(c.added.is_empty());
        assert_eq!(c.expected_delta_keys(), HashSet::from(["P/a".to_string()]));
        assert!(!c.is_noop());
    }

    #[test]
    fn classify_treats_missing_from_prior_as_added() {
        let prior = prior_with(vec![]);
        let current = vec![snapshot("P", "new-plan", "hash-x")];
        let c = PlanClassification::classify(&prior, &current);
        assert!(c.unchanged_rows.is_empty());
        assert!(c.changed.is_empty());
        assert_eq!(c.added.len(), 1);
        assert_eq!(c.added[0].plan, "new-plan");
    }

    #[test]
    fn classify_records_removed_keys() {
        let prior = prior_with(vec![
            row("P", "a", "hash-a"),
            row("P", "gone", "hash-gone"),
        ]);
        let current = vec![snapshot("P", "a", "hash-a")];
        let c = PlanClassification::classify(&prior, &current);
        assert_eq!(c.removed_keys, vec!["P/gone".to_string()]);
    }

    #[test]
    fn classify_handles_mixed_classification() {
        let prior = prior_with(vec![
            row("P", "unchanged", "keep"),
            row("P", "changed", "old"),
            row("P", "removed", "whatever"),
        ]);
        let current = vec![
            snapshot("P", "unchanged", "keep"),
            snapshot("P", "changed", "new"),
            snapshot("P", "added", "fresh"),
        ];
        let c = PlanClassification::classify(&prior, &current);
        assert_eq!(c.unchanged_rows.len(), 1);
        assert_eq!(c.unchanged_rows[0].plan, "unchanged");
        assert_eq!(c.changed.len(), 1);
        assert_eq!(c.changed[0].plan, "changed");
        assert_eq!(c.added.len(), 1);
        assert_eq!(c.added[0].plan, "added");
        assert_eq!(c.removed_keys, vec!["P/removed".to_string()]);
    }

    #[test]
    fn plans_to_analyse_concatenates_changed_then_added() {
        let prior = prior_with(vec![row("P", "c", "old")]);
        let current = vec![
            snapshot("P", "c", "new"),
            snapshot("P", "added", "fresh"),
        ];
        let c = PlanClassification::classify(&prior, &current);
        let to_analyse = c.plans_to_analyse();
        assert_eq!(to_analyse.len(), 2);
        let names: Vec<&str> = to_analyse.iter().map(|s| s.plan.as_str()).collect();
        assert!(names.contains(&"c"));
        assert!(names.contains(&"added"));
    }

    #[test]
    fn merge_delta_combines_unchanged_with_llm_rows_sorted() {
        // Classification: one unchanged, one changed; LLM returns the
        // changed row with a note. Merged result has both rows sorted
        // by (project, plan).
        let prior = prior_with(vec![
            row("P", "aaa-unchanged", "keep"),
            row("P", "zzz-changed", "old"),
        ]);
        let current_snapshots = vec![
            snapshot("P", "aaa-unchanged", "keep"),
            snapshot("P", "zzz-changed", "new"),
        ];
        let classification = PlanClassification::classify(&prior, &current_snapshots);

        let llm_delta = prior_with(vec![PlanRow {
            notes: "re-analysed".into(),
            ..row("P", "zzz-changed", "new")
        }]);
        let merged = merge_delta(classification, llm_delta).unwrap();
        assert_eq!(merged.plans.len(), 2);
        assert_eq!(merged.plans[0].plan, "aaa-unchanged");
        assert_eq!(merged.plans[1].plan, "zzz-changed");
        assert_eq!(merged.plans[1].notes, "re-analysed");
    }

    #[test]
    fn merge_delta_rejects_rows_outside_expected_set() {
        let prior = prior_with(vec![row("P", "a", "old")]);
        let current = vec![snapshot("P", "a", "new")];
        let classification = PlanClassification::classify(&prior, &current);

        // LLM hallucinated an extra row for a plan it wasn't asked about.
        let llm_delta = prior_with(vec![
            row("P", "a", "new"),
            row("P", "hallucinated", "anything"),
        ]);
        let err = merge_delta(classification, llm_delta).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("outside"), "got: {msg}");
        assert!(msg.contains("hallucinated"), "got: {msg}");
    }

    #[test]
    fn merge_delta_rejects_missing_expected_row() {
        let prior = prior_with(vec![row("P", "a", "old"), row("P", "b", "stale")]);
        let current = vec![snapshot("P", "a", "new"), snapshot("P", "b", "fresh")];
        let classification = PlanClassification::classify(&prior, &current);

        // LLM returned only one of the two changed rows.
        let llm_delta = prior_with(vec![row("P", "a", "new")]);
        let err = merge_delta(classification, llm_delta).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing"), "got: {msg}");
        assert!(msg.contains("P/b"), "got: {msg}");
    }

    #[test]
    fn merge_delta_uses_llm_annotation_sections_verbatim() {
        use super::super::schema::{Blocker, ParallelStream, Recommendation};
        let prior = prior_with(vec![row("P", "a", "old")]);
        let current = vec![snapshot("P", "a", "new")];
        let classification = PlanClassification::classify(&prior, &current);

        let mut llm_delta = prior_with(vec![row("P", "a", "new")]);
        llm_delta.cross_plan_blockers = vec![Blocker {
            blocked: "P/a".into(),
            blocker: "Q/z".into(),
            rationale: "new blocker".into(),
        }];
        llm_delta.parallel_streams = vec![ParallelStream {
            name: "s1".into(),
            plans: vec!["P/a".into()],
            rationale: "why".into(),
        }];
        llm_delta.recommended_invocation_order = vec![Recommendation {
            plan: "P/a".into(),
            order: 1,
            rationale: "go".into(),
        }];

        let merged = merge_delta(classification, llm_delta).unwrap();
        assert_eq!(merged.cross_plan_blockers.len(), 1);
        assert_eq!(merged.cross_plan_blockers[0].rationale, "new blocker");
        assert_eq!(merged.parallel_streams.len(), 1);
        assert_eq!(merged.recommended_invocation_order.len(), 1);
    }

    #[test]
    fn merge_delta_accepts_empty_delta_when_classification_empty() {
        // No changed/added plans: merged result is unchanged rows +
        // whatever annotations the LLM returned (in this case, empty —
        // but a noop caller would skip the LLM round-trip entirely).
        let prior = prior_with(vec![row("P", "a", "keep")]);
        let current = vec![snapshot("P", "a", "keep")];
        let classification = PlanClassification::classify(&prior, &current);
        assert!(classification.is_noop());

        let llm_delta = prior_with(vec![]);
        let merged = merge_delta(classification, llm_delta).unwrap();
        assert_eq!(merged.plans.len(), 1);
        assert_eq!(merged.plans[0].plan, "a");
    }
}
