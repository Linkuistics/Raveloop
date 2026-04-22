//! LLM-driven discovery of cross-project relationships.
//!
//! Two-stage pipeline keyed from the global projects catalog:
//! * Stage 1 (per-project, cached): subagent reads the project tree and
//!   emits a structured interaction-surface record.
//! * Stage 2 (global, uncached): one LLM call over all N surface records
//!   proposes edges, written to `<config-dir>/discover-proposals.yaml`
//!   for review.
//!
//! Spec: `docs/r7-related-projects-discovery-design.md`.

pub mod schema;
