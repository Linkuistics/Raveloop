//! Subtree-scoped project state for a project path.
//!
//! Works for both top-level repos and monorepo subtrees by computing
//! `rel = <project_path> relative to repo toplevel`, then running
//! `git rev-parse HEAD:<rel>` for the committed tree SHA and producing
//! an auxiliary `dirty_hash` over any uncommitted state (modifications,
//! staged changes, untracked files). The two together form the cache
//! key: unchanged committed + unchanged dirty state = cache hit.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Combined state of a project subtree: committed content plus any
/// uncommitted diff/untracked content. `dirty_hash` is the empty
/// string when the subtree is clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectState {
    pub tree_sha: String,
    /// Empty string when the subtree was clean. Serde-default preserves
    /// read-compat with files emitted before this field was added.
    #[serde(default)]
    pub dirty_hash: String,
}

/// Computes the full `ProjectState` for `project_path`.
///
/// Bails only if `project_path` is not inside a git repository.
/// Uncommitted changes are no longer a hard error — they contribute to
/// `dirty_hash` so the cache invalidates when they change but discovery
/// can still run against the current working tree.
pub fn compute_project_state(project_path: &Path) -> Result<ProjectState> {
    let toplevel = repo_toplevel(project_path)?;
    // Canonicalise so macOS symlinks like /var -> /private/var match git's
    // `--show-toplevel` output exactly.
    let canon_project = std::fs::canonicalize(project_path).with_context(|| {
        format!("failed to canonicalise project path {}", project_path.display())
    })?;
    let rel = canon_project
        .strip_prefix(&toplevel)
        .with_context(|| {
            format!(
                "project path {} is not a subpath of its git toplevel {}",
                canon_project.display(),
                toplevel.display()
            )
        })?;

    let tree_sha = compute_subtree_tree_sha(&toplevel, rel)?;
    let dirty_hash = compute_subtree_dirty_hash(&toplevel, rel)?;
    Ok(ProjectState { tree_sha, dirty_hash })
}

fn compute_subtree_tree_sha(toplevel: &Path, rel: &Path) -> Result<String> {
    let spec = if rel.as_os_str().is_empty() {
        "HEAD^{tree}".to_string()
    } else {
        format!("HEAD:{}", rel.to_string_lossy())
    };

    let output = Command::new("git")
        .arg("-C")
        .arg(toplevel)
        .arg("rev-parse")
        .arg(&spec)
        .output()
        .context("failed to spawn `git rev-parse`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git rev-parse {} failed in {}: {}",
            spec,
            toplevel.display(),
            stderr.trim()
        );
    }
    let sha = String::from_utf8(output.stdout)
        .context("git rev-parse output was not valid UTF-8")?
        .trim()
        .to_string();
    if sha.is_empty() {
        bail!("git rev-parse {} returned empty output", spec);
    }
    Ok(sha)
}

/// Compute a deterministic hash over the subtree's uncommitted state
/// (staged + unstaged diffs + untracked file contents). Returns the
/// empty string when the subtree is clean. The hash is produced via
/// `git hash-object --stdin`, so it's SHA-1-sized and requires no new
/// hashing dependency.
fn compute_subtree_dirty_hash(toplevel: &Path, rel: &Path) -> Result<String> {
    // Capture staged + unstaged modifications vs HEAD.
    let diff = run_git_bytes(
        toplevel,
        &dirty_args(&["diff", "HEAD"], rel),
        "git diff HEAD",
    )?;

    // Capture untracked (non-ignored) paths; NUL-terminated for safe parsing.
    let untracked_list = run_git_bytes(
        toplevel,
        &dirty_args(&["ls-files", "--others", "--exclude-standard", "-z"], rel),
        "git ls-files --others",
    )?;

    if diff.is_empty() && untracked_list.is_empty() {
        return Ok(String::new());
    }

    // Deterministic payload: diff first, then untracked paths + content.
    let mut payload: Vec<u8> = Vec::with_capacity(diff.len() + untracked_list.len() + 1024);
    payload.extend_from_slice(&diff);
    payload.extend_from_slice(b"\n---UNTRACKED-LIST---\n");
    payload.extend_from_slice(&untracked_list);

    for path_bytes in untracked_list.split(|b| *b == 0u8).filter(|s| !s.is_empty()) {
        let path_str = std::str::from_utf8(path_bytes)
            .context("untracked path is not valid UTF-8")?;
        let abs = toplevel.join(path_str);
        // Skip directories and unreadable entries — porcelain listing
        // already fingerprints their presence, so silent skip here just
        // means the directory itself has no content to fold in.
        if let Ok(bytes) = std::fs::read(&abs) {
            payload.extend_from_slice(b"\n---CONTENT---\n");
            payload.extend_from_slice(path_bytes);
            payload.extend_from_slice(b"\n");
            payload.extend_from_slice(&bytes);
        }
    }

    hash_payload_via_git(&payload, toplevel)
}

fn dirty_args(base: &[&str], rel: &Path) -> Vec<String> {
    let mut v: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    if !rel.as_os_str().is_empty() {
        v.push("--".to_string());
        v.push(rel.to_string_lossy().into_owned());
    }
    v
}

fn run_git_bytes(toplevel: &Path, args: &[String], description: &str) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(toplevel)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `{description}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{} failed in {}: {}",
            description,
            toplevel.display(),
            stderr.trim()
        );
    }
    Ok(output.stdout)
}

fn hash_payload_via_git(payload: &[u8], toplevel: &Path) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(toplevel)
        .arg("hash-object")
        .arg("-t")
        .arg("blob")
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn `git hash-object --stdin`")?;
    child
        .stdin
        .as_mut()
        .context("git hash-object stdin unavailable")?
        .write_all(payload)
        .context("failed to write payload to git hash-object")?;
    let out = child
        .wait_with_output()
        .context("waiting on git hash-object")?;
    if !out.status.success() {
        bail!(
            "git hash-object --stdin failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let sha = String::from_utf8(out.stdout)
        .context("git hash-object output was not valid UTF-8")?
        .trim()
        .to_string();
    if sha.is_empty() {
        bail!("git hash-object --stdin returned empty output");
    }
    Ok(sha)
}

fn repo_toplevel(project_path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .context("failed to spawn `git rev-parse --show-toplevel`")?;
    if !output.status.success() {
        bail!(
            "project at {} is not inside a git repository — initialise with \
             `git init` or remove from the catalog",
            project_path.display()
        );
    }
    let s = String::from_utf8(output.stdout)
        .context("git --show-toplevel output was not valid UTF-8")?
        .trim()
        .to_string();
    Ok(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_repo_with_readme(path: &Path) {
        run(path, &["init", "-q", "-b", "main"]);
        run(path, &["config", "user.email", "test@example.com"]);
        run(path, &["config", "user.name", "test"]);
        std::fs::write(path.join("README.md"), "hello\n").unwrap();
        run(path, &["add", "README.md"]);
        run(path, &["commit", "-q", "-m", "init"]);
    }

    fn run(cwd: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(cwd).args(args).status().unwrap();
        assert!(status.success(), "git {:?} in {} failed", args, cwd.display());
    }

    #[test]
    fn top_level_repo_yields_non_empty_sha() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sha = compute_project_state(tmp.path()).unwrap().tree_sha;
        assert_eq!(sha.len(), 40, "expected 40-hex SHA, got {:?}", sha);
    }

    #[test]
    fn monorepo_subtrees_have_independent_shas() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let sha_a = compute_project_state(&sub_a).unwrap().tree_sha;
        let sha_b = compute_project_state(&sub_b).unwrap().tree_sha;
        assert_ne!(sha_a, sha_b, "subtrees with different content must have different SHAs");
    }

    #[test]
    fn sibling_subtree_change_does_not_invalidate_other_subtree() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B1\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let state_b_before = compute_project_state(&sub_b).unwrap();

        std::fs::write(sub_a.join("a.txt"), "A-edited\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "edit sub-a"]);

        let state_b_after = compute_project_state(&sub_b).unwrap();
        assert_eq!(state_b_before, state_b_after, "sub-b's state must be stable across a commit that only touches sub-a");
    }

    #[test]
    fn non_git_project_bails_with_actionable_message() {
        let tmp = TempDir::new().unwrap();
        let err = compute_project_state(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not inside a git repository"), "got: {msg}");
        assert!(msg.contains("git init"), "got: {msg}");
    }

    #[test]
    fn clean_subtree_has_empty_dirty_hash() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let state = compute_project_state(tmp.path()).unwrap();
        assert_eq!(state.dirty_hash, "", "clean tree must have empty dirty_hash");
    }

    #[test]
    fn dirty_subtree_produces_non_empty_dirty_hash() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        std::fs::write(tmp.path().join("README.md"), "edited\n").unwrap();

        let state = compute_project_state(tmp.path()).unwrap();
        assert_eq!(state.dirty_hash.len(), 40, "expected SHA-1, got {:?}", state.dirty_hash);
    }

    #[test]
    fn dirty_hash_changes_when_tracked_file_changes() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        std::fs::write(tmp.path().join("README.md"), "first-edit\n").unwrap();
        let state_a = compute_project_state(tmp.path()).unwrap();

        std::fs::write(tmp.path().join("README.md"), "second-edit\n").unwrap();
        let state_b = compute_project_state(tmp.path()).unwrap();

        assert_eq!(state_a.tree_sha, state_b.tree_sha, "tree_sha must not change without a commit");
        assert_ne!(state_a.dirty_hash, state_b.dirty_hash, "dirty_hash must change when tracked file content changes");
    }

    #[test]
    fn dirty_hash_changes_when_untracked_file_added_or_modified() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let clean_state = compute_project_state(tmp.path()).unwrap();
        assert_eq!(clean_state.dirty_hash, "");

        std::fs::write(tmp.path().join("new.txt"), "hi\n").unwrap();
        let with_untracked = compute_project_state(tmp.path()).unwrap();
        assert_ne!(with_untracked.dirty_hash, "", "adding untracked file must populate dirty_hash");

        std::fs::write(tmp.path().join("new.txt"), "different content\n").unwrap();
        let with_modified_untracked = compute_project_state(tmp.path()).unwrap();
        assert_ne!(
            with_untracked.dirty_hash, with_modified_untracked.dirty_hash,
            "editing an untracked file's contents must change dirty_hash"
        );
    }

    #[test]
    fn dirty_hash_is_subtree_scoped_in_monorepo() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let state_b_before = compute_project_state(&sub_b).unwrap();

        // Dirty change in sub-a only. sub-b's state must stay unchanged.
        std::fs::write(sub_a.join("a.txt"), "A-edited-but-uncommitted\n").unwrap();

        let state_b_after = compute_project_state(&sub_b).unwrap();
        assert_eq!(state_b_before, state_b_after, "dirty edits in sibling subtree must not affect this subtree's state");
    }
}
