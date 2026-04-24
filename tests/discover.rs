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

/// Canned Stage 2 proposal, representing a single edge the fake-claude
/// shim should emit via the `add-proposal` CLI.
struct Stage2Proposal {
    kind: &'static str,
    lifecycle: &'static str,
    from: &'static str,
    to: &'static str,
    evidence_grade: &'static str,
    evidence_fields: &'static [&'static str],
    rationale: &'static str,
}

/// Writes a bash shim to `shim_dir/claude` that, for each invocation,
/// grabs the `-p <prompt>` argument and branches on header marker:
///
/// * **Stage 1** (`Extract Interaction Surface`): extract the first
///   `.yaml` path from the prompt and write `stage1_yaml` there. Matches
///   the legacy Write-a-YAML-file flow Stage 1 still uses.
/// * **Stage 2** (otherwise): issue one
///   `ravel-lite state discover-proposals add-proposal` invocation per
///   entry in `stage2_proposals`. This mirrors the production flow
///   exactly — proposals land through the CLI, not through a YAML
///   write — so the validation pipeline is exercised end-to-end.
///
/// `ravel_lite_bin` is baked into the shim literally so it works under
/// any test `PATH` rewrite.
fn write_fake_claude(
    shim_dir: &Path,
    ravel_lite_bin: &Path,
    config_root: &Path,
    stage1_yaml: &str,
    stage2_proposals: &[Stage2Proposal],
) -> PathBuf {
    let mut stage2_commands = String::new();
    for p in stage2_proposals {
        stage2_commands.push_str(&format!(
            "{bin:?} state discover-proposals add-proposal \\\n\
             \t--config {cfg:?} \\\n\
             \t--kind {kind} \\\n\
             \t--lifecycle {lifecycle} \\\n\
             \t--participant {from} \\\n\
             \t--participant {to} \\\n\
             \t--evidence-grade {grade} \\\n",
            bin = ravel_lite_bin,
            cfg = config_root,
            kind = p.kind,
            lifecycle = p.lifecycle,
            from = p.from,
            to = p.to,
            grade = p.evidence_grade,
        ));
        for f in p.evidence_fields {
            stage2_commands.push_str(&format!("\t--evidence-field {f:?} \\\n"));
        }
        stage2_commands.push_str(&format!("\t--rationale {:?}\n", p.rationale));
    }

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
{stage2}
fi
"#,
        stage1 = stage1_yaml,
        stage2 = stage2_commands,
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
        &bin_path(),
        &cfg,
        "purpose: alpha consumes yaml\nconsumes_files: [/data/*.yaml]\n",
        &[Stage2Proposal {
            kind: "generates",
            lifecycle: "codegen",
            from: "Beta",
            to: "Alpha",
            evidence_grade: "strong",
            evidence_fields: &[
                "Beta.surface.produces_files",
                "Alpha.surface.consumes_files",
            ],
            rationale: "beta produces /data/*.yaml that alpha consumes",
        }],
    );

    // Run discover.
    let status = Command::new(bin_path())
        .env("PATH", format!("{}:{}", shim_dir.display(), std::env::var("PATH").unwrap()))
        .args(["state", "related-components", "discover", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());

    // Proposals file exists.
    let proposals_path = cfg.join("discover-proposals.yaml");
    assert!(proposals_path.exists());
    let content = std::fs::read_to_string(&proposals_path).unwrap();
    assert!(content.contains("generates"));
    assert!(content.contains("Beta"));
    assert!(content.contains("Alpha"));

    // Cache files exist.
    assert!(cfg.join("discover-cache/Alpha.yaml").exists());
    assert!(cfg.join("discover-cache/Beta.yaml").exists());

    // Apply and verify related-components.yaml.
    let status = Command::new(bin_path())
        .args(["state", "related-components", "discover-apply", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());
    let rp = std::fs::read_to_string(cfg.join("related-components.yaml")).unwrap();
    assert!(rp.contains("generates"));

    // --- Second run: every Stage 1 must cache-hit and Stage 2 must be skipped ---
    // We hand-edit the proposals file to a marker string; if Stage 2 runs, the
    // fake claude shim would overwrite it, wiping our marker. Preserving the
    // marker proves the file was not rewritten.
    let marker = "# USER-EDIT-MARKER — discover must not overwrite this\n";
    let mut preserved = marker.to_string();
    preserved.push_str(&std::fs::read_to_string(&proposals_path).unwrap());
    std::fs::write(&proposals_path, &preserved).unwrap();

    let output = Command::new(bin_path())
        .env("PATH", format!("{}:{}", shim_dir.display(), std::env::var("PATH").unwrap()))
        .args(["state", "related-components", "discover", "--config"])
        .arg(&cfg)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "second discover run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Stage 2: skipped"),
        "expected Stage 2 skip message, got stderr:\n{stderr}"
    );

    let second_content = std::fs::read_to_string(&proposals_path).unwrap();
    assert!(
        second_content.starts_with(marker),
        "discover-proposals.yaml was rewritten despite all Stage 1 being cached"
    );
}
