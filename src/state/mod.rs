//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase` — `set-phase` (existing)
//! - `backlog` — typed backlog.yaml + CRUD verbs (R1)
//! - `memory` — typed memory.yaml + per-entry CRUD verbs (R2)
//! - `session_log` — typed session-log.yaml + latest-session.yaml
//!   verbs, plus the programmatic append used by
//!   `phase_loop::GitCommitWork` (R3)
//! - `migrate` — one-shot per-plan .md → .yaml conversion
//!   (backlog + memory + session-log/latest-session)

pub mod backlog;
pub mod memory;
pub mod migrate;
pub mod phase;
pub mod session_log;

pub use phase::run_set_phase;
