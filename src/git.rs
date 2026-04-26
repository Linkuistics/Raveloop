// src/git.rs
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

#[cfg(test)]
use crate::state::filenames::BACKLOG_FILENAME;

pub struct CommitResult {
    pub committed: bool,
    pub message: String,
}

/// One entry in `commits.yaml`: a pathspec list and the commit message that
/// should be applied to those paths. Pathspec strings are passed verbatim to
/// `git add`, so standard git pathspec features (globs like `src/**`,
/// exclusions like `:!src/generated/`) all work.
#[derive(Debug, Deserialize)]
pub struct CommitSpec {
    pub paths: Vec<String>,
    pub message: String,
}

/// Shape of `commits.yaml` — an ordered list of `CommitSpec` entries that
/// together partition the work-tree diff into logical commits. Analyse-work
/// authors this file; `git-commit-work` applies it and removes it.
#[derive(Debug, Deserialize)]
pub struct CommitsSpec {
    pub commits: Vec<CommitSpec>,
}

/// Stage the plan directory and commit it with the default message
/// `run-plan: {phase_name} ({plan_name})`. Used by the
/// reflect / dream / triage / save-*-baseline phases, none of which
/// produce a custom commit message. The work-phase commit is handled
/// separately by `apply_commits_spec` against `commits.yaml`.
/// Returns whether anything was committed.
pub fn git_commit_plan(plan_dir: &Path, plan_name: &str, phase_name: &str) -> Result<CommitResult> {
    let message = format!("run-plan: {phase_name} ({plan_name})");

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

/// Apply the ordered commit spec at `{plan_dir}/commits.yaml` to
/// `project_dir`, producing one commit per spec entry whose paths match
/// a real change.
///
/// Each spec entry is applied as:
/// 1. `git reset HEAD -- <project_dir>` — unstage anything left over
///    from a prior entry within the subtree.
/// 2. `git add -- <paths…>` — stage the entry's pathspecs.
/// 3. `git commit -m <message>` — commit if anything is staged.
///
/// All git invocations run with `current_dir(project_dir)`, so pathspecs
/// in the spec are resolved relative to the subtree root and a monorepo's
/// sibling subtrees remain invisible.
///
/// Fallback: when `commits.yaml` is missing, empty, or fails to parse, a
/// single catch-all commit covers the whole subtree with the default
/// message `run-plan: {phase_name} ({plan_name})`. This preserves
/// backwards-compat with the single-commit-per-phase behaviour.
///
/// Side effect: the spec file is removed after a successful apply so
/// subsequent phases aren't re-reading stale instructions. The file is
/// intentionally one-shot scratch.
pub fn apply_commits_spec(
    project_dir: &Path,
    plan_dir: &Path,
    plan_name: &str,
    phase_name: &str,
) -> Result<Vec<CommitResult>> {
    let spec_path = plan_dir.join("commits.yaml");
    let default_message = format!("run-plan: {phase_name} ({plan_name})");

    // Read and delete the spec BEFORE any git operations. If we left
    // the file on disk, the catch-all pathspec `"."` in an entry would
    // sweep it into the commit; subsequently removing it would leave a
    // dirty "deleted" entry in the tree. Consuming the file up front
    // sidesteps that ordering hazard and keeps the spec behaviour
    // "one-shot" even on parse failure.
    let spec: Option<CommitsSpec> = match fs::read_to_string(&spec_path) {
        Ok(raw) => {
            fs::remove_file(&spec_path).ok();
            match serde_yaml::from_str::<CommitsSpec>(&raw) {
                Ok(parsed) if !parsed.commits.is_empty() => Some(parsed),
                _ => None,
            }
        }
        Err(_) => None,
    };

    let results = match spec {
        Some(parsed) => apply_parsed_spec(project_dir, &parsed)?,
        None => vec![commit_whole_subtree(project_dir, &default_message)?],
    };

    Ok(results)
}

/// Apply each entry in a parsed `CommitsSpec`. Spec entries that stage
/// nothing yield a `CommitResult { committed: false }` so the caller can
/// log them consistently without treating the empty case as an error.
fn apply_parsed_spec(project_dir: &Path, spec: &CommitsSpec) -> Result<Vec<CommitResult>> {
    let mut results = Vec::with_capacity(spec.commits.len());
    for entry in &spec.commits {
        unstage_subtree(project_dir)?;

        if !entry.paths.is_empty() {
            let mut add = Command::new("git");
            add.current_dir(project_dir).args(["add", "--"]);
            for p in &entry.paths {
                add.arg(p);
            }
            add.output().context("Failed to run git add for commit spec")?;
        }

        let diff = Command::new("git")
            .current_dir(project_dir)
            .args(["diff", "--cached", "--quiet"])
            .output()
            .context("Failed to run git diff --cached")?;

        if diff.status.success() {
            results.push(CommitResult {
                committed: false,
                message: entry.message.clone(),
            });
            continue;
        }

        Command::new("git")
            .current_dir(project_dir)
            .args(["commit", "-m", &entry.message])
            .output()
            .context("Failed to run git commit for commit spec")?;

        results.push(CommitResult {
            committed: true,
            message: entry.message.clone(),
        });
    }
    Ok(results)
}

/// Catch-all fallback: stage every change inside `project_dir` and
/// commit once with `message`. Used when `commits.yaml` is missing or
/// malformed. Returns `committed: false` if the subtree was already
/// clean at call time (no-op, same shape as `git_commit_plan`).
fn commit_whole_subtree(project_dir: &Path, message: &str) -> Result<CommitResult> {
    unstage_subtree(project_dir)?;

    Command::new("git")
        .current_dir(project_dir)
        .args(["add", "-A", "--", "."])
        .output()
        .context("Failed to run git add -A")?;

    let diff = Command::new("git")
        .current_dir(project_dir)
        .args(["diff", "--cached", "--quiet"])
        .output()
        .context("Failed to run git diff --cached")?;

    if diff.status.success() {
        return Ok(CommitResult {
            committed: false,
            message: message.to_string(),
        });
    }

    Command::new("git")
        .current_dir(project_dir)
        .args(["commit", "-m", message])
        .output()
        .context("Failed to run git commit (catch-all)")?;

    Ok(CommitResult {
        committed: true,
        message: message.to_string(),
    })
}

/// Unstage every tracked change inside the subtree rooted at `project_dir`
/// without touching the working tree. Safe to call when nothing is staged
/// (a no-op). The explicit `--` separator keeps a `HEAD` argument from
/// being mistaken for a pathspec.
fn unstage_subtree(project_dir: &Path) -> Result<()> {
    Command::new("git")
        .current_dir(project_dir)
        .args(["reset", "-q", "HEAD", "--", "."])
        .output()
        .context("Failed to run git reset")?;
    Ok(())
}

/// Capture the current HEAD sha into `<plan_dir>/<filename>` as a
/// phase-summary baseline. Used to write `<phase>-baseline` files
/// (work, reflect, dream, triage) that downstream LLM phases pass to
/// `state phase-summary render --baseline`.
pub fn git_save_baseline(plan_dir: &Path, filename: &str) {
    let baseline_path = plan_dir.join(filename);
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
/// `project_dir`. Runs `git diff --name-only <baseline> -- <project_dir>`;
/// the pathspec scopes the query to the subtree so a monorepo's sibling
/// subtrees are invisible. Untracked files are NOT included — they're
/// invisible to `diff` by definition and the caller handles them as an
/// always-included category.
///
/// Returned as a `HashSet` so callers can do O(1) membership tests
/// against porcelain paths when narrowing a dirty-tree warning.
pub fn paths_changed_since_baseline(
    project_dir: &Path,
    baseline_sha: &str,
) -> Result<std::collections::HashSet<String>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args(["diff", "--name-only", baseline_sha, "--"])
        .arg(project_dir)
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

/// Lines from `git status --porcelain -- <project_dir>`. Each entry is
/// the raw porcelain line including the two-character XY status prefix
/// — preserved so the caller can render them identically to what the user
/// would see if they ran `git status` themselves. The pathspec scopes
/// the query to the subtree so sibling subtrees in a monorepo don't
/// leak into the warning.
///
/// Used by the work-phase commit boundary as a postcondition: a clean
/// subtree after the work commit means the agent committed everything
/// it claimed; non-empty output means something was edited but not
/// committed (the silent-failure mode that masks lost work as
/// "backlog empty"). Returns `Ok(vec![])` on a clean subtree.
pub fn working_tree_status(project_dir: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args(["status", "--porcelain", "--"])
        .arg(project_dir)
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

/// Derive the subtree root controlled by ravel-lite from a plan
/// directory. By ravel-lite convention every plan lives at
/// `<subtree>/<state-dir>/<plan>` (typically `<project>/LLM_STATE/<plan>`),
/// so the subtree root is `<plan>/../..` — pure path math, no disk walk.
///
/// This is deliberately decoupled from `.git` location. In a single-repo
/// layout the subtree root also happens to be the git-repo root; in a
/// monorepo the git repo is somewhere further up and we don't need to
/// know where. Git queries that care about scope take this path as a
/// pathspec.
///
/// Errors if the plan dir doesn't have two parent directories (e.g.
/// `/plan` or `/a/plan`) — any well-formed plan path has at least two.
pub fn project_root_for_plan(plan_dir: &Path) -> Result<String> {
    let canon = plan_dir
        .canonicalize()
        .unwrap_or_else(|_| plan_dir.to_path_buf());
    let root: PathBuf = canon
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .with_context(|| {
            format!(
                "Cannot derive subtree root from plan dir {} — expected <subtree>/<state-dir>/<plan> layout",
                plan_dir.display()
            )
        })?;
    Ok(root.to_string_lossy().to_string())
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
        .args(["diff", "--stat", baseline_sha, "--"])
        .arg(project_dir)
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            None
        });
    let status = Command::new("git")
        .current_dir(project_dir)
        .args(["status", "--porcelain", "--"])
        .arg(project_dir)
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
    fn project_root_for_plan_derives_two_levels_up() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plan_dir = tmp.path().join("LLM_STATE").join("plan-x");
        fs::create_dir_all(&plan_dir).unwrap();
        let root = project_root_for_plan(&plan_dir).unwrap();
        // canonicalize may prepend /private on macOS; compare via canonicalize.
        let expected = tmp.path().canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(root, expected);
    }

    #[test]
    fn project_root_for_plan_errors_on_shallow_path() {
        // `/plan` has no grandparent — the function must reject it rather
        // than silently returning `/` and letting downstream git calls
        // operate on the whole filesystem.
        let result = project_root_for_plan(Path::new("/"));
        assert!(result.is_err(), "shallow path should error, got {result:?}");
    }

    #[test]
    fn project_root_for_plan_works_on_non_existent_paths() {
        // Pure path math — no disk walk, so non-existent paths are fine
        // as long as the derivation has two parent levels.
        let root = project_root_for_plan(Path::new("/a/b/c")).unwrap();
        assert_eq!(root, "/a");
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

    /// Monorepo scoping: the git queries ignore edits outside the
    /// subtree rooted at `project_dir`. Simulates:
    ///
    ///   outer-repo/.git
    ///   outer-repo/sibling/src.rs            <- edits here must be invisible
    ///   outer-repo/tools/ravel-lite/...      <- `project_dir` = subtree root
    ///   outer-repo/tools/ravel-lite/src.rs   <- edits here must be visible
    ///
    /// The three query functions (`working_tree_status`,
    /// `paths_changed_since_baseline`, `work_tree_snapshot`) all apply
    /// the same pathspec, so one synthesis covers all three.
    #[test]
    fn git_queries_are_scoped_to_subtree_in_monorepo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let outer = tmp.path().canonicalize().unwrap();
        Command::new("git").current_dir(&outer).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(&outer).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(&outer).args(["config", "user.name", "t"]).output().unwrap();

        let sibling = outer.join("sibling");
        let subtree = outer.join("tools").join("ravel-lite");
        fs::create_dir_all(&sibling).unwrap();
        fs::create_dir_all(&subtree).unwrap();
        fs::write(sibling.join("src.rs"), "fn sibling_v1() {}\n").unwrap();
        fs::write(subtree.join("src.rs"), "fn subtree_v1() {}\n").unwrap();

        Command::new("git").current_dir(&outer).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(&outer).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        let baseline = String::from_utf8(
            Command::new("git").current_dir(&outer).args(["rev-parse", "HEAD"]).output().unwrap().stdout
        ).unwrap().trim().to_string();

        // Simulate a work phase: modify a file in each subtree, add an
        // untracked file in each subtree.
        fs::write(sibling.join("src.rs"), "fn sibling_v2() {}\n").unwrap();
        fs::write(sibling.join("new.rs"), "fn sibling_new() {}\n").unwrap();
        fs::write(subtree.join("src.rs"), "fn subtree_v2() {}\n").unwrap();
        fs::write(subtree.join("new.rs"), "fn subtree_new() {}\n").unwrap();

        let status = working_tree_status(&subtree).unwrap();
        let status_blob = status.join("\n");
        assert!(
            status_blob.contains("tools/ravel-lite/src.rs")
                && status_blob.contains("tools/ravel-lite/new.rs"),
            "subtree edits must appear in status, got: {status_blob}"
        );
        assert!(
            !status_blob.contains("sibling/src.rs") && !status_blob.contains("sibling/new.rs"),
            "sibling edits must NOT appear in status, got: {status_blob}"
        );

        let changed = paths_changed_since_baseline(&subtree, &baseline).unwrap();
        assert!(
            changed.contains("tools/ravel-lite/src.rs"),
            "subtree diff must see modified subtree file, got {changed:?}"
        );
        assert!(
            !changed.contains("sibling/src.rs"),
            "subtree diff must NOT see modified sibling file, got {changed:?}"
        );

        let snapshot = work_tree_snapshot(&subtree, &baseline);
        assert!(
            snapshot.contains("tools/ravel-lite/src.rs")
                && snapshot.contains("tools/ravel-lite/new.rs"),
            "snapshot must include subtree changes, got:\n{snapshot}"
        );
        assert!(
            !snapshot.contains("sibling/src.rs") && !snapshot.contains("sibling/new.rs"),
            "snapshot must NOT include sibling changes, got:\n{snapshot}"
        );
    }

    // ------------------------------------------------------------------
    // apply_commits_spec tests
    // ------------------------------------------------------------------

    /// Initialise a git repo with minimal identity so commits succeed.
    /// Shared by the apply_commits_spec tests below.
    fn init_repo(path: &Path) {
        Command::new("git").current_dir(path).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(path).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(path).args(["config", "user.name", "t"]).output().unwrap();
    }

    /// Return the most recent N commit subject lines, oldest first.
    fn recent_subjects(repo: &Path, n: usize) -> Vec<String> {
        let out = Command::new("git")
            .current_dir(repo)
            .args(["log", "--reverse", &format!("-{n}"), "--pretty=%s"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Make a baseline commit containing `files` (path, contents), and
    /// return the repo for further work. Centralising the baseline setup
    /// keeps the spec tests focused on their own assertions.
    fn seed_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        for (path, content) in files {
            let full = repo.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, content).unwrap();
        }
        Command::new("git").current_dir(repo).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        tmp
    }

    #[test]
    fn apply_commits_spec_applies_single_commit_covering_all_paths() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        // Simulate a work cycle: one source edit plus one plan-state write.
        fs::write(repo.join("tracked.rs"), "v2\n").unwrap();
        fs::write(plan_dir.join(BACKLOG_FILENAME), "entries: []\n").unwrap();

        fs::write(
            plan_dir.join("commits.yaml"),
            "commits:\n  - paths: [\".\"]\n    message: Update tracked and seed backlog\n",
        ).unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].committed, "single entry should commit");
        assert_eq!(results[0].message, "Update tracked and seed backlog");
        assert!(
            working_tree_status(repo).unwrap().is_empty(),
            "tree should be clean after applying single catch-all commit"
        );
        assert_eq!(
            recent_subjects(repo, 1),
            vec!["Update tracked and seed backlog".to_string()]
        );
    }

    #[test]
    fn apply_commits_spec_applies_multiple_commits_in_spec_order() {
        let tmp = seed_repo(&[("src/main.rs", "fn main() {}\n"), ("LLM_STATE/core/backlog.yaml", "entries: []\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");

        fs::write(repo.join("src/main.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
        fs::write(plan_dir.join(BACKLOG_FILENAME), "entries:\n  - id: x\n").unwrap();

        // Spec order: source first, plan-state second. Verifies that the
        // applier commits in the order declared (the LLM's intent), not
        // alphabetical or any other order.
        let spec_yaml = r#"commits:
  - paths: ["src/**"]
    message: Wire a greeting into main
  - paths: ["LLM_STATE/**"]
    message: Seed backlog entry
"#;
        fs::write(plan_dir.join("commits.yaml"), spec_yaml).unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.committed));
        assert_eq!(
            recent_subjects(repo, 2),
            vec![
                "Wire a greeting into main".to_string(),
                "Seed backlog entry".to_string(),
            ]
        );
    }

    #[test]
    fn apply_commits_spec_falls_back_when_yaml_missing() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("tracked.rs"), "v2\n").unwrap();
        fs::write(repo.join("untracked.rs"), "new\n").unwrap();

        // No commits.yaml — expect a single catch-all commit with the
        // default `run-plan: <phase> (<plan>)` message.
        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].committed);
        assert_eq!(results[0].message, "run-plan: work (core)");
        assert!(working_tree_status(repo).unwrap().is_empty());
    }

    #[test]
    fn apply_commits_spec_falls_back_on_malformed_yaml() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("tracked.rs"), "v2\n").unwrap();
        fs::write(plan_dir.join("commits.yaml"), "this: is not: a valid: commits spec\n").unwrap();

        // Malformed YAML must not abort the phase — that would wedge the
        // loop on every bad spec. Fall back to the catch-all commit.
        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].committed);
        assert_eq!(results[0].message, "run-plan: work (core)");
    }

    #[test]
    fn apply_commits_spec_falls_back_on_empty_commits_list() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("tracked.rs"), "v2\n").unwrap();
        // An empty list is syntactically valid but semantically a fallback
        // trigger: no declared commits means no partition preference, so
        // the catch-all commits everything under one default message.
        fs::write(plan_dir.join("commits.yaml"), "commits: []\n").unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "run-plan: work (core)");
    }

    #[test]
    fn apply_commits_spec_supports_glob_pathspecs_that_scope_each_commit() {
        let tmp = seed_repo(&[
            ("src/a.rs", "a v1\n"),
            ("src/b.rs", "b v1\n"),
            ("docs/readme.md", "v1\n"),
        ]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("src/a.rs"), "a v2\n").unwrap();
        fs::write(repo.join("src/b.rs"), "b v2\n").unwrap();
        fs::write(repo.join("docs/readme.md"), "v2\n").unwrap();

        let spec_yaml = r#"commits:
  - paths: ["src/**"]
    message: Bump src
  - paths: ["docs/**"]
    message: Update docs
"#;
        fs::write(plan_dir.join("commits.yaml"), spec_yaml).unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 2);

        // Each commit's --name-only output must be scoped to its pathspec,
        // proving the glob matched and reset unstaged the previous entry's
        // additions.
        let bump_src = Command::new("git")
            .current_dir(repo)
            .args(["log", "-1", "--name-only", "--pretty=", "HEAD^"])
            .output()
            .unwrap();
        let update_docs = Command::new("git")
            .current_dir(repo)
            .args(["log", "-1", "--name-only", "--pretty=", "HEAD"])
            .output()
            .unwrap();
        let bump_src_paths = String::from_utf8(bump_src.stdout).unwrap();
        let update_docs_paths = String::from_utf8(update_docs.stdout).unwrap();
        assert!(bump_src_paths.contains("src/a.rs") && bump_src_paths.contains("src/b.rs"));
        assert!(!bump_src_paths.contains("docs/"));
        assert!(update_docs_paths.contains("docs/readme.md"));
        assert!(!update_docs_paths.contains("src/"));
    }

    #[test]
    fn apply_commits_spec_commits_untracked_files_covered_by_paths() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        // Only an untracked new file — no modifications.
        fs::write(repo.join("new.rs"), "fn new() {}\n").unwrap();

        let spec_yaml = r#"commits:
  - paths: ["."]
    message: Add new module
"#;
        fs::write(plan_dir.join("commits.yaml"), spec_yaml).unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].committed, "untracked files must be staged by `git add`");
        assert!(working_tree_status(repo).unwrap().is_empty());
    }

    #[test]
    fn apply_commits_spec_records_noncommitted_entry_when_paths_match_nothing() {
        let tmp = seed_repo(&[("src/a.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("src/a.rs"), "v2\n").unwrap();

        let spec_yaml = r#"commits:
  - paths: ["docs/**"]
    message: Update docs (will match nothing)
  - paths: ["src/**"]
    message: Bump src
"#;
        fs::write(plan_dir.join("commits.yaml"), spec_yaml).unwrap();

        let results = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 2);
        // Empty-match entry is preserved in the result list so the phase
        // loop can log it consistently — silently dropping it would hide
        // mismatches between spec and actual diff.
        assert!(!results[0].committed);
        assert_eq!(results[0].message, "Update docs (will match nothing)");
        assert!(results[1].committed);
        assert_eq!(results[1].message, "Bump src");
    }

    #[test]
    fn apply_commits_spec_removes_spec_file_after_apply() {
        let tmp = seed_repo(&[("tracked.rs", "v1\n")]);
        let repo = tmp.path();
        let plan_dir = repo.join("LLM_STATE").join("core");
        fs::create_dir_all(&plan_dir).unwrap();

        fs::write(repo.join("tracked.rs"), "v2\n").unwrap();
        fs::write(
            plan_dir.join("commits.yaml"),
            "commits:\n  - paths: [\".\"]\n    message: Bump\n",
        ).unwrap();

        let _ = apply_commits_spec(repo, &plan_dir, "core", "work").unwrap();
        assert!(!plan_dir.join("commits.yaml").exists(), "spec file must be removed after apply");
    }

    #[test]
    fn apply_commits_spec_is_scoped_to_subtree_in_monorepo() {
        // Outer repo with two subtrees: the project under test lives in
        // `tools/ravel-lite/`; `sibling/` is a separate subtree that must
        // remain dirty after the apply (its edits belong to another plan).
        let tmp = tempfile::TempDir::new().unwrap();
        let outer = tmp.path().canonicalize().unwrap();
        init_repo(&outer);

        let sibling = outer.join("sibling");
        let subtree = outer.join("tools").join("ravel-lite");
        let plan_dir = subtree.join("LLM_STATE").join("core");
        fs::create_dir_all(&sibling).unwrap();
        fs::create_dir_all(&plan_dir).unwrap();
        fs::write(sibling.join("src.rs"), "sib v1\n").unwrap();
        fs::write(subtree.join("src.rs"), "sub v1\n").unwrap();

        Command::new("git").current_dir(&outer).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(&outer).args(["commit", "-q", "-m", "baseline"]).output().unwrap();

        // Simulate concurrent edits in both subtrees.
        fs::write(sibling.join("src.rs"), "sib v2\n").unwrap();
        fs::write(subtree.join("src.rs"), "sub v2\n").unwrap();

        fs::write(
            plan_dir.join("commits.yaml"),
            "commits:\n  - paths: [\".\"]\n    message: Subtree-only bump\n",
        ).unwrap();

        let results = apply_commits_spec(&subtree, &plan_dir, "core", "work").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].committed);

        // The committed diff must contain only the subtree's file; the
        // sibling's uncommitted edit is the canary — if subtree scoping
        // leaked, it would have been swept into the commit too.
        let committed_paths = Command::new("git")
            .current_dir(&outer)
            .args(["log", "-1", "--name-only", "--pretty="])
            .output()
            .unwrap();
        let blob = String::from_utf8(committed_paths.stdout).unwrap();
        assert!(blob.contains("tools/ravel-lite/src.rs"));
        assert!(!blob.contains("sibling/src.rs"));

        let sibling_status = working_tree_status(&sibling).unwrap();
        assert!(
            sibling_status.iter().any(|l| l.contains("sibling/src.rs")),
            "sibling edit must still be dirty, got {sibling_status:?}"
        );
    }
}
