//! End-to-end CLI integration tests for `ravel-lite state backlog *`
//! and `ravel-lite state migrate`. Shells out to the built binary via
//! CARGO_BIN_EXE_ravel-lite, matching the pattern in tests/integration.rs.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn seed_two_task_backlog_md(plan_dir: &std::path::Path) {
    let content = "\
### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Cargo clippy is clean today. Add a CI gate to keep it that way.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Blocked on upstream Claude Code release past 2.1.116.

**Results:** _pending_

---
";
    std::fs::write(plan_dir.join("backlog.md"), content).unwrap();
}

#[test]
fn migrate_converts_backlog_md_to_yaml_and_list_emits_ready_tasks() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    let migrate = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        migrate.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    assert!(tmp.path().join("backlog.yaml").exists());
    assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--status", "not_started", "--ready"])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        list.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(stdout.contains("add-clippy-d-warnings-ci-gate"), "output must include task id: {stdout}");
    assert!(
        stdout.contains("remove-claude-code-debug-file-workaround"),
        "output must include second task id: {stdout}"
    );
}

#[test]
fn migrate_dry_run_does_not_write_yaml() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(out.status.success(), "dry-run must exit 0");
    assert!(!tmp.path().join("backlog.yaml").exists(), "dry-run wrote yaml");
}

#[test]
fn migrate_is_idempotent_across_repeated_runs() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    for _ in 0..2 {
        let out = Command::new(bin())
            .args(["state", "migrate"])
            .arg(tmp.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "migrate failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // List must still yield two tasks (no duplication, no corruption).
    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let tasks: usize = stdout.matches("id:").count();
    assert_eq!(tasks, 2, "expected two tasks after idempotent migrate, got stdout:\n{stdout}");
}

#[test]
fn migrate_parse_failure_leaves_filesystem_untouched() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("backlog.md"), "### Bad\n\nno fields\n").unwrap();

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "malformed input must exit non-zero");
    assert!(!tmp.path().join("backlog.yaml").exists(), "partial write on parse failure");
}

fn add_seed_task(plan_dir: &std::path::Path) {
    // Start from an empty backlog.yaml and append one task via the CLI
    // so state_backlog tests share a compact, repeatable setup.
    std::fs::write(plan_dir.join("backlog.yaml"), "tasks: []\n").unwrap();
    let add = Command::new(bin())
        .args(["state", "backlog", "add"])
        .arg(plan_dir)
        .args(["--title", "Seed task", "--category", "maintenance"])
        .args(["--description", "Original body.\n"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "seed add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
}

#[test]
fn set_description_via_body_file_round_trips_through_cli() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let body_file = tmp.path().join("new-body.md");
    std::fs::write(&body_file, "Fresh brief from disk.\n").unwrap();

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body-file"])
        .arg(&body_file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "set-description failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(
        stdout.contains("Fresh brief from disk."),
        "show must reflect new body: {stdout}"
    );
    assert!(
        !stdout.contains("Original body."),
        "old body must be gone: {stdout}"
    );
}

#[test]
fn set_description_via_body_stdin_round_trips_through_cli() {
    use std::io::Write;
    use std::process::Stdio;

    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let mut child = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Piped-in body.\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "set-description failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Piped-in body."));
}

#[test]
fn set_description_errors_on_unknown_task() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["nonexistent", "--body", "anything\n"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "unknown id must exit non-zero");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("nonexistent"), "stderr must cite the id: {stderr}");
}

#[test]
fn set_description_rejects_empty_body() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body", ""])
        .output()
        .unwrap();
    assert!(!out.status.success(), "empty body must exit non-zero");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("empty"), "stderr must mention empty: {stderr}");

    // Original body preserved.
    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Original body."));
}

#[test]
fn set_description_preserves_sibling_fields() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    // Pre-load sibling fields; note the plan_dir goes immediately after
    // the subcommand, then positional task id + verb-specific args.
    let run = |args: &[&str]| {
        let out = Command::new(bin())
            .args(["state", "backlog"])
            .arg(args[0])
            .arg(tmp.path())
            .args(&args[1..])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["set-status", "seed-task", "in_progress"]);
    run(&["set-title", "seed-task", "Renamed Seed"]);
    run(&["set-results", "seed-task", "--body", "Results body.\n"]);
    run(&["set-handoff", "seed-task", "--body", "Handoff body.\n"]);

    run(&["set-description", "seed-task", "--body", "Rewritten brief.\n"]);

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Rewritten brief."), "desc updated: {stdout}");
    assert!(stdout.contains("in_progress"), "status preserved: {stdout}");
    assert!(stdout.contains("Renamed Seed"), "title preserved: {stdout}");
    assert!(stdout.contains("Results body."), "results preserved: {stdout}");
    assert!(stdout.contains("Handoff body."), "handoff preserved: {stdout}");
    // The task id must remain stable across the rename + description rewrite.
    assert!(stdout.contains("id: seed-task"), "id stable: {stdout}");
}

#[test]
fn add_set_status_set_results_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty backlog.yaml so add has nothing to collide with.
    std::fs::write(
        tmp.path().join("backlog.yaml"),
        "tasks: []\n",
    )
    .unwrap();

    let add = Command::new(bin())
        .args(["state", "backlog", "add"])
        .arg(tmp.path())
        .args(["--title", "New task", "--category", "maintenance"])
        .args(["--description", "Task body.\n"])
        .output()
        .unwrap();
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let set_status = Command::new(bin())
        .args(["state", "backlog", "set-status"])
        .arg(tmp.path())
        .args(["new-task", "in_progress"])
        .output()
        .unwrap();
    assert!(set_status.status.success());

    let set_results = Command::new(bin())
        .args(["state", "backlog", "set-results"])
        .arg(tmp.path())
        .args(["new-task", "--body", "Finished.\n"])
        .output()
        .unwrap();
    assert!(set_results.status.success());

    // set-results is only meaningful on `done` tasks conceptually, but
    // the verb itself accepts any status; the conceptual invariant
    // (flip status to done first) is a prompt-side concern.
    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("new-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("in_progress"));
    assert!(stdout.contains("Finished."));
}
