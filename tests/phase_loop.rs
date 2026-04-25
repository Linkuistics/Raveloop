use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::TempDir;

use ravel_lite::agent::Agent;
use ravel_lite::phase_loop::phase_loop;
use ravel_lite::types::{LlmPhase, PlanContext, SharedConfig};
use ravel_lite::ui::{UIMessage, UISender, UI};

mod common;
use common::{init_test_repo, MockAgent};

/// Mirrors the analyse-work safety-net: within each `---`-delimited
/// task block, flip `Status: not_started` or `Status: in_progress`
/// to `Status: done` when a non-empty `Results:` block is present on
/// the same task. `_pending_` or an empty `Results:` marker means the
/// task isn't actually complete and should not be flipped.
fn flip_stale_task_statuses(backlog: &str) -> String {
    backlog
        .split("\n---")
        .map(|block| {
            let has_nonempty_results = block.lines().any(|line| {
                let trimmed = line.trim_start();
                if !trimmed.starts_with("**Results:**") {
                    return false;
                }
                let after = trimmed.trim_start_matches("**Results:**").trim();
                !after.is_empty() && after != "_pending_"
            });
            if !has_nonempty_results {
                return block.to_string();
            }
            block
                .replace("**Status:** `not_started`", "**Status:** `done`")
                .replace("**Status:** `in_progress`", "**Status:** `done`")
        })
        .collect::<Vec<_>>()
        .join("\n---")
}

#[derive(Clone, Debug)]
enum HandoffDisposition {
    Promote,
    Archive,
}

#[derive(Clone)]
struct HandoffInjection {
    target_task_title: String,
    handoff_title: String,
    handoff_body: String,
    disposition: HandoffDisposition,
}

/// Appends a `[HANDOFF] <title>` marker plus inlined body to the
/// matching task's block in `backlog.md`. Match is by exact `### <title>`
/// heading line. Panics if no matching block is found — callers seed
/// the backlog themselves.
fn inject_handoff_into_task_block(
    backlog: &str,
    target_task_title: &str,
    handoff_title: &str,
    handoff_body: &str,
) -> String {
    let target_heading = format!("### {target_task_title}");
    let mut found = false;
    let updated: Vec<String> = backlog
        .split("\n---")
        .map(|block| {
            let matches = block.lines().any(|line| line.trim_end() == target_heading);
            if !matches {
                return block.to_string();
            }
            found = true;
            format!(
                "{}\n\n[HANDOFF] {handoff_title}\n{handoff_body}\n",
                block.trim_end()
            )
        })
        .collect();
    assert!(
        found,
        "inject_handoff_into_task_block: no task '{target_task_title}' in backlog"
    );
    updated.join("\n---")
}

/// Scans a backlog block for a `[HANDOFF] <title>` marker and returns
/// `(title, body)` if found. The body runs from the line after the
/// marker up to the next blank line or end-of-block.
fn extract_handoff_from_block(block: &str) -> Option<(String, String)> {
    let mut lines = block.lines();
    while let Some(line) = lines.next() {
        if let Some(title) = line.strip_prefix("[HANDOFF] ") {
            let mut body_lines: Vec<&str> = Vec::new();
            for next in lines.by_ref() {
                if next.trim().is_empty() {
                    break;
                }
                body_lines.push(next);
            }
            return Some((title.trim().to_string(), body_lines.join("\n")));
        }
    }
    None
}

/// Writes per-phase files matching the prompt contract the embedded
/// defaults describe. Exists so the integration test can swap in a
/// "well-behaved model" and detect drift between phase prompts and the
/// orchestrator's file-read expectations (latest-session.md,
/// commits.yaml, memory.md/backlog.md updates, phase.md
/// transitions).
struct ContractMockAgent {
    plan_dir: std::path::PathBuf,
    /// When `Some`, the mock additionally simulates the analyse-work
    /// phase's source-commit step: it stages any path outside the plan
    /// directory that appears in the work-tree snapshot and commits it.
    /// This mirrors the behaviour of a well-behaved LLM following the
    /// updated analyse-work prompt — left opt-in so tests that don't
    /// care about commits stay fast.
    commit_project_dir: Option<std::path::PathBuf>,
    /// Captures the prompt text each headless phase received, keyed by
    /// phase. Tests inspect this to verify token substitution (e.g. that
    /// `{{WORK_TREE_STATUS}}` is replaced with real snapshot output).
    captured_prompts: Arc<Mutex<HashMap<LlmPhase, String>>>,
    /// When `Some`, analyse-work injects a `[HANDOFF]` marker into the
    /// named task's Results block and writes a matching `## Hand-offs`
    /// section to latest-session.md; triage then mines the marker and
    /// promotes or archives it per the disposition.
    handoff_injection: Option<HandoffInjection>,
}

impl ContractMockAgent {
    fn new(plan_dir: std::path::PathBuf) -> Self {
        Self {
            plan_dir,
            commit_project_dir: None,
            captured_prompts: Arc::new(Mutex::new(HashMap::new())),
            handoff_injection: None,
        }
    }

    fn with_handoff_injection(mut self, injection: HandoffInjection) -> Self {
        self.handoff_injection = Some(injection);
        self
    }
}

#[async_trait]
impl Agent for ContractMockAgent {
    async fn invoke_interactive(&self, _prompt: &str, _ctx: &PlanContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn invoke_headless(
        &self,
        prompt: &str,
        _ctx: &PlanContext,
        phase: LlmPhase,
        _agent_id: &str,
        _tx: UISender,
    ) -> anyhow::Result<()> {
        self.captured_prompts
            .lock()
            .unwrap()
            .insert(phase, prompt.to_string());
        let plan = &self.plan_dir;
        match phase {
            LlmPhase::AnalyseWork => {
                fs::write(
                    plan.join("latest-session.md"),
                    "### Session 1 (2026-04-18T00:00:00Z) — contract test\n\
                     - mock analyse-work output\n",
                )?;
                // R3: analyse-work also emits latest-session.yaml (the
                // typed surface). GitCommitWork will parse this and
                // append to session-log.yaml via the programmatic
                // `append_latest_to_log` entry point.
                fs::write(
                    plan.join("latest-session.yaml"),
                    "id: 2026-04-18-contract-test\n\
                     timestamp: 2026-04-18T00:00:00Z\n\
                     phase: work\n\
                     body: |\n  \
                       - mock analyse-work output\n",
                )?;
                fs::write(
                    plan.join("commits.yaml"),
                    "commits:\n\
                     - paths:\n  \
                         - .\n  \
                         message: |\n    \
                             analyse-work: contract test session\n\
                             \n    \
                             Written by the ContractMockAgent to exercise the\n    \
                             phase → file-write contract.\n",
                )?;
                // Safety-net simulation: a well-behaved model following
                // the analyse-work prompt flips stale Status: lines on
                // tasks whose Results: block is now non-empty. When the
                // pre-seeded backlog has no such tasks, this is a no-op.
                let backlog_path = plan.join("backlog.md");
                if let Ok(backlog) = fs::read_to_string(&backlog_path) {
                    let flipped = flip_stale_task_statuses(&backlog);
                    if flipped != backlog {
                        fs::write(&backlog_path, flipped)?;
                    }
                }
                // Hand-off injection: after the safety-net flip, a
                // well-behaved model following the analyse-work prompt's
                // fallback path appends a `[HANDOFF]` marker to the
                // completing task's Results block and mirrors the
                // hand-off into latest-session.md under `## Hand-offs`.
                // Triage mines the marker from the backlog next cycle.
                if let Some(injection) = &self.handoff_injection {
                    let backlog = fs::read_to_string(&backlog_path)?;
                    let injected = inject_handoff_into_task_block(
                        &backlog,
                        &injection.target_task_title,
                        &injection.handoff_title,
                        &injection.handoff_body,
                    );
                    fs::write(&backlog_path, injected)?;
                    fs::write(
                        plan.join("latest-session.md"),
                        format!(
                            "### Session 1 (2026-04-18T00:00:00Z) — handoff test\n\
                             - mock analyse-work output with handoff\n\n\
                             ## Hand-offs\n\n\
                             ### {}\n\
                             - {}\n",
                            injection.handoff_title, injection.handoff_body,
                        ),
                    )?;
                }
                // Source-commit simulation: a well-behaved model reads
                // the WORK_TREE_STATUS snapshot in the prompt, stages
                // every path outside the plan dir, and commits it with
                // a descriptive message. Opt-in via commit_project_dir.
                if let Some(project_dir) = &self.commit_project_dir {
                    let plan_rel = plan
                        .strip_prefix(project_dir)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let pathspec = format!(":(exclude){plan_rel}");
                    let add = Command::new("git")
                        .current_dir(project_dir)
                        .args(["add", "-A", "--", ".", &pathspec])
                        .output()?;
                    if !add.status.success() {
                        anyhow::bail!(
                            "mock git add failed: {}",
                            String::from_utf8_lossy(&add.stderr)
                        );
                    }
                    let diff = Command::new("git")
                        .current_dir(project_dir)
                        .args(["diff", "--cached", "--quiet"])
                        .output()?;
                    if !diff.status.success() {
                        let commit = Command::new("git")
                            .current_dir(project_dir)
                            .args(["commit", "-m", "mock: analyse-work source commit"])
                            .output()?;
                        if !commit.status.success() {
                            anyhow::bail!(
                                "mock git commit failed: {}",
                                String::from_utf8_lossy(&commit.stderr)
                            );
                        }
                    }
                }
                fs::write(plan.join("phase.md"), "git-commit-work")?;
            }
            LlmPhase::Reflect => {
                fs::write(
                    plan.join("memory.md"),
                    "# Memory\n\n## Mock learning\nExercised by the contract test.\n",
                )?;
                fs::write(plan.join("phase.md"), "git-commit-reflect")?;
            }
            LlmPhase::Dream => {
                // Not expected in this test (memory too small for headroom),
                // but handle gracefully so a future change doesn't crash the
                // mock.
                fs::write(plan.join("phase.md"), "git-commit-dream")?;
            }
            LlmPhase::Triage => {
                let backlog_path = plan.join("backlog.md");
                let existing = fs::read_to_string(&backlog_path).unwrap_or_default();
                let new_backlog = if let Some(injection) = &self.handoff_injection {
                    // Hand-off mining simulation: a well-behaved model
                    // scans every Status: done task for `[HANDOFF]`
                    // markers, promotes or archives each one per the
                    // disposition, and then deletes the done task. This
                    // is the production triage contract, minus the
                    // promote-vs-archive judgement call, which the test
                    // pins via the injection's disposition field.
                    let mut kept_blocks: Vec<String> = Vec::new();
                    let mut mined: Vec<(String, String)> = Vec::new();
                    for block in existing.split("\n---") {
                        let is_done = block.contains("**Status:** `done`");
                        if is_done {
                            if let Some(handoff) = extract_handoff_from_block(block) {
                                mined.push(handoff);
                            }
                            // Drop every done task — triage deletes them
                            // unconditionally after mining hand-offs.
                            continue;
                        }
                        kept_blocks.push(block.to_string());
                    }
                    let mut backlog_after = kept_blocks.join("\n---");
                    if let HandoffDisposition::Promote = injection.disposition {
                        for (title, body) in &mined {
                            backlog_after.push_str(&format!(
                                "\n### {title}\n\n\
                                 **Category:** `followup`\n\
                                 **Status:** `not_started`\n\
                                 **Dependencies:** none\n\n\
                                 **Description:**\n\n\
                                 {body}\n\n\
                                 **Results:** _pending_\n\n\
                                 ---\n"
                            ));
                        }
                    }
                    if let HandoffDisposition::Archive = injection.disposition {
                        let memory_path = plan.join("memory.md");
                        let mut memory = fs::read_to_string(&memory_path)
                            .unwrap_or_else(|_| "# Memory\n".to_string());
                        for (title, body) in &mined {
                            memory.push_str(&format!("\n## {title}\n{body}\n"));
                        }
                        fs::write(&memory_path, memory)?;
                    }
                    backlog_after
                } else if existing.trim().is_empty() {
                    // Default behaviour: append a placeholder so the
                    // contract test can observe that triage wrote
                    // backlog.md. Mirrors "models in production don't
                    // wipe the backlog each triage."
                    "# Backlog\n\n## Placeholder task\nAdded by contract test.\n".to_string()
                } else {
                    format!(
                        "{}\n## Placeholder task\nAdded by contract test.\n",
                        existing.trim_end()
                    )
                };
                fs::write(&backlog_path, new_backlog)?;
                fs::write(plan.join("phase.md"), "git-commit-triage")?;
            }
            LlmPhase::Work => {
                // Work is interactive in the phase loop; invoke_headless is
                // not expected to fire for it. Fall through without writing.
            }
        }
        Ok(())
    }

    async fn dispatch_subagent(
        &self,
        _prompt: &str,
        _target_plan: &str,
        _agent_id: &str,
        _tx: UISender,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn tokens(&self) -> HashMap<String, String> {
        // The phase prompts the orchestrator exercises headlessly
        // (analyse-work, reflect, triage) only reference the built-in
        // tokens (PLAN, PROJECT, ORCHESTRATOR, RELATED_PLANS). The work
        // prompt uses {{TOOL_READ}}, but this test never enters Work —
        // it declines the final confirm.
        HashMap::new()
    }
}

/// Shared fixture for the hand-off convention tests: a pre-seeded
/// backlog with one finished-but-unflipped task the safety-net will
/// flip to `done` during analyse-work, creating the hand-off mining
/// target triage then consumes.
const HANDOFF_TARGET_TASK_TITLE: &str = "Hand-off source task";

fn seed_handoff_backlog() -> String {
    format!(
        "\
# Backlog

## Tasks

### {HANDOFF_TARGET_TASK_TITLE}

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

A task that the work phase completed; analyse-work's safety-net will
flip it to done and the hand-off convention will attach a `[HANDOFF]`
marker to its Results block.

**Results:** Implemented the change and the test suite passed.

---
"
    )
}

#[tokio::test]
async fn phase_loop_triage_cycle_exits_cleanly_on_no_confirm() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let plan_dir = root.join("plans/test-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "triage").unwrap();

    let config_root = root.join("config");
    fs::create_dir_all(config_root.join("phases")).unwrap();
    fs::write(config_root.join("phases/triage.md"), "triage on {{PLAN}}").unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let agent = Arc::new(MockAgent {
        calls: calls.clone(),
        next_phase_after: HashMap::from([(LlmPhase::Triage, "git-commit-triage")]),
        plan_dir: plan_dir.clone(),
    });

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 1500,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Drain the channel, auto-reply "no" to confirms (simulates user pressing N).
    // Mirrors run_tui's shutdown protocol: break on Quit so drain.await can complete
    // even while the test still owns a live sender via `ui`.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    let calls = calls.lock().unwrap();
    assert_eq!(*calls, vec![LlmPhase::Triage]);

    let final_phase = fs::read_to_string(plan_dir.join("phase.md")).unwrap();
    assert_eq!(
        final_phase.trim(),
        "work",
        "expected phase.md to advance to 'work' after git-commit-triage"
    );

    let log = Command::new("git")
        .current_dir(root)
        .args(["log", "--oneline", "--all"])
        .output()
        .unwrap();
    let log_str = String::from_utf8(log.stdout).unwrap();
    assert!(
        log_str.contains("triage"),
        "expected a triage commit, got log:\n{log_str}"
    );
}

#[tokio::test]
async fn phase_contract_round_trip_writes_expected_files() {
    // Installs the real embedded defaults into a tempdir, runs a mock
    // agent that writes the files each phase prompt instructs a
    // well-behaved model to write, and asserts those files exist with
    // plausible contents after the cycle. If a phase prompt drifts to
    // a different filename, this test starts failing at the assertion
    // for that file — before a real run does in the field.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/contract-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    // Start at analyse-work: work is interactive and not part of this
    // test's contract (it has no file-write postcondition beyond what
    // analyse-work checks via the diff).
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();
    fs::write(plan_dir.join("backlog.md"), "# Backlog\n").unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();
    // analyse-work reads work-baseline to diff against; point it at the
    // repo's initial commit so any downstream `git diff` is well-formed.
    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    let agent = Arc::new(ContractMockAgent::new(plan_dir.clone()));

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000, // High: ensures dream never triggers.
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // The phase loop asks for confirmation once per full cycle: after
    // git-commit-triage ("Proceed to next work phase?"). Decline it so
    // the loop exits cleanly after one cycle without entering the
    // interactive work phase of the next.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    // Contract assertion 1: analyse-work produced latest-session.md.
    let latest = fs::read_to_string(plan_dir.join("latest-session.md"))
        .expect("latest-session.md should exist after analyse-work");
    assert!(
        latest.contains("### Session"),
        "latest-session.md should contain a Session heading, got:\n{latest}"
    );

    // Contract assertion 2: commits.yaml was consumed by
    // git-commit-work and is no longer on disk.
    assert!(
        !plan_dir.join("commits.yaml").exists(),
        "commits.yaml should have been consumed by git-commit-work"
    );

    // Contract assertion 3: reflect wrote memory.md in the expected
    // location.
    let memory = fs::read_to_string(plan_dir.join("memory.md"))
        .expect("memory.md should exist after reflect");
    assert!(
        memory.contains("Mock learning"),
        "reflect should have written memory.md with new content, got:\n{memory}"
    );

    // Contract assertion 4: triage wrote backlog.md in the expected
    // location.
    let backlog = fs::read_to_string(plan_dir.join("backlog.md"))
        .expect("backlog.md should exist after triage");
    assert!(
        backlog.contains("Placeholder task"),
        "triage should have updated backlog.md, got:\n{backlog}"
    );

    // Contract assertion 5: phase.md has advanced back to the start of
    // the next cycle.
    let final_phase = fs::read_to_string(plan_dir.join("phase.md")).unwrap();
    assert_eq!(
        final_phase.trim(),
        "work",
        "expected phase.md to advance to 'work' after git-commit-triage"
    );

    // Contract assertion 6 (R3): GitCommitWork appended latest-session.yaml
    // to session-log.yaml. The record is identified by its session id,
    // not a tail-string match — the new append is idempotent on id.
    let session_log_yaml = fs::read_to_string(plan_dir.join("session-log.yaml"))
        .expect("session-log.yaml should exist after git-commit-work");
    assert!(
        session_log_yaml.contains("id: 2026-04-18-contract-test"),
        "session-log.yaml should contain the appended session id, got:\n{session_log_yaml}"
    );

    // Contract assertion 7: each audit-trail commit was produced. The
    // analyse-work commit message is the one the mock wrote; reflect
    // and triage fall back to the default message shape.
    let log = Command::new("git")
        .current_dir(root)
        .args(["log", "--oneline", "--all"])
        .output()
        .unwrap();
    let log_str = String::from_utf8(log.stdout).unwrap();
    assert!(
        log_str.contains("analyse-work: contract test session"),
        "expected analyse-work commit from custom commits.yaml, got log:\n{log_str}"
    );
    assert!(
        log_str.contains("reflect"),
        "expected a reflect commit, got log:\n{log_str}"
    );
    assert!(
        log_str.contains("triage"),
        "expected a triage commit, got log:\n{log_str}"
    );
}

/// Pins the analyse-work safety-net: when a task has a non-empty
/// `Results:` block but its `Status:` line is still `not_started` or
/// `in_progress`, a well-behaved model (as simulated by
/// `ContractMockAgent`) flips the status to `done`. Runs the full
/// cycle (no pre-reflect gate exists to stop at) and relies on the
/// mock's append-only triage to preserve analyse-work's edits into
/// the final on-disk backlog.
#[tokio::test]
async fn analyse_work_flips_stale_task_status_per_safety_net() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/stale-status-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();

    // A task that the (hypothetical) work phase finished — the Results
    // block is populated — but whose Status: line was never flipped.
    // Exactly the drift the safety-net catches.
    let stale_backlog = "\
# Backlog

## Tasks

### Example finished task

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Placeholder for the safety-net test.

**Results:** Implemented the fix and ran the test suite. It passed.

---
";
    fs::write(plan_dir.join("backlog.md"), stale_backlog).unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();

    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    let agent = Arc::new(ContractMockAgent::new(plan_dir.clone()));

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Decline every confirm — only the post-triage gate exists now,
    // so this simply stops the loop after the full cycle completes.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    let backlog = fs::read_to_string(plan_dir.join("backlog.md")).unwrap();
    assert!(
        backlog.contains("**Status:** `done`"),
        "analyse-work should have flipped the stale Status line to done, got:\n{backlog}"
    );
    assert!(
        !backlog.contains("**Status:** `not_started`"),
        "analyse-work should have flipped the stale Status line away from not_started, got:\n{backlog}"
    );
}

/// End-to-end coverage for the `[HANDOFF]` convention, promote path:
/// analyse-work attaches a hand-off marker to a completing task's
/// Results block, git-commit-work commits plan state, then triage
/// mines the marker and promotes it to a new `not_started` backlog
/// task while deleting the original done task. Guards the pipeline
/// that `defaults/phases/analyse-work.md` and `defaults/phases/triage.md`
/// jointly implement.
#[tokio::test]
async fn handoff_marker_in_analyse_work_is_promoted_by_triage() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/handoff-promote-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();
    fs::write(plan_dir.join("backlog.md"), seed_handoff_backlog()).unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();

    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    let handoff_title = "Follow-up: extract shared helper".to_string();
    let handoff_body = "Problem: three callers duplicate the block-split parse. \
         Decision: introduce `parse_backlog_blocks()` in `src/backlog.rs`. \
         References: tests/integration.rs:570 (flip_stale_task_statuses)."
        .to_string();

    let agent = Arc::new(
        ContractMockAgent::new(plan_dir.clone()).with_handoff_injection(HandoffInjection {
            target_task_title: HANDOFF_TARGET_TASK_TITLE.to_string(),
            handoff_title: handoff_title.clone(),
            handoff_body: handoff_body.clone(),
            disposition: HandoffDisposition::Promote,
        }),
    );

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    // The original completed task is gone (triage deletes done tasks
    // after mining).
    let backlog = fs::read_to_string(plan_dir.join("backlog.md")).unwrap();
    assert!(
        !backlog.contains(HANDOFF_TARGET_TASK_TITLE),
        "triage should delete the original done task after mining hand-offs, got:\n{backlog}"
    );

    // A promoted task carries the hand-off title as its heading and is
    // not_started, with the inlined body preserved.
    assert!(
        backlog.contains(&format!("### {handoff_title}")),
        "triage should promote the hand-off into a new backlog task, got:\n{backlog}"
    );
    assert!(
        backlog.contains("**Status:** `not_started`"),
        "promoted hand-off task should be not_started, got:\n{backlog}"
    );
    assert!(
        backlog.contains("introduce `parse_backlog_blocks()`"),
        "promoted task should preserve the inlined hand-off body, got:\n{backlog}"
    );

    // memory.md is untouched by triage in the promote path — reflect's
    // write is the final memory state.
    let memory = fs::read_to_string(plan_dir.join("memory.md")).unwrap();
    assert!(
        !memory.contains(&handoff_title),
        "promote disposition should not write the hand-off to memory.md, got:\n{memory}"
    );

    // The analyse-work prompt actually loaded (smoke: the captured
    // prompt contains the hand-off convention text from the embedded
    // default prompt, confirming no {{…}} token drift blocked it).
    // We verify the mirrored latest-session.md hand-off block survives
    // the cycle — latest-session.md is only overwritten by analyse-work.
    let latest = fs::read_to_string(plan_dir.join("latest-session.md")).unwrap();
    assert!(
        latest.contains("## Hand-offs"),
        "latest-session.md should retain the ## Hand-offs section, got:\n{latest}"
    );
    assert!(
        latest.contains(&format!("### {handoff_title}")),
        "latest-session.md should list the hand-off by title, got:\n{latest}"
    );
}

/// End-to-end coverage for the `[HANDOFF]` convention, archive path:
/// analyse-work attaches a hand-off marker, triage archives it to
/// `memory.md` (not a new backlog task) and deletes the original done
/// task. Complementary to the promote test — both dispositions must
/// survive the cycle.
#[tokio::test]
async fn handoff_marker_in_analyse_work_is_archived_by_triage() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/handoff-archive-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();
    fs::write(plan_dir.join("backlog.md"), seed_handoff_backlog()).unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();

    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    let handoff_title = "Strategic: revisit parser architecture".to_string();
    let handoff_body = "Not concrete enough for a backlog task. Keep as memory: the \
         markdown-first approach has friction points we should revisit \
         once three more plans are in flight."
        .to_string();

    let agent = Arc::new(
        ContractMockAgent::new(plan_dir.clone()).with_handoff_injection(HandoffInjection {
            target_task_title: HANDOFF_TARGET_TASK_TITLE.to_string(),
            handoff_title: handoff_title.clone(),
            handoff_body: handoff_body.clone(),
            disposition: HandoffDisposition::Archive,
        }),
    );

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    // Original done task deleted; no new backlog entry for the hand-off.
    let backlog = fs::read_to_string(plan_dir.join("backlog.md")).unwrap();
    assert!(
        !backlog.contains(HANDOFF_TARGET_TASK_TITLE),
        "triage should delete the original done task, got:\n{backlog}"
    );
    assert!(
        !backlog.contains(&format!("### {handoff_title}")),
        "archive disposition should not create a new backlog task, got:\n{backlog}"
    );

    // The hand-off lives in memory.md instead.
    let memory = fs::read_to_string(plan_dir.join("memory.md")).unwrap();
    assert!(
        memory.contains(&format!("## {handoff_title}")),
        "triage should append the archived hand-off as a memory entry, got:\n{memory}"
    );
    assert!(
        memory.contains("markdown-first approach has friction"),
        "archived hand-off should preserve the body, got:\n{memory}"
    );
}

/// End-to-end check that the orchestrator's work-tree snapshot reaches
/// the analyse-work prompt AND that a well-behaved model acting on the
/// snapshot commits uncommitted source files. Mirrors the production
/// hand-off: work phase leaves source edits in the tree, analyse-work
/// sees them via `{{WORK_TREE_STATUS}}`, commits them.
#[tokio::test]
async fn analyse_work_receives_snapshot_and_commits_uncommitted_source() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/snapshot-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();
    fs::write(plan_dir.join("backlog.md"), "# Backlog\n").unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();

    // Baseline captured BEFORE the simulated work-phase edits.
    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    // Simulate the work phase leaving a source file uncommitted — the
    // exact hand-off shape the snapshot is supposed to surface.
    fs::write(
        root.join("abandoned_by_work.rs"),
        "fn added_in_work_phase() {}\n",
    )
    .unwrap();

    let agent = Arc::new(ContractMockAgent {
        plan_dir: plan_dir.clone(),
        commit_project_dir: Some(root.to_path_buf()),
        captured_prompts: Arc::new(Mutex::new(HashMap::new())),
        handoff_injection: None,
    });
    let captured = agent.captured_prompts.clone();

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Decline every confirm — we only care about the analyse-work →
    // git-commit-work hand-off.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    // Contract 1: the analyse-work prompt actually received a
    // substituted WORK_TREE_STATUS block — no leftover `{{...}}`
    // placeholder, and the uncommitted file is named in it.
    let prompts = captured.lock().unwrap();
    let analyse_prompt = prompts
        .get(&LlmPhase::AnalyseWork)
        .expect("analyse-work should have been invoked");
    assert!(
        !analyse_prompt.contains("{{WORK_TREE_STATUS}}"),
        "analyse-work prompt still has unsubstituted WORK_TREE_STATUS token"
    );
    assert!(
        !analyse_prompt.contains("{{BACKLOG_TRANSITIONS}}"),
        "analyse-work prompt still has unsubstituted BACKLOG_TRANSITIONS token"
    );
    assert!(
        analyse_prompt.contains("abandoned_by_work.rs"),
        "analyse-work prompt should surface the uncommitted source file in the snapshot; got prompt:\n{analyse_prompt}"
    );

    // Contract 2: the source file is no longer in the working tree as
    // untracked — the well-behaved mock committed it.
    let status = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    let status_str = String::from_utf8(status.stdout).unwrap();
    assert!(
        !status_str.contains("abandoned_by_work.rs"),
        "analyse-work mock should have committed the source file, but porcelain still lists it:\n{status_str}"
    );

    // Contract 3: git log contains a dedicated source commit separate
    // from the plan-state commit.
    let log = Command::new("git")
        .current_dir(root)
        .args(["log", "--oneline", "--all"])
        .output()
        .unwrap();
    let log_str = String::from_utf8(log.stdout).unwrap();
    assert!(
        log_str.contains("mock: analyse-work source commit"),
        "expected a source commit from the analyse-work mock, got log:\n{log_str}"
    );
    assert!(
        log_str.contains("analyse-work: contract test session"),
        "expected the plan-state commit to use commits.yaml, got log:\n{log_str}"
    );
}

/// Invariant: at the user-prompt that follows git-commit-triage, the
/// plan tree must be fully committed. A dirty phase.md (or work-baseline,
/// or latest-session.md) here leaks into sibling plans in multi-plan
/// monorepos — `warn_if_project_tree_dirty` scans the whole project dir
/// and treats the leak as "work agent forgot to commit", which is a
/// false positive that has caused operator confusion in the field.
#[tokio::test]
async fn git_commit_triage_leaves_plan_tree_clean_at_user_prompt() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let plan_dir = root.join("plans/clean-triage-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "triage").unwrap();

    let config_root = root.join("config");
    fs::create_dir_all(config_root.join("phases")).unwrap();
    fs::write(config_root.join("phases/triage.md"), "triage on {{PLAN}}").unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let agent = Arc::new(MockAgent {
        calls: calls.clone(),
        next_phase_after: HashMap::from([(LlmPhase::Triage, "git-commit-triage")]),
        plan_dir: plan_dir.clone(),
    });

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 1500,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Decline the "proceed to next work phase?" prompt so the loop exits
    // at the very spot where the invariant is supposed to hold.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    let status = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--", "plans/clean-triage-plan"])
        .output()
        .unwrap();
    let dirty = String::from_utf8(status.stdout).unwrap();
    assert!(
        dirty.is_empty(),
        "plan tree should be clean after git-commit-triage, but porcelain shows:\n{dirty}"
    );
}

/// Invariant: at the user-prompt that follows git-commit-work, the plan
/// tree must be fully committed. Same rationale as the triage variant —
/// sibling plans in the same repo should never observe a dirty plan dir
/// at a user decision point.
#[tokio::test]
async fn git_commit_work_leaves_plan_tree_clean_at_user_prompt() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let plan_dir = root.join("plans/clean-work-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "analyse-work").unwrap();
    fs::write(plan_dir.join("backlog.md"), "# Backlog\n").unwrap();
    fs::write(plan_dir.join("memory.md"), "# Memory\n").unwrap();

    let head = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    fs::write(plan_dir.join("work-baseline"), &head.stdout).unwrap();

    let agent = Arc::new(ContractMockAgent::new(plan_dir.clone()));

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 10_000,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Decline the "proceed to reflect phase?" prompt (the first one
    // after analyse-work → git-commit-work). That is exactly the
    // checkpoint where the invariant must hold.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;

    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    let status = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--", "plans/clean-work-plan"])
        .output()
        .unwrap();
    let dirty = String::from_utf8(status.stdout).unwrap();
    assert!(
        dirty.is_empty(),
        "plan tree should be clean after git-commit-work, but porcelain shows:\n{dirty}"
    );
}

/// Invariant: after git-commit-triage, `work-baseline` contains the SHA
/// of the triage commit — not the SHA of the prior (reflect) commit.
///
/// The saved SHA is the baseline the next cycle's analyse-work diffs
/// `{{BACKLOG_TRANSITIONS}}` against. If it points at the reflect
/// commit, every next-cycle diff conflates this cycle's triage
/// mutations with the next cycle's work changes — tasks deleted in
/// triage look like tasks deleted during work, and tasks added in
/// triage look like tasks added during work.
#[tokio::test]
async fn git_commit_triage_records_work_baseline_at_triage_commit_sha() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let plan_dir = root.join("plans/baseline-sha-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "triage").unwrap();
    // Seed a plan-dir file so the triage commit has real content to
    // record — otherwise `git_commit_plan` would short-circuit on an
    // empty diff and HEAD wouldn't advance.
    fs::write(plan_dir.join("backlog.md"), "# Backlog\n").unwrap();

    let config_root = root.join("config");
    fs::create_dir_all(config_root.join("phases")).unwrap();
    fs::write(config_root.join("phases/triage.md"), "triage on {{PLAN}}").unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let agent = Arc::new(MockAgent {
        calls: calls.clone(),
        next_phase_after: HashMap::from([(LlmPhase::Triage, "git-commit-triage")]),
        plan_dir: plan_dir.clone(),
    });

    let shared = SharedConfig {
        agent: "mock".into(),
        headroom: 1500,
    };

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { reply, .. } => {
                    let _ = reply.send(false);
                }
                _ => {}
            }
        }
    });

    let result = phase_loop(agent, &ctx, &shared, &ui).await;
    ui.quit();
    let _ = drain.await;
    assert!(result.is_ok(), "phase_loop returned error: {result:?}");

    // Resolve the SHA of the commit whose message starts with
    // `run-plan: triage`. That is the commit `work-baseline` should
    // name — distinct from the reflect-era HEAD the prior code saved
    // and distinct from the follow-on `save-work-baseline` commit.
    let triage_sha = String::from_utf8(
        Command::new("git")
            .current_dir(root)
            .args(["log", "--grep=^run-plan: triage", "--format=%H", "-n", "1"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert!(
        !triage_sha.is_empty(),
        "expected a commit with message starting with 'run-plan: triage'"
    );

    let baseline = fs::read_to_string(plan_dir.join("work-baseline"))
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        baseline, triage_sha,
        "work-baseline should name the triage commit SHA, not the reflect HEAD"
    );

    // The follow-on commit that persists `work-baseline` must be
    // present — otherwise the plan tree would be dirty at the user
    // prompt (guarded by a separate invariant test).
    let has_save_commit = Command::new("git")
        .current_dir(root)
        .args(["log", "--grep=^run-plan: save-work-baseline", "--format=%H", "-n", "1"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8(has_save_commit.stdout).unwrap().trim().is_empty(),
        "expected a follow-on 'save-work-baseline' commit alongside the triage commit"
    );
}
