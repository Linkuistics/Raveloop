#![cfg(unix)]

//! End-to-end integration test for `state related-projects discover`
//! and `discover-apply`. Uses a fake `claude` shell script on PATH that
//! reads the prompt, extracts the output-path tokens, and writes canned
//! YAML there.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Scaffold a monorepo with two subtree projects, both committed to
/// a single git repo, plus a catalogued config dir.
fn scaffold(tmp: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let repo = tmp.join("mono");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "test"]);

    let alpha = repo.join("alpha");
    let beta = repo.join("beta");
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    std::fs::write(alpha.join("README.md"), "alpha consumes /data/*.yaml\n").unwrap();
    std::fs::write(beta.join("README.md"), "beta produces /data/*.yaml\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-q", "-m", "init"]);

    let cfg = tmp.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    (repo, alpha, beta, cfg)
}

fn run_git(cwd: &Path, args: &[&str]) {
    let s = Command::new("git").arg("-C").arg(cwd).args(args).status().unwrap();
    assert!(s.success(), "git {args:?} in {} failed", cwd.display());
}

/// Writes a bash shim to `shim_dir/claude` that, for each invocation,
/// grabs the `-p <prompt>` argument, detects Stage 1 vs Stage 2 by the
/// prompt's header, extracts the first `.yaml` path from the prompt
/// (assumed to be the output-path placeholder), and writes the
/// appropriate canned YAML to it.
///
/// Brittleness: the `grep | head -n1` extractor assumes the output-path
/// placeholder appears in the prompt *before* any other yaml paths that
/// might be inlined via `{{SURFACE_RECORDS_YAML}}`. This holds today
/// because the Output-format section precedes the Input section in both
/// discover-stage{1,2}.md. If that ordering inverts, the shim will
/// point cat at the wrong file.
fn write_fake_claude(shim_dir: &Path, stage1_yaml: &str, stage2_yaml: &str) -> PathBuf {
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

prompt_arg=""
for ((i=1; i<=$#; i++)); do
  if [[ "${{!i}}" == "-p" ]]; then
    ((j=i+1))
    prompt_arg="${{!j}}"
    break
  fi
done

if grep -q 'Extract Interaction Surface' <<<"$prompt_arg"; then
  out=$(grep -oE '/[^[:space:]]+\.yaml' <<<"$prompt_arg" | head -n1)
  cat >"$out" <<'YAML'
{stage1}
YAML
else
  out=$(grep -oE '/[^[:space:]]+\.yaml' <<<"$prompt_arg" | head -n1)
  cat >"$out" <<'YAML'
{stage2}
YAML
fi
"#,
        stage1 = stage1_yaml,
        stage2 = stage2_yaml,
    );
    let path = shim_dir.join("claude");
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
fn discover_writes_proposals_and_apply_merges_them() {
    let tmp = TempDir::new().unwrap();
    let (_repo, alpha, _beta, cfg) = scaffold(tmp.path());

    // Catalogue both projects.
    let status = Command::new(bin_path())
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "Alpha", "--path"])
        .arg(&alpha)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new(bin_path())
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "Beta", "--path"])
        .arg(tmp.path().join("mono").join("beta"))
        .status()
        .unwrap();
    assert!(status.success());

    // Copy prompt templates into the config root (discover reads them
    // from <config-root>/discover-stage{1,2}.md).
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(
        repo_root.join("defaults/discover-stage1.md"),
        cfg.join("discover-stage1.md"),
    )
    .unwrap();
    std::fs::copy(
        repo_root.join("defaults/discover-stage2.md"),
        cfg.join("discover-stage2.md"),
    )
    .unwrap();
    // Minimal config.yaml (both `agent` and `headroom` required by SharedConfig)
    // + agent config so load_shared_config / load_agent_config succeed.
    std::fs::write(
        cfg.join("config.yaml"),
        "agent: claude-code\nheadroom: 0\n",
    )
    .unwrap();
    std::fs::create_dir_all(cfg.join("agents/claude-code")).unwrap();
    std::fs::write(
        cfg.join("agents/claude-code/config.yaml"),
        "models:\n  discover: fake-model\n",
    )
    .unwrap();

    // Install fake claude on PATH.
    let shim_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&shim_dir).unwrap();
    write_fake_claude(
        &shim_dir,
        "purpose: alpha consumes yaml\nconsumes_files: [/data/*.yaml]\n",
        "generated_at: 2026-04-22T00:00:00Z\nproposals:\n  - kind: parent-of\n    participants: [Beta, Alpha]\n    rationale: 'beta produces, alpha consumes'\n    supporting_surface_fields: []\n",
    );

    // Run discover.
    let status = Command::new(bin_path())
        .env("PATH", format!("{}:{}", shim_dir.display(), std::env::var("PATH").unwrap()))
        .args(["state", "related-projects", "discover", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());

    // Proposals file exists.
    let proposals_path = cfg.join("discover-proposals.yaml");
    assert!(proposals_path.exists());
    let content = std::fs::read_to_string(&proposals_path).unwrap();
    assert!(content.contains("parent-of"));
    assert!(content.contains("Beta"));
    assert!(content.contains("Alpha"));

    // Cache files exist.
    assert!(cfg.join("discover-cache/Alpha.yaml").exists());
    assert!(cfg.join("discover-cache/Beta.yaml").exists());

    // Apply and verify related-projects.yaml.
    let status = Command::new(bin_path())
        .args(["state", "related-projects", "discover-apply", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());
    let rp = std::fs::read_to_string(cfg.join("related-projects.yaml")).unwrap();
    assert!(rp.contains("parent-of"));
}
