//! `state discover-proposals <verb>` CLI verbs.
//!
//! Unlike the other `state/` areas, the proposals schema and storage
//! path live in `src/discover/` — this module is a thin typed-CLI façade
//! whose purpose is to reject ill-formed Stage 2 LLM output at
//! emission time. The established pattern (phase prompts use
//! `ravel-lite state` verbs rather than writing YAML directly) extends
//! here: Stage 2's LLM calls `add-proposal` once per edge, and a single
//! hallucinated `--kind` argument fails only that call rather than
//! nuking an entire batched YAML document.

pub mod verbs;

pub use verbs::{run_add_proposal, AddProposalRequest};
