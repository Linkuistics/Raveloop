//! Canonical filenames for plan-state files.
//!
//! A plan directory holds well-known files at fixed names. Centralising
//! those names here makes a future rename a single-file change. Use
//! these constants instead of literal strings whenever the filename
//! appears in source, prompts, comments, or tests.

pub const BACKLOG_FILENAME: &str = "backlog.yaml";
pub const MEMORY_FILENAME: &str = "memory.yaml";
pub const SESSION_LOG_FILENAME: &str = "session-log.yaml";
pub const LATEST_SESSION_FILENAME: &str = "latest-session.yaml";
pub const SUBAGENT_DISPATCH_FILENAME: &str = "subagent-dispatch.yaml";
pub const COMMITS_FILENAME: &str = "commits.yaml";
pub const PHASE_FILENAME: &str = "phase.md";
pub const RELATED_COMPONENTS_FILENAME: &str = "related-components.yaml";
pub const DREAM_WORD_COUNT_FILENAME: &str = "dream-word-count";
