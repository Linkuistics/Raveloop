// src/survey.rs
//
// Multi-project plan status survey. Gathers `phase.md`, `backlog.yaml`,
// and `memory.yaml` from every plan directory under one or more roots,
// renders them as a single prompt, and shells out to a headless
// `claude` session for LLM-driven summarisation and prioritisation.
//
// The command is intentionally single-shot and read-only: no tool use,
// no file writes, no session persistence. Fresh context per invocation
// by construction.
//
// Module layout:
//   discover — walk + classify plans (project name from nearest .git)
//   compose  — plan bundle → prompt text + prompt template loader
//   schema   — YAML response types + tolerant parser
//   render   — deterministic human-readable output
//   invoke   — spawn claude, orchestrate end-to-end `run_survey`

mod compose;
mod delta;
mod discover;
mod invoke;
mod render;
mod schema;

pub use compose::{
    load_survey_incremental_prompt, load_survey_prompt, render_survey_input,
    render_survey_input_incremental,
};
pub use delta::{merge_delta, PlanClassification};
pub use discover::{PlanSnapshot, load_plan};
pub use schema::{
    emit_survey_yaml, inject_input_hashes, parse_survey_response, plan_key, PlanRow,
    SurveyResponse,
};
pub use render::render_survey_output;
pub use invoke::{compute_survey_response, run_survey, run_survey_format};
