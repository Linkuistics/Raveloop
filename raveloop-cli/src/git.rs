// src/git.rs
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub struct CommitResult {
    pub committed: bool,
    pub message: String,
}

/// Stage plan directory and commit with the message from commit-message.md
/// (or a default message). Returns whether anything was committed.
pub fn git_commit_plan(plan_dir: &Path, plan_name: &str, phase_name: &str) -> Result<CommitResult> {
    let commit_msg_path = plan_dir.join("commit-message.md");
    let message = if commit_msg_path.exists() {
        let msg = fs::read_to_string(&commit_msg_path)
            .context("Failed to read commit-message.md")?
            .trim()
            .to_string();
        fs::remove_file(&commit_msg_path).ok();
        msg
    } else {
        format!("run-plan: {phase_name} ({plan_name})")
    };

    // Stage the plan directory
    Command::new("git")
        .args(["add", &plan_dir.to_string_lossy()])
        .output()
        .context("Failed to run git add")?;

    // Check if there are staged changes
    let diff = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()
        .context("Failed to run git diff")?;

    if diff.status.success() {
        // Exit code 0 means no changes
        return Ok(CommitResult {
            committed: false,
            message,
        });
    }

    // Commit
    Command::new("git")
        .args(["commit", "-m", &message])
        .output()
        .context("Failed to run git commit")?;

    Ok(CommitResult {
        committed: true,
        message,
    })
}

/// Save the current HEAD sha as the work baseline.
pub fn git_save_work_baseline(plan_dir: &Path) {
    let baseline_path = plan_dir.join("work-baseline");
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    let _ = fs::write(&baseline_path, &sha);
}

/// Find the project root by walking up from a directory to find .git.
pub fn find_project_root(start_dir: &Path) -> Result<String> {
    let mut dir = start_dir.canonicalize().unwrap_or_else(|_| start_dir.to_path_buf());
    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_string_lossy().to_string());
        }
        if !dir.pop() {
            anyhow::bail!("No .git found above {}", start_dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_project_root_finds_git() {
        // This test runs inside a git repo (the raveloop-cli project itself)
        let result = find_project_root(Path::new("."));
        assert!(result.is_ok());
    }

    #[test]
    fn find_project_root_errors_on_root() {
        let result = find_project_root(Path::new("/tmp/nonexistent-asdhjkasd"));
        assert!(result.is_err());
    }
}
