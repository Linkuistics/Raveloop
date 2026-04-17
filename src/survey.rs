// src/survey.rs
//
// Multi-project plan status survey. Gathers `phase.md`, `backlog.md`,
// and `memory.md` from every plan directory under one or more roots,
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
mod discover;
mod invoke;
mod render;
mod schema;

// `main.rs` declares `mod survey;` of its own, so the binary crate
// compiles this file alongside the library crate. These re-exports
// are the library's public API (reached via `raveloop::survey::*`
// from integration tests); the binary never touches them. Silence
// the bin-side `unused_imports` warning so `deny(warnings)` doesn't
// reject the build.
#[allow(unused_imports)]
pub use compose::{load_survey_prompt, render_survey_input};
#[allow(unused_imports)]
pub use discover::{PlanSnapshot, discover_plans};
pub use invoke::run_survey;
