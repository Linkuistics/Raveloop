//! Canonical filenames for plan-state files.
//!
//! A plan directory holds four typed YAML files at well-known names.
//! Centralising those names here makes a future rename a single-file
//! change. Use these constants instead of literal strings whenever the
//! filename appears in source, prompts, comments, or tests.

pub const BACKLOG_FILENAME: &str = "backlog.yaml";
pub const MEMORY_FILENAME: &str = "memory.yaml";
pub const SESSION_LOG_FILENAME: &str = "session-log.yaml";
pub const LATEST_SESSION_FILENAME: &str = "latest-session.yaml";
