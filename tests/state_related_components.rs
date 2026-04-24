//! End-to-end CLI integration tests for `ravel-lite state related-components *`.
//! Shells out to the built binary via CARGO_BIN_EXE_ravel-lite, matching the
//! pattern in tests/integration.rs.

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

/// Minimal strong-evidence add-edge invocation builder. Keeps tests
/// compact and consistent with the exit-criteria smoke-test shape:
/// `add-edge <kind> <lifecycle> <a> <b> --evidence-grade strong
/// --evidence-field <a>.produces_files --evidence-field <b>.consumes_files
/// --rationale "<a> emits schemas <b> consumes"`.
fn strong_add_edge(
    cfg: &Path,
    kind: &str,
    lifecycle: &str,
    a: &str,
    b: &str,
) -> std::process::Output {
    Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args([kind, lifecycle, a, b])
        .args(["--evidence-grade", "strong"])
        .args(["--evidence-field", &format!("{a}.produces_files")])
        .args(["--evidence-field", &format!("{b}.consumes_files")])
        .args(["--rationale", &format!("{a} emits schemas {b} consumes")])
        .output()
        .unwrap()
}

#[test]
fn add_list_remove_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = strong_add_edge(&cfg, "generates", "codegen", "Me", "Peer");
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
    assert!(yaml.contains("evidence_grade: strong"), "list output: {yaml}");
    assert!(yaml.contains("Me.produces_files"), "list output: {yaml}");
    assert!(yaml.contains("Peer.consumes_files"), "list output: {yaml}");
    assert!(
        yaml.contains("emits schemas"),
        "list output must preserve rationale: {yaml}"
    );
    assert!(yaml.contains("Me"), "list output: {yaml}");
    assert!(yaml.contains("Peer"), "list output: {yaml}");

    let remove = Command::new(bin())
        .args(["state", "related-components", "remove-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "codegen", "Me", "Peer"])
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
fn add_edge_accepts_weak_grade_without_evidence_fields() {
    // `weak` is the only grade permitted to omit `--evidence-field`.
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "codegen", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "weakly suggested by prose overlap"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "weak add-edge should succeed without --evidence-field: stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );
}

#[test]
fn add_edge_rejects_strong_grade_without_evidence_fields() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "codegen", "Me", "Peer"])
        .args(["--evidence-grade", "strong"])
        .args(["--rationale", "strong but unsourced"])
        .output()
        .unwrap();
    assert!(
        !add.status.success(),
        "strong without --evidence-field must be rejected"
    );
    let stderr = String::from_utf8_lossy(&add.stderr);
    assert!(
        stderr.contains("evidence_field"),
        "stderr must point at the evidence-field invariant: {stderr}"
    );
}

#[test]
fn add_edge_rejects_missing_lifecycle_positional() {
    // v1-style `add-edge <kind> <a> <b>` (omitting lifecycle) must
    // fail with clap's required-arg error — no silent synthesis.
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "missing lifecycle"])
        .output()
        .unwrap();
    assert!(
        !add.status.success(),
        "v1-style add-edge without lifecycle must be rejected"
    );
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
        .args(["co-implements", "design", "Beta", "Alpha"])
        .args(["--evidence-grade", "medium"])
        .args(["--evidence-field", "Alpha.purpose"])
        .args(["--rationale", "shared RFC"])
        .status()
        .unwrap();
    assert!(first.success());

    let second = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["co-implements", "design", "Alpha", "Beta"])
        .args(["--evidence-grade", "medium"])
        .args(["--evidence-field", "Alpha.purpose"])
        .args(["--rationale", "shared RFC"])
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
        .args(["sibling", "codegen", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "test"])
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
fn add_edge_rejects_unknown_lifecycle_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["generates", "breakfast", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "test"])
        .output()
        .unwrap();
    assert!(!add.status.success(), "bogus lifecycle must be rejected");
    let stderr = String::from_utf8(add.stderr).unwrap();
    assert!(
        stderr.contains("invalid lifecycle"),
        "stderr must explain the rejection: {stderr}"
    );
}

#[test]
fn add_edge_rejects_unknown_component_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &[]);

    let add = strong_add_edge(&cfg, "generates", "codegen", "Me", "Ghost");
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
        let out = strong_add_edge(&cfg, "generates", "codegen", a, b);
        assert!(
            out.status.success(),
            "seed add-edge failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
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
fn list_with_kind_filter_restricts_to_matching_kind() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let g = strong_add_edge(&cfg, "generates", "codegen", "Me", "Peer");
    assert!(g.status.success(), "generates add failed");
    let o = strong_add_edge(&cfg, "orchestrates", "dev-workflow", "Me", "Peer");
    assert!(o.status.success(), "orchestrates add failed");

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--kind", "generates"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("kind: generates"), "filtered output: {yaml}");
    assert!(
        !yaml.contains("kind: orchestrates"),
        "filter must exclude other kinds: {yaml}"
    );
}

#[test]
fn list_with_lifecycle_filter_restricts_to_matching_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    // Same pair + same kind at two lifecycles.
    let a = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["depends-on", "build", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "manifest"])
        .status()
        .unwrap();
    assert!(a.success());
    let b = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["depends-on", "runtime", "Me", "Peer"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "dynamic load"])
        .status()
        .unwrap();
    assert!(b.success());

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--lifecycle", "runtime"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("lifecycle: runtime"), "filtered output: {yaml}");
    assert!(
        !yaml.contains("lifecycle: build"),
        "filter must exclude other lifecycles: {yaml}"
    );
}

#[test]
fn list_kind_and_lifecycle_filters_compose() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let g = strong_add_edge(&cfg, "generates", "codegen", "Me", "Peer");
    assert!(g.status.success());
    let o = strong_add_edge(&cfg, "orchestrates", "dev-workflow", "Me", "Peer");
    assert!(o.status.success());

    // Intersection of (kind=generates, lifecycle=dev-workflow) matches nothing.
    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--kind", "generates"])
        .args(["--lifecycle", "dev-workflow"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(!yaml.contains("kind:"), "empty intersection must emit no edges: {yaml}");
}

#[test]
fn remove_edge_requires_matching_lifecycle() {
    // Add the same (kind, pair) at two lifecycles, remove only one,
    // verify the other survives.
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    for lifecycle in ["build", "runtime"] {
        let out = Command::new(bin())
            .args(["state", "related-components", "add-edge"])
            .args(["--config", cfg.to_str().unwrap()])
            .args(["depends-on", lifecycle, "Me", "Peer"])
            .args(["--evidence-grade", "weak"])
            .args(["--rationale", "x"])
            .status()
            .unwrap();
        assert!(out.success(), "seed add failed for lifecycle={lifecycle}");
    }

    let rm = Command::new(bin())
        .args(["state", "related-components", "remove-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["depends-on", "build", "Me", "Peer"])
        .status()
        .unwrap();
    assert!(rm.success());

    let list = Command::new(bin())
        .args(["state", "related-components", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("lifecycle: runtime"), "runtime must survive: {yaml}");
    assert!(!yaml.contains("lifecycle: build"), "build must be gone: {yaml}");
}
