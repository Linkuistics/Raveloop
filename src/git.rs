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

    Command::new("git")
        .current_dir(plan_dir)
        .args(["add", "."])
        .output()
        .context("Failed to run git add")?;

    let diff = Command::new("git")
        .current_dir(plan_dir)
        .args(["diff", "--cached", "--quiet"])
        .output()
        .context("Failed to run git diff")?;

    if diff.status.success() {
        return Ok(CommitResult {
            committed: false,
            message,
        });
    }

    Command::new("git")
        .current_dir(plan_dir)
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
        .current_dir(plan_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    let _ = fs::write(&baseline_path, &sha);
}

/// Paths that differ from `baseline_sha` in the working tree of
/// `project_dir`. Runs `git diff --name-only <baseline>`; untracked
/// files are NOT included — they're invisible to `diff` by definition
/// and the caller handles them as an always-included category.
///
/// Returned as a `HashSet` so callers can do O(1) membership tests
/// against porcelain paths when narrowing a dirty-tree warning.
pub fn paths_changed_since_baseline(
    project_dir: &Path,
    baseline_sha: &str,
) -> Result<std::collections::HashSet<String>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args(["diff", "--name-only", baseline_sha])
        .output()
        .context("Failed to run git diff --name-only")?;
    if !output.status.success() {
        anyhow::bail!(
            "git diff --name-only exited {} in {}",
            output.status,
            project_dir.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect())
}

/// Lines from `git status --porcelain` run from `project_dir`. Each entry
/// is the raw porcelain line including the two-character XY status prefix
/// — preserved so the caller can render them identically to what the user
/// would see if they ran `git status` themselves.
///
/// Used by the work-phase commit boundary as a postcondition: a clean
/// project tree after the work commit means the agent committed
/// everything it claimed; non-empty output means something was edited
/// but not committed (the silent-failure mode that masks lost work as
/// "backlog empty"). Returns `Ok(vec![])` on a clean tree.
pub fn working_tree_status(project_dir: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args(["status", "--porcelain"])
        .output()
        .context("Failed to run git status")?;
    if !output.status.success() {
        anyhow::bail!(
            "git status exited {} in {}",
            output.status,
            project_dir.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
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

/// Captures the work-tree state after the work phase exits so the
/// analyse-work prompt can inject it verbatim. Two parts:
///
/// 1. `git diff --stat <baseline>` — summarises every tracked-file
///    change since the baseline, committed or not.
/// 2. `git status --porcelain` — the raw list of uncommitted and
///    untracked paths, which catches new files that `diff --stat`
///    misses.
///
/// Soft-fails on a git error: returns a human-readable error string
/// rather than propagating, because the analyse-work prompt needs
/// *something* in the `WORK_TREE_STATUS` slot — an `Err` would bubble
/// up into `compose_prompt` and wedge the whole loop.
pub fn work_tree_snapshot(project_dir: &Path, baseline_sha: &str) -> String {
    let diff_stat = Command::new("git")
        .current_dir(project_dir)
        .args(["diff", "--stat", baseline_sha])
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            None
        });
    let status = Command::new("git")
        .current_dir(project_dir)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            None
        });

    let diff_block = match &diff_stat {
        Some(s) if !s.trim().is_empty() => s.trim_end().to_string(),
        Some(_) => "(no tracked-file changes since baseline)".to_string(),
        None => "(git diff --stat failed)".to_string(),
    };
    let status_block = match &status {
        Some(s) if !s.trim().is_empty() => s.trim_end().to_string(),
        Some(_) => "(clean — nothing uncommitted or untracked)".to_string(),
        None => "(git status failed)".to_string(),
    };

    format!(
        "Files changed since work baseline (git diff --stat {baseline_sha}):\n\
         {diff_block}\n\
         \n\
         Currently uncommitted or untracked (git status --porcelain):\n\
         {status_block}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_project_root_finds_git() {
        // This test runs inside a git repo (the ravel-lite project itself)
        let result = find_project_root(Path::new("."));
        assert!(result.is_ok());
    }

    #[test]
    fn find_project_root_errors_on_root() {
        let result = find_project_root(Path::new("/tmp/nonexistent-asdhjkasd"));
        assert!(result.is_err());
    }

    #[test]
    fn working_tree_status_reports_dirty_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        // git init must succeed before we can stage anything; minimal config so commits work.
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();

        // Untracked file shows up as ?? in porcelain output.
        fs::write(repo.join("dirty.txt"), "x").unwrap();
        let status = working_tree_status(repo).unwrap();
        assert!(
            status.iter().any(|l| l.contains("dirty.txt")),
            "expected dirty.txt in porcelain output, got: {status:?}"
        );
    }

    #[test]
    fn work_tree_snapshot_includes_tracked_changes_and_untracked_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();

        // Baseline commit: one tracked file.
        fs::write(repo.join("tracked.txt"), "v1\n").unwrap();
        Command::new("git").current_dir(repo).args(["add", "tracked.txt"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        let baseline = String::from_utf8(
            Command::new("git").current_dir(repo).args(["rev-parse", "HEAD"]).output().unwrap().stdout
        ).unwrap().trim().to_string();

        // Simulate a work phase: edit the tracked file (uncommitted) and
        // introduce a new untracked file.
        fs::write(repo.join("tracked.txt"), "v2\n").unwrap();
        fs::write(repo.join("untracked.rs"), "fn added() {}\n").unwrap();

        let snapshot = work_tree_snapshot(repo, &baseline);
        assert!(
            snapshot.contains("tracked.txt"),
            "diff --stat section should list edited tracked files, got:\n{snapshot}"
        );
        assert!(
            snapshot.contains("untracked.rs"),
            "status --porcelain section should list untracked files, got:\n{snapshot}"
        );
        assert!(
            snapshot.contains("git diff --stat"),
            "snapshot should label the diff section, got:\n{snapshot}"
        );
        assert!(
            snapshot.contains("git status --porcelain"),
            "snapshot should label the status section, got:\n{snapshot}"
        );
    }

    #[test]
    fn work_tree_snapshot_reports_clean_tree_with_no_changes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["commit", "-q", "--allow-empty", "-m", "empty"]).output().unwrap();
        let baseline = String::from_utf8(
            Command::new("git").current_dir(repo).args(["rev-parse", "HEAD"]).output().unwrap().stdout
        ).unwrap().trim().to_string();

        let snapshot = work_tree_snapshot(repo, &baseline);
        assert!(
            snapshot.contains("(no tracked-file changes since baseline)"),
            "clean diff should render an explicit empty-state marker, got:\n{snapshot}"
        );
        assert!(
            snapshot.contains("(clean — nothing uncommitted or untracked)"),
            "clean status should render an explicit empty-state marker, got:\n{snapshot}"
        );
    }

    #[test]
    fn paths_changed_since_baseline_returns_tracked_modifications() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();

        fs::write(repo.join("a.txt"), "v1\n").unwrap();
        fs::write(repo.join("b.txt"), "v1\n").unwrap();
        Command::new("git").current_dir(repo).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        let baseline = String::from_utf8(
            Command::new("git").current_dir(repo).args(["rev-parse", "HEAD"]).output().unwrap().stdout
        ).unwrap().trim().to_string();

        // One tracked file modified (uncommitted), one untracked file added.
        fs::write(repo.join("a.txt"), "v2\n").unwrap();
        fs::write(repo.join("c.txt"), "new\n").unwrap();

        let changed = paths_changed_since_baseline(repo, &baseline).unwrap();
        assert!(changed.contains("a.txt"), "modified tracked file should be in set, got {changed:?}");
        assert!(!changed.contains("b.txt"), "untouched file must not be in set, got {changed:?}");
        assert!(!changed.contains("c.txt"), "untracked file is diff-invisible, got {changed:?}");
    }

    #[test]
    fn working_tree_status_empty_on_clean_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();
        // Empty repo with no untracked files — porcelain output should be empty.
        let status = working_tree_status(repo).unwrap();
        assert!(status.is_empty(), "expected empty status on clean tree, got: {status:?}");
    }
}
