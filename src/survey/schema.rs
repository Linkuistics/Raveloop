// src/survey/schema.rs
//
// Typed YAML schema for the LLM's survey response, plus a tolerant
// parser that accepts an optional ```yaml / ``` code fence around the
// document (some models emit one despite instructions otherwise).

use anyhow::{Context, Result};

use crate::state::backlog::{PlanRowCounts, TaskCounts};

/// Canonical schema version for emitted and consumed YAML. Bumped only
/// when a struct-level change would make prior YAML deserialise into a
/// subtly-different meaning. Prior YAML with a mismatched explicit
/// version is rejected at `--prior` load time with a remediation hint
/// pointing at `--force`.
///
/// Emitted YAML always carries this value explicitly. Prior YAML
/// missing the field (emitted before 5b landed) defaults to this same
/// value via `default_schema_version`, giving the 5a→5b transition a
/// one-time amnesty: pre-existing surveys are treated as v1-compatible.
/// Once a v2 ships, the field's presence on v1-emitted YAML is what
/// makes the mismatch detectable.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// Typed deserialisation target for the YAML document the LLM emits,
/// and the canonical serialisation form written back to disk. The LLM
/// does classification and reasoning; the tool owns rendering.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SurveyResponse {
    /// Schema version marker. `#[serde(default = "default_schema_version")]`
    /// lets 5a-emitted YAML (which predates this field) deserialise
    /// cleanly, treated as the current version for the one-time 5a→5b
    /// transition. Serialisation always emits it explicitly, so future
    /// version bumps can fail-fast on actually-stale YAML.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub plans: Vec<PlanRow>,
    #[serde(default)]
    pub cross_plan_blockers: Vec<Blocker>,
    #[serde(default)]
    pub parallel_streams: Vec<ParallelStream>,
    #[serde(default)]
    pub recommended_invocation_order: Vec<Recommendation>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct PlanRow {
    pub project: String,
    pub plan: String,
    pub phase: String,
    /// Injected by `inject_plan_row_counts` from the plan's parsed
    /// `backlog.yaml` — the LLM's emitted value is overwritten. Kept
    /// in the schema for wire compatibility with prior surveys and
    /// because downstream render still reads it. `#[serde(default)]`
    /// lets the LLM omit the field entirely once it drops out of the
    /// prompt contract.
    #[serde(default)]
    pub unblocked: usize,
    /// Injected by `inject_plan_row_counts`. See `unblocked`.
    #[serde(default)]
    pub blocked: usize,
    pub done: usize,
    /// Injected by `inject_plan_row_counts`. See `unblocked`.
    #[serde(default)]
    pub received: usize,
    #[serde(default)]
    pub notes: String,
    /// SHA-256 hex digest over `phase.md` + `backlog.yaml` + `memory.yaml`
    /// + `related-plans.md` contents.
    ///
    /// Computed entirely in Rust and injected into each row after the
    /// LLM's response is parsed (matched by `project` + `plan`). The
    /// LLM never sees or emits this field — `#[serde(default)]` lets
    /// LLM YAML without the field deserialise, leaving the hash empty
    /// until injected.
    ///
    /// Forward-compat seam for 5b's incremental-survey change
    /// detection.
    #[serde(default)]
    pub input_hash: String,
    /// Per-status task tally computed from the plan's parsed
    /// `backlog.yaml` by `BacklogFile::task_counts`. Injected after the
    /// LLM's response is parsed so the model never has to count tasks
    /// itself — mechanical work belongs in Rust.
    ///
    /// `None` when `backlog.yaml` is absent or fails to parse; the
    /// survey notes surface that as "backlog.yaml missing" or
    /// "backlog.yaml unparseable".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_counts: Option<TaskCounts>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Blocker {
    pub blocked: String,
    pub blocker: String,
    pub rationale: String,
}

/// A group of plans whose work can proceed concurrently with other
/// groups. Within a stream, work may still be sequential (gates,
/// dependencies) — that's what `rationale` explains.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct ParallelStream {
    pub name: String,
    pub plans: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Recommendation {
    pub plan: String,
    /// Priority rank. Multiple recommendations sharing the same `order`
    /// are mutually parallelisable — start them in any order, they
    /// don't block each other. Smaller numbers come before larger
    /// numbers. Within a shared number, list position expresses a
    /// secondary ranking (earlier entries unblock more downstream).
    pub order: usize,
    pub rationale: String,
}

/// Parse the LLM's stdout as a YAML survey response. Tolerates an
/// optional ```yaml / ``` code fence that some models emit despite
/// instructions otherwise.
pub fn parse_survey_response(stdout: &str) -> Result<SurveyResponse> {
    let content = strip_code_fence(stdout);
    serde_yaml::from_str(content).with_context(|| {
        format!(
            "Failed to parse survey response as YAML. Raw output from claude:\n---\n{stdout}\n---"
        )
    })
}

/// Serialise a `SurveyResponse` to canonical YAML. Re-emission via
/// `serde_yaml::to_string` proves every written document round-trips
/// through the typed schema — two emissions of the same struct
/// produce byte-identical YAML.
pub fn emit_survey_yaml(response: &SurveyResponse) -> Result<String> {
    serde_yaml::to_string(response).context("Failed to serialise SurveyResponse as YAML")
}

/// Inject the Rust-computed `input_hash` into every `PlanRow`, matched
/// by `(project, plan)`. Every plan in `response.plans` must have a
/// matching entry in `hashes_by_plan_key`; any mismatch is a hard
/// error because it indicates either an LLM drift (a row for a plan
/// we didn't discover) or a discovery bug (a plan we gathered but the
/// LLM dropped). Both are failure modes the user should see loudly
/// rather than a half-populated hash slipping into persisted YAML.
///
/// Returns the count of rows whose hash was injected. A non-empty
/// `hashes_by_plan_key` map with no matching rows is also an error —
/// the LLM must include every discovered plan per the prompt contract.
pub fn inject_input_hashes(
    response: &mut SurveyResponse,
    hashes_by_plan_key: &std::collections::HashMap<String, String>,
) -> Result<usize> {
    let mut injected = 0;
    for row in &mut response.plans {
        let key = plan_key(&row.project, &row.plan);
        let hash = hashes_by_plan_key.get(&key).with_context(|| {
            format!(
                "survey response contains plan {key} that was not discovered. \
                 The LLM returned a plan row we did not supply — this usually \
                 means the model hallucinated a plan identifier."
            )
        })?;
        row.input_hash = hash.clone();
        injected += 1;
    }
    if injected != hashes_by_plan_key.len() {
        let response_keys: std::collections::HashSet<String> = response
            .plans
            .iter()
            .map(|r| plan_key(&r.project, &r.plan))
            .collect();
        let missing: Vec<&String> = hashes_by_plan_key
            .keys()
            .filter(|k| !response_keys.contains(*k))
            .collect();
        anyhow::bail!(
            "survey response is missing {} discovered plan(s) — the prompt \
             contract requires every discovered plan to appear in the response. \
             Missing: {:?}",
            missing.len(),
            missing
        );
    }
    Ok(injected)
}

/// Inject Rust-computed `task_counts` into every `PlanRow` that has a
/// matching entry in `counts_by_plan_key`. Rows whose key is absent
/// from the map are left with `task_counts = None` — this is the
/// missing-/unparseable-backlog case, surfaced via the LLM's `notes`
/// rather than as a hard error.
///
/// Unlike `inject_input_hashes`, this is NOT a drift check: the map
/// only carries entries for plans whose `backlog.yaml` parsed
/// successfully, so a missing key is an expected state rather than a
/// contract violation.
pub fn inject_task_counts(
    response: &mut SurveyResponse,
    counts_by_plan_key: &std::collections::HashMap<String, TaskCounts>,
) {
    for row in &mut response.plans {
        let key = plan_key(&row.project, &row.plan);
        if let Some(counts) = counts_by_plan_key.get(&key) {
            row.task_counts = Some(*counts);
        }
    }
}

/// Inject Rust-computed `unblocked`, `blocked`, and `received` counts
/// into every `PlanRow` that has a matching entry in
/// `counts_by_plan_key`. Rows whose key is absent from the map are
/// left at their deserialised values — which for an LLM response that
/// follows the post-extraction prompt will be zeros via
/// `#[serde(default)]`. Silent skip follows the same convention as
/// `inject_task_counts`: a missing backlog surfaces to the user
/// through the LLM's `notes`, not through a hard error here.
pub fn inject_plan_row_counts(
    response: &mut SurveyResponse,
    counts_by_plan_key: &std::collections::HashMap<String, PlanRowCounts>,
) {
    for row in &mut response.plans {
        let key = plan_key(&row.project, &row.plan);
        if let Some(counts) = counts_by_plan_key.get(&key) {
            row.unblocked = counts.unblocked;
            row.blocked = counts.blocked;
            row.received = counts.received;
        }
    }
}

/// Canonical `project/plan` key string used to match discovered plans
/// to rows in the LLM's response.
pub fn plan_key(project: &str, plan: &str) -> String {
    format!("{project}/{plan}")
}

/// Strip a leading ```yaml or ``` fence and matching trailing fence,
/// if present. Otherwise returns the input trimmed of outer whitespace.
fn strip_code_fence(s: &str) -> &str {
    let trimmed = s.trim();
    let body = trimmed
        .strip_prefix("```yaml")
        .or_else(|| trimmed.strip_prefix("```yml"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let body = body.trim_start_matches('\n').trim_end();
    body.strip_suffix("```").unwrap_or(body).trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
plans:
  - project: Ravel
    plan: sub-A-global-store
    phase: work
    unblocked: 1
    blocked: 15
    done: 0
    received: 0
    notes: Task 0 gate unblocked

cross_plan_blockers:
  - blocked: Ravel/sub-F-hierarchy
    blocker: Ravel/sub-B-phase-cycle
    rationale: |
      Sub-B's downstream task list must be rewritten before
      Sub-F's Task 0 readiness gate can fire.

recommended_invocation_order:
  - plan: Ravel/ravel-orchestrator
    order: 1
    rationale: Dispatch Sub-C work-phase cycle.
"#
    }

    #[test]
    fn parse_survey_response_parses_valid_yaml() {
        let resp = parse_survey_response(sample_yaml()).unwrap();
        assert_eq!(resp.plans.len(), 1);
        assert_eq!(resp.plans[0].plan, "sub-A-global-store");
        assert_eq!(resp.plans[0].unblocked, 1);
        assert_eq!(resp.cross_plan_blockers.len(), 1);
        assert!(resp.cross_plan_blockers[0].rationale.contains("readiness gate"));
        assert_eq!(resp.recommended_invocation_order.len(), 1);
    }

    #[test]
    fn parse_survey_response_strips_yaml_code_fence() {
        let wrapped = format!("```yaml\n{}\n```\n", sample_yaml().trim());
        let resp = parse_survey_response(&wrapped).unwrap();
        assert_eq!(resp.plans.len(), 1);
    }

    #[test]
    fn parse_survey_response_strips_plain_code_fence() {
        let wrapped = format!("```\n{}\n```\n", sample_yaml().trim());
        let resp = parse_survey_response(&wrapped).unwrap();
        assert_eq!(resp.plans.len(), 1);
    }

    #[test]
    fn parse_survey_response_errors_include_raw_output_for_debugging() {
        let malformed = "not: valid: yaml: at: all:\n  - [";
        let err = parse_survey_response(malformed).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Raw output from claude"));
    }

    #[test]
    fn parse_survey_response_allows_missing_optional_sections() {
        let minimal = r#"
plans:
  - project: P
    plan: x
    phase: work
    unblocked: 0
    blocked: 0
    done: 0
    received: 0
"#;
        let resp = parse_survey_response(minimal).unwrap();
        assert!(resp.cross_plan_blockers.is_empty());
        assert!(resp.parallel_streams.is_empty());
        assert!(resp.recommended_invocation_order.is_empty());
        assert_eq!(resp.plans[0].notes, "");
    }

    #[test]
    fn emit_survey_yaml_round_trips_byte_identical() {
        // Parse once, serialise, parse the serialised form, serialise
        // again: the second emission must be byte-identical to the first.
        // This is the central invariant re-emission buys us — every
        // persisted survey YAML is guaranteed parseable by the same
        // struct that emitted it, and re-emission is idempotent.
        let resp = parse_survey_response(sample_yaml()).unwrap();
        let first = emit_survey_yaml(&resp).unwrap();
        let resp2 = parse_survey_response(&first).unwrap();
        let second = emit_survey_yaml(&resp2).unwrap();
        assert_eq!(first, second, "two emissions should be byte-identical");
    }

    #[test]
    fn emit_survey_yaml_is_parseable_as_survey_response() {
        let resp = parse_survey_response(sample_yaml()).unwrap();
        let emitted = emit_survey_yaml(&resp).unwrap();
        // Emitted YAML does NOT have a code fence — it's canonical.
        assert!(!emitted.starts_with("```"));
        let reparsed = parse_survey_response(&emitted).unwrap();
        assert_eq!(resp, reparsed);
    }

    #[test]
    fn emit_survey_yaml_preserves_injected_input_hash() {
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        let mut hashes = std::collections::HashMap::new();
        hashes.insert("Ravel/sub-A-global-store".to_string(), "deadbeef".to_string());
        inject_input_hashes(&mut resp, &hashes).unwrap();
        let emitted = emit_survey_yaml(&resp).unwrap();
        assert!(emitted.contains("input_hash: deadbeef"), "emitted: {emitted}");
        let reparsed = parse_survey_response(&emitted).unwrap();
        assert_eq!(reparsed.plans[0].input_hash, "deadbeef");
    }

    #[test]
    fn inject_input_hashes_errors_when_response_row_has_no_match() {
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        // Deliberately provide no hash for the row in the response —
        // the LLM "hallucinated a plan identifier" case.
        let hashes = std::collections::HashMap::new();
        let err = inject_input_hashes(&mut resp, &hashes).unwrap_err();
        assert!(format!("{err:#}").contains("not discovered"));
    }

    #[test]
    fn inject_input_hashes_errors_when_discovered_plan_is_missing_from_response() {
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        let mut hashes = std::collections::HashMap::new();
        hashes.insert("Ravel/sub-A-global-store".to_string(), "hash-a".to_string());
        hashes.insert("Ravel/sub-B-missing".to_string(), "hash-b".to_string());
        let err = inject_input_hashes(&mut resp, &hashes).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing"), "got: {msg}");
        assert!(msg.contains("sub-B-missing"), "got: {msg}");
    }

    #[test]
    fn plan_key_joins_project_and_plan_with_slash() {
        assert_eq!(plan_key("Ravel", "sub-A"), "Ravel/sub-A");
    }

    #[test]
    fn inject_task_counts_populates_matching_rows_and_skips_unmatched() {
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        let mut counts = std::collections::HashMap::new();
        counts.insert(
            "Ravel/sub-A-global-store".to_string(),
            TaskCounts {
                total: 16,
                not_started: 15,
                in_progress: 0,
                done: 1,
                blocked: 0,
            },
        );
        inject_task_counts(&mut resp, &counts);
        let injected = resp.plans[0].task_counts.unwrap();
        assert_eq!(injected.total, 16);
        assert_eq!(injected.not_started, 15);
        assert_eq!(injected.done, 1);

        // Unmatched map entries are silently ignored; absence is not a
        // hard error (unlike `inject_input_hashes`).
        let mut unrelated = std::collections::HashMap::new();
        unrelated.insert(
            "Other/plan".to_string(),
            TaskCounts { total: 1, ..Default::default() },
        );
        let mut resp2 = parse_survey_response(sample_yaml()).unwrap();
        inject_task_counts(&mut resp2, &unrelated);
        assert!(resp2.plans[0].task_counts.is_none());
    }

    #[test]
    fn inject_plan_row_counts_overwrites_llm_emitted_fields() {
        // LLM-emitted values in sample_yaml have unblocked: 1, blocked: 15.
        // Injection must overwrite them with the Rust-computed values so
        // drift between LLM arithmetic and ground truth cannot slip
        // through to the persisted survey.
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        assert_eq!(resp.plans[0].unblocked, 1);
        assert_eq!(resp.plans[0].blocked, 15);
        assert_eq!(resp.plans[0].received, 0);

        let mut counts = std::collections::HashMap::new();
        counts.insert(
            "Ravel/sub-A-global-store".to_string(),
            PlanRowCounts { unblocked: 3, blocked: 7, received: 2 },
        );
        inject_plan_row_counts(&mut resp, &counts);
        assert_eq!(resp.plans[0].unblocked, 3);
        assert_eq!(resp.plans[0].blocked, 7);
        assert_eq!(resp.plans[0].received, 2);
    }

    #[test]
    fn inject_plan_row_counts_leaves_unmatched_rows_untouched() {
        // A map entry for an unrelated plan must not write into the
        // response's row, and a row whose key has no map entry must
        // carry whatever value it already held.
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        let before = (
            resp.plans[0].unblocked,
            resp.plans[0].blocked,
            resp.plans[0].received,
        );
        let mut unrelated = std::collections::HashMap::new();
        unrelated.insert(
            "Other/plan".to_string(),
            PlanRowCounts { unblocked: 99, blocked: 99, received: 99 },
        );
        inject_plan_row_counts(&mut resp, &unrelated);
        assert_eq!(
            (
                resp.plans[0].unblocked,
                resp.plans[0].blocked,
                resp.plans[0].received,
            ),
            before,
            "an unrelated map entry must not overwrite the row"
        );
    }

    #[test]
    fn parse_survey_response_defaults_unblocked_blocked_received_when_absent() {
        // Once the extraction's prompt change lands, the LLM may omit
        // the three injected fields entirely. `#[serde(default)]` must
        // let those rows deserialise cleanly; the values are then
        // filled in by `inject_plan_row_counts`.
        let yaml = r#"
plans:
  - project: P
    plan: x
    phase: work
    done: 0
"#;
        let resp = parse_survey_response(yaml).unwrap();
        assert_eq!(resp.plans[0].unblocked, 0);
        assert_eq!(resp.plans[0].blocked, 0);
        assert_eq!(resp.plans[0].received, 0);
    }

    #[test]
    fn task_counts_round_trip_through_emitted_survey_yaml() {
        let mut resp = parse_survey_response(sample_yaml()).unwrap();
        let mut counts = std::collections::HashMap::new();
        counts.insert(
            "Ravel/sub-A-global-store".to_string(),
            TaskCounts {
                total: 3,
                not_started: 1,
                in_progress: 1,
                done: 1,
                blocked: 0,
            },
        );
        inject_task_counts(&mut resp, &counts);
        let emitted = emit_survey_yaml(&resp).unwrap();
        assert!(emitted.contains("task_counts:"), "emitted: {emitted}");
        let reparsed = parse_survey_response(&emitted).unwrap();
        assert_eq!(reparsed.plans[0].task_counts.unwrap().total, 3);
        assert_eq!(reparsed.plans[0].task_counts.unwrap().in_progress, 1);
    }

    #[test]
    fn parse_survey_response_defaults_schema_version_when_field_absent() {
        // 5a-emitted YAML has no `schema_version` field. The one-time
        // 5a→5b amnesty: absent field deserialises to the current
        // SCHEMA_VERSION so pre-existing surveys load as compatible.
        let yaml = r#"
plans:
  - project: P
    plan: x
    phase: work
    unblocked: 0
    blocked: 0
    done: 0
    received: 0
"#;
        let resp = parse_survey_response(yaml).unwrap();
        assert_eq!(resp.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn parse_survey_response_preserves_explicit_schema_version() {
        // A future v2 survey with explicit marker parses to that value,
        // so a v1 binary can detect the mismatch at `--prior` load time.
        let yaml = r#"
schema_version: 2
plans:
  - project: P
    plan: x
    phase: work
    unblocked: 0
    blocked: 0
    done: 0
    received: 0
"#;
        let resp = parse_survey_response(yaml).unwrap();
        assert_eq!(resp.schema_version, 2);
    }

    #[test]
    fn emit_survey_yaml_writes_schema_version_explicitly() {
        // Emitted YAML always carries the marker — that's what makes
        // future-version mismatches detectable when that YAML is
        // later fed to a binary on a different SCHEMA_VERSION.
        let resp = parse_survey_response(sample_yaml()).unwrap();
        let emitted = emit_survey_yaml(&resp).unwrap();
        assert!(
            emitted.contains(&format!("schema_version: {SCHEMA_VERSION}")),
            "emitted YAML is missing schema_version marker:\n{emitted}"
        );
    }

    #[test]
    fn parse_survey_response_parses_parallel_streams() {
        let yaml = r#"
plans:
  - project: P
    plan: a
    phase: work
    unblocked: 1
    blocked: 0
    done: 0
    received: 0

parallel_streams:
  - name: Critical path
    plans:
      - P/a
      - P/b
    rationale: |
      Sequential chain within stream.
  - name: Independent research
    plans: [P/c]
    rationale: No cross-project dependencies.
"#;
        let resp = parse_survey_response(yaml).unwrap();
        assert_eq!(resp.parallel_streams.len(), 2);
        assert_eq!(resp.parallel_streams[0].name, "Critical path");
        assert_eq!(resp.parallel_streams[0].plans, vec!["P/a", "P/b"]);
        assert!(resp.parallel_streams[0].rationale.contains("Sequential chain"));
        assert_eq!(resp.parallel_streams[1].plans, vec!["P/c"]);
    }
}
