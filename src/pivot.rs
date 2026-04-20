//! Stack-based pivot mechanism for nested plan execution.
//!
//! When a plan's work phase appends a frame to `<root>/stack.yaml`,
//! the driver pushes the new top onto an in-memory `Vec<PlanContext>`
//! and runs a full cycle of the target plan before popping back.
//!
//! This module owns:
//! - `Frame` / `Stack` — the on-disk schema.
//! - `read_stack` / `write_stack` — file I/O.
//! - `validate_push` — depth cap + cycle detection + target validity.
//! - `decide_after_work` / `decide_after_cycle` — pure state transitions.
//!
//! No tokio, no async. The async driver in `phase_loop.rs` orchestrates.
//! See docs/superpowers/specs/2026-04-20-hierarchical-pivot-design.md.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Hard compile-time cap on nesting depth. Prevents runaway recursion
/// from a buggy coordinator prompt.
pub const MAX_STACK_DEPTH: usize = 5;

/// One entry in `stack.yaml`. Path is the only required field; `pushed_at`
/// and `reason` are informational and appear in the TUI / session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The on-disk stack. Ordered; `frames.last()` is the currently-executing plan.
/// A stack with `len <= 1` means "just the root"; the file is normally
/// deleted in that state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stack {
    #[serde(default)]
    pub frames: Vec<Frame>,
}

/// Read `stack.yaml` from `path`. Returns `Ok(None)` if the file is absent
/// (normal state at depth 1), `Ok(Some(stack))` on successful parse, or an
/// error that includes the path for diagnosability.
pub fn read_stack(path: &Path) -> Result<Option<Stack>> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let stack: Stack = serde_yaml::from_str(&s)
                .with_context(|| format!("Failed to parse stack.yaml at {}", path.display()))?;
            Ok(Some(stack))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => {
            Err(e).with_context(|| format!("Failed to read stack.yaml at {}", path.display()))
        }
    }
}

/// Write `stack` to `path`, creating parent directories as needed.
pub fn write_stack(path: &Path, stack: &Stack) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent for {}", path.display()))?;
    }
    let s = serde_yaml::to_string(stack).context("Failed to serialize Stack")?;
    fs::write(path, s)
        .with_context(|| format!("Failed to write stack.yaml at {}", path.display()))
}
