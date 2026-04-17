// src/survey/schema.rs
//
// Typed YAML schema for the LLM's survey response, plus a tolerant
// parser that accepts an optional ```yaml / ``` code fence around the
// document (some models emit one despite instructions otherwise).

use anyhow::{Context, Result};

/// Typed deserialisation target for the YAML document the LLM emits.
/// The LLM does classification and reasoning; the tool owns rendering.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct SurveyResponse {
    pub plans: Vec<PlanRow>,
    #[serde(default)]
    pub cross_plan_blockers: Vec<Blocker>,
    #[serde(default)]
    pub parallel_streams: Vec<ParallelStream>,
    #[serde(default)]
    pub recommended_invocation_order: Vec<Recommendation>,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct PlanRow {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub unblocked: usize,
    pub blocked: usize,
    pub done: usize,
    pub received: usize,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct Blocker {
    pub blocked: String,
    pub blocker: String,
    pub rationale: String,
}

/// A group of plans whose work can proceed concurrently with other
/// groups. Within a stream, work may still be sequential (gates,
/// dependencies) — that's what `rationale` explains.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct ParallelStream {
    pub name: String,
    pub plans: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
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
  - project: Mnemosyne
    plan: sub-A-global-store
    phase: work
    unblocked: 1
    blocked: 15
    done: 0
    received: 0
    notes: Task 0 gate unblocked

cross_plan_blockers:
  - blocked: Mnemosyne/sub-F-hierarchy
    blocker: Mnemosyne/sub-B-phase-cycle
    rationale: |
      Sub-B's downstream task list must be rewritten before
      Sub-F's Task 0 readiness gate can fire.

recommended_invocation_order:
  - plan: Mnemosyne/mnemosyne-orchestrator
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
