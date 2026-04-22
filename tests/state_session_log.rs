//! End-to-end CLI integration tests for `ravel-lite state session-log *`
//! and the session-log.md / latest-session.md paths of
//! `ravel-lite state migrate`. Shells out to the built binary via
//! CARGO_BIN_EXE_ravel-lite.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn seed_two_session_log_md(plan_dir: &std::path::Path) {
    let content = "\
# Session Log

### Session 1 (2026-04-21T08:03:01Z) — First migrated

- Bullet one.
- Bullet two.

### Session 2 (2026-04-22T06:14:36Z) — Second migrated

Paragraph body.
";
    std::fs::write(plan_dir.join("session-log.md"), content).unwrap();
}

fn seed_latest_session_md(plan_dir: &std::path::Path) {
    std::fs::write(
        plan_dir.join("latest-session.md"),
        "\
### Session 11 (2026-04-22T12:00:00Z) — Fresh latest

- Body bullet.
",
    )
    .unwrap();
}

#[test]
fn migrate_converts_session_log_md_to_yaml_and_list_emits_sessions() {
    let tmp = TempDir::new().unwrap();
    seed_two_session_log_md(tmp.path());

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
    assert!(tmp.path().join("session-log.yaml").exists());
    assert!(
        tmp.path().join("session-log.md").exists(),
        "default is keep-originals"
    );

    let list = Command::new(bin())
        .args(["state", "session-log", "list"])
        .arg(tmp.path())
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        list.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(
        stdout.contains("2026-04-21-first-migrated"),
        "output must include first session id: {stdout}"
    );
    assert!(
        stdout.contains("2026-04-22-second-migrated"),
        "output must include second session id: {stdout}"
    );
}

#[test]
fn migrate_converts_latest_session_md_to_yaml() {
    let tmp = TempDir::new().unwrap();
    seed_latest_session_md(tmp.path());

    let migrate = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        migrate.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    assert!(tmp.path().join("latest-session.yaml").exists());

    let show = Command::new(bin())
        .args(["state", "session-log", "show-latest"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("2026-04-22-fresh-latest"), "id expected: {stdout}");
    assert!(stdout.contains("2026-04-22T12:00:00Z"), "timestamp expected: {stdout}");
}

#[test]
fn append_set_latest_and_show_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty plan; append creates session-log.yaml.
    let append = Command::new(bin())
        .args(["state", "session-log", "append"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-first"])
        .args(["--timestamp", "2026-04-22T14:00:00Z"])
        .args(["--phase", "work"])
        .args(["--body", "First body.\n"])
        .output()
        .unwrap();
    assert!(
        append.status.success(),
        "append failed: stderr={}",
        String::from_utf8_lossy(&append.stderr)
    );
    assert!(tmp.path().join("session-log.yaml").exists());

    // Idempotent re-append: same id → no-op.
    let append2 = Command::new(bin())
        .args(["state", "session-log", "append"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-first"])
        .args(["--timestamp", "2026-04-22T14:00:00Z"])
        .args(["--phase", "work"])
        .args(["--body", "First body.\n"])
        .output()
        .unwrap();
    assert!(append2.status.success());

    let list = Command::new(bin())
        .args(["state", "session-log", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let id_hits: usize = stdout.matches("id: 2026-04-22-first").count();
    assert_eq!(id_hits, 1, "idempotent append should not duplicate: {stdout}");

    // set-latest writes latest-session.yaml.
    let set_latest = Command::new(bin())
        .args(["state", "session-log", "set-latest"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-second"])
        .args(["--timestamp", "2026-04-22T15:00:00Z"])
        .args(["--body", "Second body.\n"])
        .output()
        .unwrap();
    assert!(set_latest.status.success());

    let show = Command::new(bin())
        .args(["state", "session-log", "show-latest"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("2026-04-22-second"), "id expected: {stdout}");

    // show <id> on the log returns the first session.
    let show_id = Command::new(bin())
        .args(["state", "session-log", "show"])
        .arg(tmp.path())
        .arg("2026-04-22-first")
        .output()
        .unwrap();
    assert!(show_id.status.success());
    let stdout = String::from_utf8(show_id.stdout).unwrap();
    assert!(stdout.contains("First body."), "body expected: {stdout}");
}

#[test]
fn migrate_handles_backlog_memory_and_session_log_together_in_one_run() {
    let tmp = TempDir::new().unwrap();
    seed_two_session_log_md(tmp.path());
    seed_latest_session_md(tmp.path());
    std::fs::write(
        tmp.path().join("backlog.md"),
        "\
### Solo task

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Body.

**Results:** _pending_

---
",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("memory.md"),
        "\
# Memory

## Alpha entry
Alpha body.
",
    )
    .unwrap();

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
    assert!(tmp.path().join("backlog.yaml").exists());
    assert!(tmp.path().join("memory.yaml").exists());
    assert!(tmp.path().join("session-log.yaml").exists());
    assert!(tmp.path().join("latest-session.yaml").exists());
}
