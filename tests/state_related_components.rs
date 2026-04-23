//! End-to-end CLI integration tests for `ravel-lite state related-components *`,
//! plus the deprecated `state related-projects` alias. Shells out to the
//! built binary via CARGO_BIN_EXE_ravel-lite, matching the pattern in
//! tests/integration.rs.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Scaffold: `<tmp>/cfg/` for the config dir and `<tmp>/projects/<name>/`
/// for each project directory. Seeds the projects catalog with `me` plus
/// the listed peers.
fn scaffold(tmp: &Path, me: &str, peers: &[&str]) -> (PathBuf, PathBuf) {
    let cfg = tmp.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let projects_root = tmp.join("projects");

    for name in std::iter::once(me).chain(peers.iter().copied()) {
        let p = projects_root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        let status = Command::new(bin())
            .args(["state", "projects", "add"])
            .args(["--config", cfg.to_str().unwrap()])
            .args(["--name", name])
            .args(["--path", p.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success(), "seed projects add failed for {name}");
    }

    let plan_dir = projects_root.join(me).join("LLM_STATE").join("core");
    std::fs::create_dir_all(&plan_dir).unwrap();
    (cfg, plan_dir)
}

#[test]
fn add_list_remove_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    // `generates` is a directed v2 kind — order is semantic.
    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "Me", "Peer"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "add-edge failed: stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(cfg.join("related-components.yaml").exists());

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("kind: generates"), "list output: {yaml}");
    assert!(yaml.contains("lifecycle: codegen"), "list output: {yaml}");
    assert!(yaml.contains("evidence_grade: weak"), "list output: {yaml}");
    assert!(yaml.contains("Me"), "list output: {yaml}");
    assert!(yaml.contains("Peer"), "list output: {yaml}");

    let remove = Command::new(bin())
        .args(["state", "related-components", "remove-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "Me", "Peer"])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "remove-edge failed: stderr={}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let list2 = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    let yaml2 = String::from_utf8(list2.stdout).unwrap();
    assert!(!yaml2.contains("kind: generates"), "edge must be gone: {yaml2}");
}

#[test]
fn add_edge_canonicalises_symmetric_kind_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Alpha", &["Beta"]);

    // `co-implements` is symmetric — adding (Beta, Alpha) must canonicalise
    // to (Alpha, Beta), and a follow-up (Alpha, Beta) must dedup.
    let first = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["co-implements", "Beta", "Alpha"])
        .status()
        .unwrap();
    assert!(first.success());

    let second = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["co-implements", "Alpha", "Beta"])
        .output()
        .unwrap();
    assert!(second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("already present"),
        "second add must dedup: stderr={stderr}"
    );

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(
        yaml.contains("- Alpha\n  - Beta"),
        "symmetric participants must be sorted in storage: {yaml}"
    );
}

#[test]
fn add_edge_rejects_unknown_kind_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    // `sibling` was a v1 kind name; v2 rejects it.
    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["sibling", "Me", "Peer"])
        .output()
        .unwrap();
    assert!(!add.status.success(), "v1 kind 'sibling' must be rejected");
    let stderr = String::from_utf8(add.stderr).unwrap();
    assert!(
        stderr.contains("invalid kind"),
        "stderr must explain the rejection: {stderr}"
    );
    assert!(
        stderr.contains("ontology v2"),
        "stderr must mention ontology v2 vocabulary: {stderr}"
    );
}

#[test]
fn add_edge_rejects_unknown_component_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &[]);

    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "Me", "Ghost"])
        .output()
        .unwrap();
    assert!(!add.status.success(), "unknown component must fail");
    let stderr = String::from_utf8(add.stderr).unwrap();
    assert!(
        stderr.contains("Ghost"),
        "stderr must name the missing component: {stderr}"
    );
}

#[test]
fn list_with_plan_filter_restricts_to_plan_component_edges() {
    let tmp = TempDir::new().unwrap();
    let (cfg, plan_dir) = scaffold(tmp.path(), "Me", &["Peer", "Other", "Third"]);

    // Two edges: one involves Me, one doesn't.
    for (a, b) in [("Me", "Peer"), ("Other", "Third")] {
        let status = Command::new(bin())
            .args(["state", "related-components", "add-edge"])
            .args(["--config", cfg.to_str().unwrap()])
            .args(["generates", a, b])
            .status()
            .unwrap();
        assert!(status.success());
    }

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--plan", plan_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("Peer"), "filtered output must contain Peer: {yaml}");
    assert!(!yaml.contains("Other"), "filtered output must exclude non-Me edges: {yaml}");
}

#[test]
fn deprecated_related_projects_alias_warns_and_forwards() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    // Add via the canonical verb.
    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "Me", "Peer"])
        .status()
        .unwrap();
    assert!(add.success());

    // List via the deprecated alias must succeed AND emit a stderr warning.
    let list = Command::new(bin())
        .args(["state", "related-projects", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success(), "deprecated alias must still work");

    let stderr = String::from_utf8_lossy(&list.stderr);
    assert!(
        stderr.contains("deprecated"),
        "deprecated alias must emit a warning to stderr: {stderr}"
    );
    assert!(
        stderr.contains("related-components"),
        "deprecation warning must point at the new verb: {stderr}"
    );

    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("kind: generates"), "alias output: {yaml}");
}

#[test]
fn loading_legacy_v1_filename_hard_errors_with_actionable_message() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &[]);

    // Plant a v1 file at the legacy path.
    std::fs::write(
        cfg.join("related-projects.yaml"),
        "schema_version: 1\nedges: []\n",
    )
    .unwrap();

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!list.status.success(), "legacy v1 file must hard-error");
    let stderr = String::from_utf8(list.stderr).unwrap();
    assert!(
        stderr.contains("legacy v1 file"),
        "error must name the legacy file: {stderr}"
    );
    assert!(
        stderr.contains("discover --apply"),
        "error must point at the regenerate command: {stderr}"
    );
}
