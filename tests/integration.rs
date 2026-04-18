use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::TempDir;

use raveloop::agent::Agent;
use raveloop::phase_loop::phase_loop;
use raveloop::types::{LlmPhase, PlanContext, SharedConfig};
use raveloop::ui::{UI, UIMessage, UISender};

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    assert!(!raveloop::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(100)).unwrap();
    raveloop::dream::update_dream_baseline(plan);

    fs::write(plan.join("memory.md"), "word ".repeat(200)).unwrap();
    assert!(!raveloop::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(2000)).unwrap();
    assert!(raveloop::dream::should_dream(plan, 1500));

    raveloop::dream::update_dream_baseline(plan);
    assert!(!raveloop::dream::should_dream(plan, 1500));
}

#[test]
fn config_loading_integration() {
    let dir = TempDir::new().unwrap();
    let config_root = dir.path();

    fs::write(config_root.join("config.yaml"), "agent: claude-code\nheadroom: 1500\n").unwrap();
    fs::create_dir_all(config_root.join("agents/claude-code")).unwrap();
    fs::write(
        config_root.join("agents/claude-code/config.yaml"),
        "models:\n  work: claude-sonnet-4-6\n  reflect: claude-haiku-4-5\nparams:\n  work:\n    dangerous: true\n",
    ).unwrap();
    fs::write(
        config_root.join("agents/claude-code/tokens.yaml"),
        "TOOL_READ: Read\n",
    ).unwrap();

    let shared = raveloop::config::load_shared_config(config_root).unwrap();
    assert_eq!(shared.agent, "claude-code");
    assert_eq!(shared.headroom, 1500);

    let agent = raveloop::config::load_agent_config(config_root, "claude-code").unwrap();
    assert_eq!(agent.models.get("work").unwrap(), "claude-sonnet-4-6");
    assert!(agent.params.get("work").unwrap().get("dangerous").is_some());

    let tokens = raveloop::config::load_tokens(config_root, "claude-code").unwrap();
    assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
}

#[test]
fn embedded_defaults_are_valid() {
    // init into a temp dir, then load every config with the real loaders.
    // Catches regressions where a default file drifts and stops parsing.
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("cfg");
    raveloop::init::run_init(&target, false).unwrap();

    let shared = raveloop::config::load_shared_config(&target).unwrap();
    assert!(!shared.agent.is_empty());
    assert!(shared.headroom > 0);

    let cc = raveloop::config::load_agent_config(&target, "claude-code").unwrap();
    assert!(cc.models.contains_key("reflect"));

    let pi = raveloop::config::load_agent_config(&target, "pi").unwrap();
    assert!(pi.models.contains_key("reflect"));

    // Every LLM phase in every embedded agent config must declare a
    // non-empty model string. An empty string silently delegates to
    // whatever `claude` / `pi` pick at spawn time, which is neither
    // auditable nor stable across releases.
    for (agent_name, cfg) in [("claude-code", &cc), ("pi", &pi)] {
        for phase in ["work", "analyse-work", "reflect", "dream", "triage"] {
            let model = cfg.models.get(phase).unwrap_or_else(|| {
                panic!("{agent_name} defaults missing model for phase {phase}")
            });
            assert!(
                !model.trim().is_empty(),
                "{agent_name} defaults have empty model for phase {phase}; pick an explicit default"
            );
        }
    }

    // Pi's `build_headless_args` falls back to `"anthropic"` when the
    // config omits `provider`, which is an implicit drift source — a
    // future provider change that only edits `PiAgent` will silently
    // disagree with the shipped config. Require the embedded default to
    // pin the value explicitly so the fallback only fires for
    // deliberately-minimal user configs.
    let pi_provider = pi.provider.as_ref()
        .expect("pi defaults must declare `provider` explicitly");
    assert!(
        !pi_provider.trim().is_empty(),
        "pi defaults have empty `provider`; pick an explicit default"
    );

    for phase in ["work", "analyse-work", "reflect", "dream", "triage"] {
        let p = target.join("phases").join(format!("{phase}.md"));
        assert!(p.exists(), "missing phase file: {}", p.display());
        let body = fs::read_to_string(&p).unwrap();
        assert!(!body.trim().is_empty(), "empty phase file: {}", p.display());
    }

    let survey = target.join("survey.md");
    assert!(survey.exists(), "missing survey prompt: {}", survey.display());
    let body = fs::read_to_string(&survey).unwrap();
    assert!(!body.trim().is_empty(), "empty survey prompt");
    let loaded = raveloop::survey::load_survey_prompt(&target).unwrap();
    assert_eq!(loaded, body);

    let create_plan = target.join("create-plan.md");
    assert!(create_plan.exists(), "missing create-plan prompt: {}", create_plan.display());
    let create_body = fs::read_to_string(&create_plan).unwrap();
    assert!(!create_body.trim().is_empty(), "empty create-plan prompt");
}

#[test]
fn survey_plan_discovery_across_multiple_roots() {
    // Two independent git projects, each with a plan-root subdirectory:
    //   tmp/ProjectA/.git
    //   tmp/ProjectA/LLM_STATE/plan-alpha/phase.md
    //   tmp/ProjectA/LLM_STATE/plan-beta/phase.md
    //   tmp/ProjectB/.git
    //   tmp/ProjectB/LLM_STATE/plan-gamma/phase.md
    // Project names should come from the git-root basenames
    // (ProjectA, ProjectB), NOT from the --root basename
    // (LLM_STATE in both cases).
    let tmp = TempDir::new().unwrap();
    let project_a = tmp.path().join("ProjectA");
    let project_b = tmp.path().join("ProjectB");
    let root_a = project_a.join("LLM_STATE");
    let root_b = project_b.join("LLM_STATE");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    fs::create_dir_all(project_a.join(".git")).unwrap();
    fs::create_dir_all(project_b.join(".git")).unwrap();

    for (root, plan_name, phase) in [
        (&root_a, "plan-alpha", "work"),
        (&root_a, "plan-beta", "triage"),
        (&root_b, "plan-gamma", "reflect"),
    ] {
        let dir = root.join(plan_name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("phase.md"), phase).unwrap();
        fs::write(dir.join("backlog.md"), format!("# backlog {plan_name}")).unwrap();
    }

    let plans_a = raveloop::survey::discover_plans(&root_a).unwrap();
    let plans_b = raveloop::survey::discover_plans(&root_b).unwrap();
    assert_eq!(plans_a.len(), 2);
    assert_eq!(plans_b.len(), 1);
    assert!(plans_a.iter().all(|p| p.project == "ProjectA"));
    assert!(plans_b.iter().all(|p| p.project == "ProjectB"));

    let mut all = Vec::new();
    all.extend(plans_a);
    all.extend(plans_b);
    let rendered = raveloop::survey::render_survey_input(&all);

    assert!(rendered.contains("## Plan: ProjectA/plan-alpha"));
    assert!(rendered.contains("## Plan: ProjectA/plan-beta"));
    assert!(rendered.contains("## Plan: ProjectB/plan-gamma"));
    assert!(rendered.contains("# backlog plan-alpha"));
    assert!(rendered.contains("### memory.md\n(missing)"));
}

struct MockAgent {
    calls: Arc<Mutex<Vec<LlmPhase>>>,
    next_phase_after: HashMap<LlmPhase, &'static str>,
    plan_dir: std::path::PathBuf,
}

#[async_trait]
impl Agent for MockAgent {
    async fn invoke_interactive(&self, _prompt: &str, _ctx: &PlanContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn invoke_headless(
        &self,
        _prompt: &str,
        _ctx: &PlanContext,
        phase: LlmPhase,
        _agent_id: &str,
        _tx: UISender,
    ) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(phase);
        if let Some(next) = self.next_phase_after.get(&phase) {
            fs::write(self.plan_dir.join("phase.md"), next)?;
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
        HashMap::new()
    }
}

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

fn init_test_repo(root: &std::path::Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["commit", "-q", "--allow-empty", "-m", "init"]);
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
    assert_eq!(final_phase.trim(), "work",
        "expected phase.md to advance to 'work' after git-commit-triage");

    let log = Command::new("git")
        .current_dir(root)
        .args(["log", "--oneline", "--all"])
        .output()
        .unwrap();
    let log_str = String::from_utf8(log.stdout).unwrap();
    assert!(log_str.contains("triage"),
        "expected a triage commit, got log:\n{log_str}");
}

/// Writes per-phase files matching the prompt contract the embedded
/// defaults describe. Exists so the integration test can swap in a
/// "well-behaved model" and detect drift between phase prompts and the
/// orchestrator's file-read expectations (latest-session.md,
/// commit-message.md, memory.md/backlog.md updates, phase.md
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
}

impl ContractMockAgent {
    fn new(plan_dir: std::path::PathBuf) -> Self {
        Self {
            plan_dir,
            commit_project_dir: None,
            captured_prompts: Arc::new(Mutex::new(HashMap::new())),
        }
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
                fs::write(
                    plan.join("commit-message.md"),
                    "analyse-work: contract test session\n\n\
                     Written by the ContractMockAgent to exercise the\n\
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
                fs::write(
                    plan.join("backlog.md"),
                    "# Backlog\n\n## Placeholder task\nAdded by contract test.\n",
                )?;
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
    raveloop::init::run_init(&config_root, false).unwrap();

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

    // The phase loop asks for confirmation at two points: after
    // git-commit-work ("Proceed to reflect phase?") and after
    // git-commit-triage ("Proceed to next work phase?"). Approve the
    // first so the full cycle runs, decline the second so the loop
    // exits without entering the interactive work phase.
    let drain = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                UIMessage::Quit => break,
                UIMessage::Confirm { message, reply } => {
                    let approve = !message.contains("next work phase");
                    let _ = reply.send(approve);
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

    // Contract assertion 2: commit-message.md was consumed by
    // git-commit-work and is no longer on disk.
    assert!(
        !plan_dir.join("commit-message.md").exists(),
        "commit-message.md should have been consumed by git-commit-work"
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

    // Contract assertion 6: each audit-trail commit was produced. The
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
        "expected analyse-work commit from custom commit-message.md, got log:\n{log_str}"
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
/// `ContractMockAgent`) flips the status to `done`. Declines the
/// "Proceed to reflect phase?" confirm to isolate the assertion from
/// subsequent phases that also rewrite `backlog.md`.
#[tokio::test]
async fn analyse_work_flips_stale_task_status_per_safety_net() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let config_root = root.join("config");
    raveloop::init::run_init(&config_root, false).unwrap();

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

    // Decline every confirm so the loop exits after git-commit-work,
    // before reflect/triage get a chance to rewrite backlog.md.
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
    raveloop::init::run_init(&config_root, false).unwrap();

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
        "expected the plan-state commit to use commit-message.md, got log:\n{log_str}"
    );
}
