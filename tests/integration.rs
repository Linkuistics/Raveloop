use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tempfile::TempDir;

use ravel_lite::agent::Agent;
use ravel_lite::phase_loop::phase_loop;
use ravel_lite::types::{LlmPhase, PlanContext, SharedConfig};
use ravel_lite::ui::{UI, UIMessage, UISender};

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(100)).unwrap();
    ravel_lite::dream::update_dream_baseline(plan);

    fs::write(plan.join("memory.md"), "word ".repeat(200)).unwrap();
    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(2000)).unwrap();
    assert!(ravel_lite::dream::should_dream(plan, 1500));

    ravel_lite::dream::update_dream_baseline(plan);
    assert!(!ravel_lite::dream::should_dream(plan, 1500));
}

/// Bootstrap regression: a plan without a `dream-baseline` file must
/// have one seeded when the loop enters `git-commit-reflect`. Before
/// this was wired in, `should_dream` returned `false` unconditionally
/// on any plan whose baseline file was never created — and since
/// `update_dream_baseline` only fires *after* a dream runs, that's a
/// permanent deadlock that keeps dream from ever triggering.
///
/// (State-level redundant seeding also happens in `run_set_phase`;
/// this test pins the `GitCommitReflect` layer independently.)
#[tokio::test]
async fn git_commit_reflect_seeds_dream_baseline_when_missing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_test_repo(root);

    let plan_dir = root.join("plans/no-baseline-plan");
    fs::create_dir_all(&plan_dir).unwrap();
    // Start at git-commit-reflect; this is the script phase that gates
    // dream and is therefore the correct seeding point.
    fs::write(plan_dir.join("phase.md"), "git-commit-reflect").unwrap();
    // 300-word memory: well below headroom (1500), so with baseline
    // seeded to 0 the guard still returns false (300 > 0 + 1500 is
    // false) and the loop proceeds to triage rather than dream.
    fs::write(plan_dir.join("memory.md"), "word ".repeat(300)).unwrap();
    // Critical precondition: no dream-baseline on disk.
    assert!(!plan_dir.join("dream-baseline").exists());

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

    // Decline the "Proceed to next work phase?" confirm so the loop
    // exits cleanly after git-commit-triage without entering a
    // spurious work phase.
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

    // Core assertion: git-commit-reflect seeded the baseline file
    // to 0 (the "never-dreamed" sentinel).
    let baseline = fs::read_to_string(plan_dir.join("dream-baseline"))
        .expect("dream-baseline must exist after git-commit-reflect");
    assert_eq!(
        baseline.trim(),
        "0",
        "baseline must be seeded to 0 — the 'never dreamed' sentinel"
    );

    // Secondary assertion: with baseline=0 and memory=300 < headroom,
    // the guard returns false and the loop skips dream → triage runs.
    let calls = calls.lock().unwrap();
    assert_eq!(
        *calls,
        vec![LlmPhase::Triage],
        "expected loop to skip dream (memory within headroom) and enter triage"
    );
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

    let shared = ravel_lite::config::load_shared_config(config_root).unwrap();
    assert_eq!(shared.agent, "claude-code");
    assert_eq!(shared.headroom, 1500);

    let agent = ravel_lite::config::load_agent_config(config_root, "claude-code").unwrap();
    assert_eq!(agent.models.get("work").unwrap(), "claude-sonnet-4-6");
    assert!(agent.params.get("work").unwrap().get("dangerous").is_some());

    let tokens = ravel_lite::config::load_tokens(config_root, "claude-code").unwrap();
    assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
}

#[test]
fn embedded_defaults_are_valid() {
    // init into a temp dir, then load every config with the real loaders.
    // Catches regressions where a default file drifts and stops parsing.
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("cfg");
    ravel_lite::init::run_init(&target, false).unwrap();

    let shared = ravel_lite::config::load_shared_config(&target).unwrap();
    assert!(!shared.agent.is_empty());
    assert!(shared.headroom > 0);

    let cc = ravel_lite::config::load_agent_config(&target, "claude-code").unwrap();
    assert!(cc.models.contains_key("reflect"));

    let pi = ravel_lite::config::load_agent_config(&target, "pi").unwrap();
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
    let loaded = ravel_lite::survey::load_survey_prompt(&target).unwrap();
    assert_eq!(loaded, body);

    let create_plan = target.join("create-plan.md");
    assert!(create_plan.exists(), "missing create-plan prompt: {}", create_plan.display());
    let create_body = fs::read_to_string(&create_plan).unwrap();
    assert!(!create_body.trim().is_empty(), "empty create-plan prompt");
}

#[test]
fn survey_loads_plans_from_multiple_projects_individually_named() {
    // Two independent git projects; the CLI names each plan directly,
    // no plan-root walk. Project names should come from the git-root
    // basenames (ProjectA, ProjectB), NOT from any intermediate
    // directory basename.
    let tmp = TempDir::new().unwrap();
    let project_a = tmp.path().join("ProjectA");
    let project_b = tmp.path().join("ProjectB");
    let plan_alpha = project_a.join("LLM_STATE").join("plan-alpha");
    let plan_beta = project_a.join("LLM_STATE").join("plan-beta");
    let plan_gamma = project_b.join("LLM_STATE").join("plan-gamma");
    fs::create_dir_all(project_a.join(".git")).unwrap();
    fs::create_dir_all(project_b.join(".git")).unwrap();

    for (plan_dir, plan_name, phase) in [
        (&plan_alpha, "plan-alpha", "work"),
        (&plan_beta, "plan-beta", "triage"),
        (&plan_gamma, "plan-gamma", "reflect"),
    ] {
        fs::create_dir_all(plan_dir).unwrap();
        fs::write(plan_dir.join("phase.md"), phase).unwrap();
        fs::write(plan_dir.join("backlog.md"), format!("# backlog {plan_name}")).unwrap();
    }

    let alpha = ravel_lite::survey::load_plan(&plan_alpha).unwrap();
    let beta = ravel_lite::survey::load_plan(&plan_beta).unwrap();
    let gamma = ravel_lite::survey::load_plan(&plan_gamma).unwrap();
    assert_eq!(alpha.project, "ProjectA");
    assert_eq!(beta.project, "ProjectA");
    assert_eq!(gamma.project, "ProjectB");

    // Each plan has its own SHA-256 hash (distinct inputs → distinct hashes).
    assert_ne!(alpha.input_hash, beta.input_hash);
    assert_ne!(alpha.input_hash, gamma.input_hash);
    assert_eq!(alpha.input_hash.len(), 64);

    let all = vec![alpha, beta, gamma];
    let rendered = ravel_lite::survey::render_survey_input(&all);
    assert!(rendered.contains("## Plan: ProjectA/plan-alpha"));
    assert!(rendered.contains("## Plan: ProjectA/plan-beta"));
    assert!(rendered.contains("## Plan: ProjectB/plan-gamma"));
    assert!(rendered.contains("# backlog plan-alpha"));
    assert!(rendered.contains("### memory.md\n(missing)"));
}

#[test]
fn survey_yaml_emit_injects_input_hashes_and_round_trips() {
    // Full pipeline sanity check: load two plans → simulate an LLM
    // YAML response that doesn't include input_hash → inject hashes
    // post-parse → emit canonical YAML → re-parse → byte-identical on
    // second emission.
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_a = project.join("plan-a");
    let plan_b = project.join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_a.join("backlog.md"), "# a").unwrap();
    fs::write(plan_b.join("phase.md"), "triage").unwrap();
    fs::write(plan_b.join("backlog.md"), "# b").unwrap();

    let snapshot_a = ravel_lite::survey::load_plan(&plan_a).unwrap();
    let snapshot_b = ravel_lite::survey::load_plan(&plan_b).unwrap();

    // Simulated LLM response: no input_hash field, matches discovered plans.
    let llm_yaml = "plans:\n  \
        - project: Proj\n    plan: plan-a\n    phase: work\n    unblocked: 1\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n  \
        - project: Proj\n    plan: plan-b\n    phase: triage\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n";

    let mut response = ravel_lite::survey::parse_survey_response(llm_yaml).unwrap();
    let hashes: std::collections::HashMap<String, String> = [
        (ravel_lite::survey::plan_key(&snapshot_a.project, &snapshot_a.plan), snapshot_a.input_hash.clone()),
        (ravel_lite::survey::plan_key(&snapshot_b.project, &snapshot_b.plan), snapshot_b.input_hash.clone()),
    ]
    .into_iter()
    .collect();
    ravel_lite::survey::inject_input_hashes(&mut response, &hashes).unwrap();

    let first_emit = ravel_lite::survey::emit_survey_yaml(&response).unwrap();
    assert!(first_emit.contains(&format!("input_hash: {}", snapshot_a.input_hash)));
    assert!(first_emit.contains(&format!("input_hash: {}", snapshot_b.input_hash)));

    let reparsed = ravel_lite::survey::parse_survey_response(&first_emit).unwrap();
    let second_emit = ravel_lite::survey::emit_survey_yaml(&reparsed).unwrap();
    assert_eq!(first_emit, second_emit, "round-trip must be byte-identical");
}

#[test]
fn survey_format_renders_markdown_matching_direct_render() {
    // Golden: `survey-format <file>` over a known YAML produces the
    // same markdown that `render_survey_output` produces when called
    // directly on the parsed struct. This pins the separation of
    // persistence (YAML) from presentation (markdown).
    let yaml = "plans:\n  \
        - project: P\n    plan: x\n    phase: work\n    unblocked: 2\n    blocked: 0\n    done: 0\n    received: 0\n    notes: note-x\n";
    let parsed = ravel_lite::survey::parse_survey_response(yaml).unwrap();
    let expected = ravel_lite::survey::render_survey_output(&parsed);

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("survey.yaml");
    fs::write(&path, yaml).unwrap();

    // run_survey_format writes to stdout — we can't easily capture it
    // without spawning a subprocess, so we verify the underlying
    // contract: the YAML on disk parses back to the same struct that
    // render_survey_output will receive.
    let reloaded = ravel_lite::survey::parse_survey_response(
        &fs::read_to_string(&path).unwrap(),
    )
    .unwrap();
    assert_eq!(ravel_lite::survey::render_survey_output(&reloaded), expected);
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
                // Append rather than overwrite so tests that seed backlog
                // content before the cycle (e.g. the safety-net test) can
                // still observe analyse-work's earlier status flips after
                // the full cycle runs. Models in production don't wipe
                // the backlog each triage either.
                let backlog_path = plan.join("backlog.md");
                let existing = fs::read_to_string(&backlog_path).unwrap_or_default();
                let appended = if existing.trim().is_empty() {
                    "# Backlog\n\n## Placeholder task\nAdded by contract test.\n".to_string()
                } else {
                    format!(
                        "{}\n## Placeholder task\nAdded by contract test.\n",
                        existing.trim_end()
                    )
                };
                fs::write(&backlog_path, appended)?;
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

/// End-to-end coverage for the real `PiAgent` impl. The earlier mocks
/// substitute for the `Agent` trait; nothing here exercised the concrete
/// pi spawn/stream/dispatch path. That gap is how the `{{MEMORY_DIR}}`
/// regression — pi prompts loaded via ad-hoc `str::replace`, bypassing
/// `substitute_tokens` — escaped into a real session without a single
/// failing test.
///
/// These tests run a fake `pi` binary on PATH (`pi` is a small shell
/// script the test writes into a tempdir). Three behaviours are pinned:
/// the phase-cycle entry point (token substitution + stream-event
/// fan-out + audit commit), the stderr-tail surfacing on non-zero exit,
/// and the dispatch-subagent argv contract.
mod pi_integration {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::OnceLock;

    use ravel_lite::agent::pi::PiAgent;
    use ravel_lite::types::AgentConfig;

    /// Process-wide lock around env-var mutation. `cargo test` runs test
    /// functions concurrently within a binary; PATH (and HOME) are
    /// process-global, so any test that prepends a fake-pi tempdir must
    /// hold this lock for the duration of the call to keep concurrent
    /// tests from clobbering each other's spawn environment.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Holds an env-var override for the lifetime of the value; restores
    /// the prior environment in `Drop` *before* the mutex guard releases
    /// (struct-field drop order: Drop::drop body first, then fields in
    /// declaration order — `_guard` last). Lock-poisoning from a panicked
    /// previous holder is recovered, since the env state is restored on
    /// every drop regardless of test outcome.
    struct EnvOverride {
        saved_path: Option<String>,
        saved_home: Option<Option<String>>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvOverride {
        fn new(bin_dir: &Path) -> Self {
            let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
            let saved_path = std::env::var("PATH").ok();
            let new_path = match &saved_path {
                Some(p) => format!("{}:{}", bin_dir.display(), p),
                None => bin_dir.display().to_string(),
            };
            unsafe {
                std::env::set_var("PATH", new_path);
            }
            Self {
                saved_path,
                saved_home: None,
                _guard: guard,
            }
        }

        fn with_home(mut self, home: &Path) -> Self {
            self.saved_home = Some(std::env::var("HOME").ok());
            unsafe {
                std::env::set_var("HOME", home);
            }
            self
        }
    }

    impl Drop for EnvOverride {
        fn drop(&mut self) {
            unsafe {
                match &self.saved_path {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
                if let Some(prev) = &self.saved_home {
                    match prev {
                        Some(h) => std::env::set_var("HOME", h),
                        None => std::env::remove_var("HOME"),
                    }
                }
            }
        }
    }

    fn write_fake_pi(bin_dir: &Path, body: &str) {
        let pi_path = bin_dir.join("pi");
        fs::write(&pi_path, body).unwrap();
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn empty_agent_config() -> AgentConfig {
        AgentConfig {
            models: HashMap::new(),
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: Some("anthropic".to_string()),
        }
    }

    fn write_minimal_pi_system_prompt(config_root: &Path) {
        let prompts_dir = config_root.join("agents/pi/prompts");
        fs::create_dir_all(&prompts_dir).unwrap();
        fs::write(
            prompts_dir.join("system-prompt.md"),
            "minimal pi system prompt for tests\n",
        )
        .unwrap();
    }

    /// Drains a `UIMessage` channel into a vector of variant-name strings
    /// so tests can assert that the right UIMessage variants flowed
    /// without depending on payload shape.
    fn message_kind(msg: &UIMessage) -> Option<&'static str> {
        match msg {
            UIMessage::Progress { .. } => Some("Progress"),
            UIMessage::Persist { .. } => Some("Persist"),
            UIMessage::AgentDone { .. } => Some("AgentDone"),
            _ => None,
        }
    }

    /// Phase-cycle pin: drives `analyse-work` through `phase_loop` with a
    /// real `PiAgent` and a fake `pi` on PATH. Asserts:
    ///   1. The prompt that reached `pi -p` has zero unresolved `{{...}}`
    ///      tokens — catches `{{MEMORY_DIR}}`-class regressions where a
    ///      pi prompt-loading path bypasses `substitute_tokens`.
    ///   2. The substituted prompt embeds the real PLAN path — catches
    ///      regressions where substitution is *attempted* but resolves
    ///      to the empty string.
    ///   3. The fake pi's scripted JSON events fan out to the right
    ///      `UIMessage` variants (Progress for `tool_execution_start`,
    ///      Persist for `message_end`, AgentDone on close).
    ///   4. The cycle advances to `git-commit-work` and produces an
    ///      audit commit using the agent's `commit-message.md`.
    #[tokio::test]
    async fn pi_phase_cycle_substitutes_tokens_and_streams_events() {
        let bin_dir = TempDir::new().unwrap();
        let fake_home = TempDir::new().unwrap();
        let _env = EnvOverride::new(bin_dir.path()).with_home(fake_home.path());

        // Pre-seed the fake home so `PiAgent::setup` skips `pi install`.
        let pi_settings_dir = fake_home.path().join(".pi/agent");
        fs::create_dir_all(&pi_settings_dir).unwrap();
        fs::write(
            pi_settings_dir.join("settings.json"),
            r#"{"packages":["@mjakl/pi-subagent"]}"#,
        )
        .unwrap();

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        init_test_repo(root);

        let config_root = root.join("config");
        ravel_lite::init::run_init(&config_root, false).unwrap();

        let plan_dir = root.join("plans/pi-cycle-plan");
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

        // Fake pi: dump the prompt arg, and advance the cycle. For the
        // analyse-work phase specifically, also write the session contract
        // files so that leg of the cycle exercises the prompt-substitution
        // and audit-commit path. For reflect and triage it just advances
        // phase.md to the next script phase — otherwise the cycle (which
        // now runs headless past the former pre-reflect gate) would spin
        // forever between GitCommitWork and a reflect step that re-wrote
        // phase.md back to `git-commit-work`.
        let prompt_dump = root.join("captured-prompt.txt");
        let template = r#"#!/bin/sh
prompt=""
while [ $# -gt 0 ]; do
    case "$1" in
        -p) shift; prompt="$1"; shift ;;
        *) shift ;;
    esac
done
printf '%s' "$prompt" > '__PROMPT_DUMP__'
current=$(cat '__PLAN_DIR__/phase.md' 2>/dev/null || echo '')
case "$current" in
    analyse-work)
        cat > '__PLAN_DIR__/latest-session.md' <<'SESSION_EOF'
### Session 1 (2026-04-19T00:00:00Z) — pi mock
- minimal session entry
SESSION_EOF
        cat > '__PLAN_DIR__/commit-message.md' <<'COMMIT_EOF'
analyse-work: pi mock session

Written by the fake pi binary in pi_phase_cycle test.
COMMIT_EOF
        printf 'git-commit-work' > '__PLAN_DIR__/phase.md'
        ;;
    reflect)
        printf 'git-commit-reflect' > '__PLAN_DIR__/phase.md'
        ;;
    triage)
        printf 'git-commit-triage' > '__PLAN_DIR__/phase.md'
        ;;
esac
echo '{"type":"tool_execution_start","tool_name":"read","tool_input":{"file_path":"/x.md"}}'
echo '{"type":"message_end","content":[{"type":"text","text":"done"}]}'
"#;
        let pi_script = template
            .replace("__PROMPT_DUMP__", &prompt_dump.display().to_string())
            .replace("__PLAN_DIR__", &plan_dir.display().to_string());
        write_fake_pi(bin_dir.path(), &pi_script);

        let agent_config = ravel_lite::config::load_agent_config(&config_root, "pi").unwrap();
        let agent = Arc::new(PiAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        ));

        let shared = SharedConfig {
            agent: "pi".into(),
            headroom: 10_000,
        };

        let ctx = PlanContext {
            plan_dir: plan_dir.to_string_lossy().to_string(),
            project_dir: root.to_string_lossy().to_string(),
            dev_root: root.parent().unwrap().to_string_lossy().to_string(),
            related_plans: String::new(),
            config_root: config_root.to_string_lossy().to_string(),
        };

        let observed = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let observed_for_drain = observed.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ui = UI::new(tx);

        let drain = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Some(kind) = message_kind(&msg) {
                    observed_for_drain.lock().unwrap().push(kind);
                }
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

        let captured = fs::read_to_string(&prompt_dump)
            .expect("fake pi should have dumped its prompt arg to disk");
        assert!(
            !captured.contains("{{"),
            "prompt sent to pi still has unsubstituted tokens: {captured}"
        );
        let plan_str = plan_dir.to_string_lossy();
        assert!(
            captured.contains(plan_str.as_ref()),
            "prompt should embed substituted PLAN path; captured:\n{captured}"
        );

        let observed = observed.lock().unwrap();
        assert!(
            observed.contains(&"Progress"),
            "tool_execution_start should yield a Progress UIMessage; saw {observed:?}"
        );
        assert!(
            observed.contains(&"Persist"),
            "message_end should yield a Persist UIMessage; saw {observed:?}"
        );
        assert!(
            observed.contains(&"AgentDone"),
            "headless invocation should signal AgentDone; saw {observed:?}"
        );

        let log = Command::new("git")
            .current_dir(root)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        let log_str = String::from_utf8(log.stdout).unwrap();
        assert!(
            log_str.contains("analyse-work: pi mock session"),
            "expected analyse-work commit using commit-message.md, got log:\n{log_str}"
        );
    }

    /// Pins the stderr-capture fix: a non-zero pi exit must surface the
    /// stderr tail in the returned error so operators don't see a bare
    /// "pi exited with code 17" with no clue why. Previously `pi` ran
    /// with `Stdio::inherit()` for stderr, which let raw stderr bleed
    /// into the terminal underneath the TUI and get overwritten on the
    /// next repaint — invisibly losing the very output the user needed.
    #[tokio::test]
    async fn pi_invoke_headless_surfaces_stderr_tail_on_failure() {
        let bin_dir = TempDir::new().unwrap();
        let _env = EnvOverride::new(bin_dir.path());

        write_fake_pi(
            bin_dir.path(),
            "#!/bin/sh\necho 'boom: pi exploded' 1>&2\nexit 17\n",
        );

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();
        let config_root = project_dir.join("config");
        write_minimal_pi_system_prompt(&config_root);

        let agent = PiAgent::new(
            empty_agent_config(),
            config_root.to_string_lossy().to_string(),
        );
        let ctx = PlanContext {
            plan_dir: project_dir.to_string_lossy().to_string(),
            project_dir: project_dir.to_string_lossy().to_string(),
            dev_root: project_dir.to_string_lossy().to_string(),
            related_plans: String::new(),
            config_root: config_root.to_string_lossy().to_string(),
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let drain = tokio::spawn(async move {
            while rx.recv().await.is_some() {}
        });

        let err = agent
            .invoke_headless("a prompt", &ctx, LlmPhase::Reflect, "test", tx)
            .await
            .expect_err("non-zero pi exit should surface as an error");
        drain.abort();

        let msg = format!("{err:#}");
        assert!(
            msg.contains("boom: pi exploded"),
            "error message should include stderr tail; got: {msg}"
        );
        assert!(
            msg.contains("17"),
            "error message should include exit code; got: {msg}"
        );
    }

    /// Pins the dispatch-subagent argv contract: the dispatched pi child
    /// must receive `--no-session`, `--mode json`, `--provider …`, and
    /// the prompt under `-p`, with the project root resolved from the
    /// target plan as `cwd`. A regression here is the kind of silent
    /// drift the existing trait-mock tests cannot catch — they never
    /// touch the real argv builder.
    #[tokio::test]
    async fn pi_dispatch_subagent_invokes_pi_with_target_plan_args() {
        let bin_dir = TempDir::new().unwrap();
        let _env = EnvOverride::new(bin_dir.path());

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();
        init_test_repo(project_dir);
        let config_root = project_dir.join("config");
        write_minimal_pi_system_prompt(&config_root);

        let target_plan = project_dir.join("plans/dispatch-target");
        fs::create_dir_all(&target_plan).unwrap();

        let argv_dump = project_dir.join("captured-args.txt");
        let template = r#"#!/bin/sh
for a; do
    printf '%s\n' "$a"
done > '__ARGV_DUMP__'
echo '{"type":"message_end","content":[{"type":"text","text":"done"}]}'
"#;
        let pi_script = template.replace("__ARGV_DUMP__", &argv_dump.display().to_string());
        write_fake_pi(bin_dir.path(), &pi_script);

        let agent = PiAgent::new(
            empty_agent_config(),
            config_root.to_string_lossy().to_string(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let drain = tokio::spawn(async move {
            while rx.recv().await.is_some() {}
        });

        let result = agent
            .dispatch_subagent("dispatch prompt", target_plan.to_str().unwrap(), "sub", tx)
            .await;
        drain.abort();

        assert!(result.is_ok(), "dispatch should succeed: {result:?}");

        let argv = fs::read_to_string(&argv_dump).expect("script should have dumped argv");
        let args: Vec<&str> = argv.lines().collect();
        for required in [
            "--no-session",
            "--append-system-prompt",
            "--provider",
            "anthropic",
            "--mode",
            "json",
            "-p",
            "dispatch prompt",
        ] {
            assert!(
                args.contains(&required),
                "dispatched argv missing `{required}`: {args:?}"
            );
        }
    }
}

// ===================== pivot: types + serde =====================

#[test]
fn pivot_stack_roundtrip_minimal_frame() {
    use ravel_lite::pivot::{Frame, Stack};
    use std::path::PathBuf;

    let s = Stack {
        frames: vec![Frame {
            path: PathBuf::from("LLM_STATE/ravel-orchestrator"),
            pushed_at: None,
            reason: None,
        }],
    };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: Stack = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
    // Minimal frames omit pushed_at/reason when serialized
    assert!(!yaml.contains("pushed_at"));
    assert!(!yaml.contains("reason"));
}

#[test]
fn pivot_stack_roundtrip_full_frame() {
    use ravel_lite::pivot::{Frame, Stack};
    use std::path::PathBuf;

    let s = Stack {
        frames: vec![
            Frame {
                path: PathBuf::from("LLM_STATE/ravel-orchestrator"),
                pushed_at: None,
                reason: None,
            },
            Frame {
                path: PathBuf::from("LLM_STATE/sub-F-hierarchy"),
                pushed_at: Some("2026-04-20T18:32:14Z".to_string()),
                reason: Some("sub-D/T2 landed".to_string()),
            },
        ],
    };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: Stack = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn pivot_max_stack_depth_constant() {
    assert_eq!(ravel_lite::pivot::MAX_STACK_DEPTH, 5);
}

#[test]
fn pivot_read_stack_missing_returns_none() {
    use ravel_lite::pivot::read_stack;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("stack.yaml");
    assert!(read_stack(&path).unwrap().is_none());
}

#[test]
fn pivot_read_stack_present_returns_some() {
    use ravel_lite::pivot::{read_stack, write_stack, Frame, Stack};
    use std::path::PathBuf;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("stack.yaml");
    let s = Stack {
        frames: vec![Frame {
            path: PathBuf::from("LLM_STATE/x"),
            pushed_at: None,
            reason: None,
        }],
    };
    write_stack(&path, &s).unwrap();
    let back = read_stack(&path).unwrap().unwrap();
    assert_eq!(back, s);
}

#[test]
fn pivot_read_stack_corrupt_yaml_returns_error_with_context() {
    use ravel_lite::pivot::read_stack;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("stack.yaml");
    fs::write(&path, "not: valid: yaml: at: all:\n  - [\n").unwrap();

    let err = read_stack(&path).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("stack.yaml"), "error should mention the file: {msg}");
}

#[test]
fn pivot_write_stack_creates_parent_if_needed() {
    use ravel_lite::pivot::{write_stack, Stack};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("some/nested/stack.yaml");
    // Parent doesn't exist; write should succeed by creating parents.
    write_stack(&path, &Stack::default()).unwrap();
    assert!(path.exists());
}

// ===================== pivot: validate_push =====================

fn _make_plan_dir(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("phase.md"), "work\n").unwrap();
    p
}

#[test]
fn pivot_validate_push_accepts_valid_target_below_cap() {
    use ravel_lite::pivot::{validate_push, Frame, Stack};
    let tmp = TempDir::new().unwrap();
    let root = _make_plan_dir(tmp.path(), "root");
    let child = _make_plan_dir(tmp.path(), "child");

    let stack = Stack {
        frames: vec![Frame {
            path: root.clone(),
            pushed_at: None,
            reason: None,
        }],
    };
    let new_frame = Frame {
        path: child,
        pushed_at: None,
        reason: None,
    };
    validate_push(&stack, &new_frame).unwrap();
}

#[test]
fn pivot_validate_push_rejects_depth_cap() {
    use ravel_lite::pivot::{validate_push, Frame, Stack, MAX_STACK_DEPTH};
    let tmp = TempDir::new().unwrap();

    let mut frames = vec![];
    for i in 0..MAX_STACK_DEPTH {
        frames.push(Frame {
            path: _make_plan_dir(tmp.path(), &format!("p{i}")),
            pushed_at: None,
            reason: None,
        });
    }
    let stack = Stack { frames };
    let new_frame = Frame {
        path: _make_plan_dir(tmp.path(), "overflow"),
        pushed_at: None,
        reason: None,
    };
    let err = validate_push(&stack, &new_frame).unwrap_err();
    assert!(format!("{err:#}").to_lowercase().contains("depth"));
}

#[test]
fn pivot_validate_push_rejects_cycle() {
    use ravel_lite::pivot::{validate_push, Frame, Stack};
    let tmp = TempDir::new().unwrap();
    let a = _make_plan_dir(tmp.path(), "a");
    let b = _make_plan_dir(tmp.path(), "b");

    let stack = Stack {
        frames: vec![
            Frame { path: a.clone(), pushed_at: None, reason: None },
            Frame { path: b, pushed_at: None, reason: None },
        ],
    };
    let dup = Frame { path: a, pushed_at: None, reason: None };
    let err = validate_push(&stack, &dup).unwrap_err();
    assert!(format!("{err:#}").to_lowercase().contains("cycle"));
}

#[test]
fn pivot_validate_push_rejects_nonexistent_path() {
    use ravel_lite::pivot::{validate_push, Frame, Stack};
    let tmp = TempDir::new().unwrap();
    let root = _make_plan_dir(tmp.path(), "root");

    let stack = Stack {
        frames: vec![Frame { path: root, pushed_at: None, reason: None }],
    };
    let bogus = Frame {
        path: tmp.path().join("does_not_exist"),
        pushed_at: None,
        reason: None,
    };
    let err = validate_push(&stack, &bogus).unwrap_err();
    assert!(format!("{err:#}").to_lowercase().contains("does not exist")
        || format!("{err:#}").to_lowercase().contains("invalid pivot"));
}

#[test]
fn pivot_validate_push_rejects_path_without_phase_md() {
    use ravel_lite::pivot::{validate_push, Frame, Stack};
    let tmp = TempDir::new().unwrap();
    let root = _make_plan_dir(tmp.path(), "root");

    // Create a directory that is NOT a plan (no phase.md).
    let fake = tmp.path().join("not_a_plan");
    fs::create_dir_all(&fake).unwrap();

    let stack = Stack {
        frames: vec![Frame { path: root, pushed_at: None, reason: None }],
    };
    let bogus = Frame { path: fake, pushed_at: None, reason: None };
    let err = validate_push(&stack, &bogus).unwrap_err();
    assert!(format!("{err:#}").to_lowercase().contains("phase.md")
        || format!("{err:#}").to_lowercase().contains("invalid pivot"));
}

#[test]
fn pivot_validate_push_rejects_cycle_with_mixed_path_forms() {
    // Regression test: cycle detection must canonicalize both sides before
    // comparing. The existing frame carries a canonical absolute path (as
    // produced by frame_to_context → plan_dir), while the new frame comes
    // from on_disk_new_top which deserialises whatever the agent wrote —
    // often a path with a redundant "./" prefix. Raw PathBuf equality would
    // fail to detect the cycle; canonicalization must bridge the gap.
    use ravel_lite::pivot::{validate_push, Frame, Stack};
    let tmp = TempDir::new().unwrap();
    // Create a real plan directory so is_dir() and phase.md checks pass if
    // the cycle guard is bypassed (they must NOT be reached — cycle wins).
    let root_abs = _make_plan_dir(tmp.path(), "root_abs");
    let canonical = root_abs.canonicalize().unwrap();

    // Existing frame: canonical absolute path (mirrors what stack_snapshot produces).
    let stack = Stack {
        frames: vec![Frame {
            path: canonical,
            pushed_at: None,
            reason: None,
        }],
    };

    // New frame: same logical directory but with a "./" prefix — raw PathBuf
    // inequality, but canonicalizes to the same path.
    let dotslash_path = tmp.path().join("./root_abs");
    let new_frame = Frame {
        path: dotslash_path,
        pushed_at: None,
        reason: None,
    };

    let err = validate_push(&stack, &new_frame).unwrap_err();
    assert!(
        format!("{err:#}").to_lowercase().contains("cycle"),
        "expected cycle error, got: {err:#}"
    );
}

#[test]
fn pivot_decide_after_work_normal_cycle() {
    use ravel_lite::pivot::{decide_after_work, NextAfterWork};
    use ravel_lite::types::LlmPhase;

    let action = decide_after_work(LlmPhase::AnalyseWork, false, None);
    assert!(matches!(action, NextAfterWork::ContinueNormalCycle));
}

#[test]
fn pivot_decide_after_work_stateful_pivot() {
    use ravel_lite::pivot::{decide_after_work, Frame, NextAfterWork};
    use ravel_lite::types::LlmPhase;
    use std::path::PathBuf;

    let frame = Frame {
        path: PathBuf::from("/some/plan"),
        pushed_at: None,
        reason: None,
    };
    let action = decide_after_work(LlmPhase::AnalyseWork, true, Some(frame.clone()));
    match action {
        NextAfterWork::PushAfterCycle(f) => assert_eq!(f, frame),
        _ => panic!("expected PushAfterCycle"),
    }
}

#[test]
fn pivot_decide_after_work_stateless_short_circuit() {
    use ravel_lite::pivot::{decide_after_work, Frame, NextAfterWork};
    use ravel_lite::types::LlmPhase;
    use std::path::PathBuf;

    let frame = Frame {
        path: PathBuf::from("/some/plan"),
        pushed_at: None,
        reason: None,
    };
    let action = decide_after_work(LlmPhase::Work, true, Some(frame.clone()));
    match action {
        NextAfterWork::PushImmediately(f) => assert_eq!(f, frame),
        _ => panic!("expected PushImmediately"),
    }
}

#[test]
fn pivot_decide_after_work_no_advance_no_pivot_is_error() {
    use ravel_lite::pivot::{decide_after_work, NextAfterWork};
    use ravel_lite::types::LlmPhase;

    let action = decide_after_work(LlmPhase::Work, false, None);
    match action {
        NextAfterWork::Error(msg) => {
            assert!(msg.to_lowercase().contains("phase did not advance"));
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn pivot_decide_after_cycle_stateful_pivot_push() {
    use ravel_lite::pivot::{decide_after_cycle, Frame, NextAfterCycle};
    use std::path::PathBuf;

    let frame = Frame {
        path: PathBuf::from("/x"),
        pushed_at: None,
        reason: None,
    };
    let action = decide_after_cycle(1, true, Some(frame.clone()));
    match action {
        NextAfterCycle::Push(f) => assert_eq!(f, frame),
        _ => panic!("expected Push"),
    }
}

#[test]
fn pivot_decide_after_cycle_pop_when_nested() {
    use ravel_lite::pivot::{decide_after_cycle, NextAfterCycle};

    let action = decide_after_cycle(2, false, None);
    assert!(matches!(action, NextAfterCycle::Pop));
}

#[test]
fn pivot_decide_after_cycle_continue_at_root() {
    use ravel_lite::pivot::{decide_after_cycle, NextAfterCycle};

    let action = decide_after_cycle(1, false, None);
    assert!(matches!(action, NextAfterCycle::Continue));
}

#[test]
fn pivot_decide_after_cycle_push_takes_precedence_over_pop() {
    use ravel_lite::pivot::{decide_after_cycle, Frame, NextAfterCycle};
    use std::path::PathBuf;

    let frame = Frame {
        path: PathBuf::from("/deeper"),
        pushed_at: None,
        reason: None,
    };
    // A nested plan wrote a new top during its own cycle: go deeper,
    // don't pop. (A plan that both writes a new top AND wants to be
    // popped is ill-formed; push wins.)
    let action = decide_after_cycle(2, true, Some(frame.clone()));
    match action {
        NextAfterCycle::Push(f) => assert_eq!(f, frame),
        _ => panic!("expected Push to take precedence"),
    }
}

#[test]
fn pivot_frame_to_context_resolves_project_dir_by_walkup() {
    use ravel_lite::pivot::{frame_to_context, Frame};
    use std::path::{Path, PathBuf};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Set up: root/.git, root/LLM_STATE/sub-F/
    fs::create_dir_all(root.join(".git")).unwrap();
    let plan = root.join("LLM_STATE").join("sub-F");
    fs::create_dir_all(&plan).unwrap();
    fs::write(plan.join("phase.md"), "work\n").unwrap();

    let frame = Frame {
        path: plan.clone(),
        pushed_at: None,
        reason: None,
    };
    let root_config_root = "/tmp/cfg".to_string();
    let ctx = frame_to_context(&frame, &root_config_root).unwrap();

    // canonicalize expected paths to match macOS /private/var resolution
    let canonical_plan = plan.canonicalize().unwrap();
    let canonical_root = root.canonicalize().unwrap();

    assert_eq!(PathBuf::from(&ctx.plan_dir), canonical_plan);
    assert_eq!(PathBuf::from(&ctx.project_dir), canonical_root);
    assert_eq!(ctx.config_root, root_config_root);
    assert_eq!(Path::new(&ctx.dev_root), canonical_root.parent().unwrap());
}

#[test]
fn pivot_frame_to_context_errors_without_git_root() {
    use ravel_lite::pivot::{frame_to_context, Frame};

    let tmp = TempDir::new().unwrap();
    let plan = tmp.path().join("lonely_plan");
    fs::create_dir_all(&plan).unwrap();
    fs::write(plan.join("phase.md"), "work\n").unwrap();

    let frame = Frame { path: plan, pushed_at: None, reason: None };
    let res = frame_to_context(&frame, "/tmp/cfg");
    assert!(res.is_err());
}

#[test]
fn pivot_breadcrumb_single_plan() {
    use ravel_lite::phase_loop::format_breadcrumb;
    let s = format_breadcrumb(&[std::path::PathBuf::from("/repo/LLM_STATE/root-plan")]);
    assert_eq!(s, "root-plan");
}

#[test]
fn pivot_breadcrumb_nested_two_deep() {
    use ravel_lite::phase_loop::format_breadcrumb;
    let s = format_breadcrumb(&[
        std::path::PathBuf::from("/repo/LLM_STATE/coord"),
        std::path::PathBuf::from("/repo/LLM_STATE/sub-F-hierarchy"),
    ]);
    assert_eq!(s, "coord → sub-F-hierarchy");
}

#[test]
fn pivot_breadcrumb_handles_three_deep() {
    use ravel_lite::phase_loop::format_breadcrumb;
    let s = format_breadcrumb(&[
        std::path::PathBuf::from("/repo/coord"),
        std::path::PathBuf::from("/repo/sub-F"),
        std::path::PathBuf::from("/repo/sub-F-sub1"),
    ]);
    assert_eq!(s, "coord → sub-F → sub-F-sub1");
}

#[tokio::test]
async fn pivot_run_stack_single_plan_completes_one_cycle() {
    use ravel_lite::phase_loop::run_stack;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    Command::new("git").arg("init").current_dir(root).output().unwrap();
    Command::new("git").args(["config", "user.email", "t@t"]).current_dir(root).output().unwrap();
    Command::new("git").args(["config", "user.name", "t"]).current_dir(root).output().unwrap();

    let plan = root.join("LLM_STATE").join("solo");
    fs::create_dir_all(&plan).unwrap();
    // Start at triage so MockAgent (which uses invoke_headless) can advance the phase.
    // git-commit-triage advances phase.md to "work" then asks the confirm; drain replies
    // false so run_stack exits after one triage cycle — no pivot logic triggered.
    fs::write(plan.join("phase.md"), "triage\n").unwrap();

    let config_root = root.join("config");
    fs::create_dir_all(config_root.join("phases")).unwrap();
    fs::write(config_root.join("phases/triage.md"), "triage on {{PLAN}}\n").unwrap();

    Command::new("git").args(["add", "."]).current_dir(root).output().unwrap();
    Command::new("git").args(["commit", "-m", "init"]).current_dir(root).output().unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let agent = Arc::new(MockAgent {
        calls: calls.clone(),
        next_phase_after: HashMap::from([(LlmPhase::Triage, "git-commit-triage")]),
        plan_dir: plan.clone(),
    });

    let ctx = ravel_lite::types::PlanContext {
        plan_dir: plan.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };
    let cfg = SharedConfig { agent: "mock".into(), headroom: 1500 };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Drain the UI channel; reply false to confirm so the loop exits after
    // one triage cycle (run_stack has no pivot logic at this stage).
    let drainer = tokio::spawn(async move {
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

    // Should complete without attempting any pivot.
    let result = run_stack(agent.clone(), ctx, &cfg, &ui).await;
    ui.quit();
    let _ = drainer.await;
    assert!(result.is_ok(), "expected clean exit, got: {result:?}");

    // Verify the single plan's phase.md advanced to "work" (git-commit-triage writes it).
    let phase = fs::read_to_string(plan.join("phase.md")).unwrap();
    assert_eq!(phase.trim(), "work");

    // No stack.yaml should have been created.
    assert!(!plan.join("stack.yaml").exists());
}

/// Short-circuit pivot: coordinator's work phase writes stack.yaml with a
/// child frame and leaves phase.md at "work". The driver detects the new
/// top, pushes child immediately, runs child's cycle, pops back to coord,
/// then resumes coord's work phase. On exit, stack.yaml must be absent.
#[tokio::test]
async fn pivot_run_stack_short_circuit_pivot() {
    use ravel_lite::phase_loop::run_stack;
    use ravel_lite::pivot::{Frame, Stack};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    Command::new("git").arg("init").current_dir(root).output().unwrap();
    Command::new("git").args(["config", "user.email", "t@t"]).current_dir(root).output().unwrap();
    Command::new("git").args(["config", "user.name", "t"]).current_dir(root).output().unwrap();

    let coord = root.join("LLM_STATE").join("coord");
    fs::create_dir_all(&coord).unwrap();
    fs::write(coord.join("phase.md"), "work\n").unwrap();

    let child = root.join("LLM_STATE").join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("phase.md"), "work\n").unwrap();

    // Config prompts for every phase the cycle can reach. The pre-reflect
    // gate used to exit the child before reflect/triage were loaded, so
    // earlier versions of this test only seeded analyse-work and work.
    let config_root = root.join("config");
    fs::create_dir_all(config_root.join("phases")).unwrap();
    fs::write(config_root.join("phases/analyse-work.md"), "analyse-work {{PLAN}}\n").unwrap();
    fs::write(config_root.join("phases/work.md"), "work {{PLAN}}\n").unwrap();
    fs::write(config_root.join("phases/reflect.md"), "reflect {{PLAN}}\n").unwrap();
    fs::write(config_root.join("phases/triage.md"), "triage {{PLAN}}\n").unwrap();
    fs::write(config_root.join("phases/dream.md"), "dream {{PLAN}}\n").unwrap();

    Command::new("git").args(["add", "."]).current_dir(root).output().unwrap();
    Command::new("git").args(["commit", "-m", "init"]).current_dir(root).output().unwrap();

    // Canonical paths for assertions — frame_to_context resolves symlinks
    // (e.g. /tmp → /private/tmp on macOS), so comparisons use canonical forms.
    let coord_canon = coord.canonicalize().unwrap();
    let child_canon = child.canonicalize().unwrap();

    let calls: Arc<Mutex<Vec<(std::path::PathBuf, LlmPhase)>>> = Arc::new(Mutex::new(vec![]));

    // Number of times coord's work phase has been invoked.
    let coord_work_count = Arc::new(AtomicUsize::new(0));

    struct PivotMockAgent {
        calls: Arc<Mutex<Vec<(std::path::PathBuf, LlmPhase)>>>,
        /// Raw (non-canonical) coord path, for matching the root_ctx.plan_dir
        /// which run_stack never canonicalizes.
        coord_raw: std::path::PathBuf,
        coord: std::path::PathBuf,
        child: std::path::PathBuf,
        coord_work_count: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ravel_lite::agent::Agent for PivotMockAgent {
        async fn invoke_interactive(&self, _prompt: &str, ctx: &PlanContext) -> anyhow::Result<()> {
            let plan_dir = std::path::PathBuf::from(&ctx.plan_dir);
            // Canonicalize for consistent comparison regardless of symlink resolution.
            let plan_dir_canon = plan_dir.canonicalize().unwrap_or(plan_dir.clone());
            self.calls.lock().unwrap().push((plan_dir_canon.clone(), LlmPhase::Work));

            if plan_dir_canon == self.coord {
                let n = self.coord_work_count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    // First coord work: short-circuit pivot — write stack.yaml,
                    // leave phase.md at "work" so the driver sees PushImmediately.
                    let stack = Stack {
                        frames: vec![
                            Frame { path: self.coord_raw.clone(), pushed_at: None, reason: None },
                            Frame { path: self.child.clone(), pushed_at: None, reason: Some("test".into()) },
                        ],
                    };
                    let y = serde_yaml::to_string(&stack).unwrap();
                    fs::write(self.coord_raw.join("stack.yaml"), y).unwrap();
                    // Leave phase.md at "work" (short-circuit pivot signal).
                } else {
                    // Second coord work (after pop): advance normally so the loop
                    // can reach git-commit-work whose confirm exits cleanly.
                    fs::write(plan_dir.join("phase.md"), "analyse-work\n").unwrap();
                }
            } else {
                // Child work: advance phase normally.
                fs::write(plan_dir.join("phase.md"), "analyse-work\n").unwrap();
            }
            Ok(())
        }

        async fn invoke_headless(
            &self,
            _prompt: &str,
            ctx: &PlanContext,
            phase: LlmPhase,
            _agent_id: &str,
            _tx: UISender,
        ) -> anyhow::Result<()> {
            let plan_dir = std::path::PathBuf::from(&ctx.plan_dir);
            let plan_dir_canon = plan_dir.canonicalize().unwrap_or(plan_dir.clone());
            self.calls.lock().unwrap().push((plan_dir_canon, phase));

            // Advance to the next script phase so the orchestrator can commit.
            let next = match phase {
                LlmPhase::AnalyseWork => "git-commit-work",
                LlmPhase::Reflect => "git-commit-reflect",
                LlmPhase::Dream => "git-commit-dream",
                LlmPhase::Triage => "git-commit-triage",
                LlmPhase::Work => unreachable!("work is interactive"),
            };
            fs::write(plan_dir.join("phase.md"), format!("{next}\n")).unwrap();
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

    let agent = Arc::new(PivotMockAgent {
        calls: calls.clone(),
        coord_raw: coord.clone(),
        coord: coord_canon.clone(),
        child: child_canon.clone(),
        coord_work_count: coord_work_count.clone(),
    });

    let ctx = PlanContext {
        plan_dir: coord.to_string_lossy().to_string(),
        project_dir: root.to_string_lossy().to_string(),
        dev_root: root.parent().unwrap().to_string_lossy().to_string(),
        related_plans: String::new(),
        config_root: config_root.to_string_lossy().to_string(),
    };
    let cfg = SharedConfig { agent: "mock".into(), headroom: 1500 };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Explicit-false drainer: reply false to every confirm (all confirms decline).
    let drainer = tokio::spawn(async move {
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

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_stack(agent.clone(), ctx, &cfg, &ui),
    )
    .await
    .unwrap_or_else(|_| panic!("test hung: run_stack did not complete within 10s"));

    ui.quit();
    let _ = drainer.await;

    assert!(result.is_ok(), "expected clean exit, got: {result:?}");

    let log = calls.lock().unwrap();

    // Minimum assertions (all comparisons use canonical paths, since
    // frame_to_context resolves symlinks via canonicalize on macOS):
    //   1. First call is coord.Work (short-circuit pivot).
    //   2. Child was invoked at some point.
    //   3. stack.yaml is absent (deleted after pop to depth 1).
    assert!(!log.is_empty(), "no calls recorded");
    assert_eq!(log[0].0, coord_canon, "first call should be coord.work; got {:?}", log[0]);
    assert_eq!(log[0].1, LlmPhase::Work, "first call should be Work phase");
    assert!(
        log.iter().any(|(p, _)| *p == child_canon),
        "child should have been invoked; call log: {log:?}"
    );

    // After pop, stack.yaml must have been deleted.
    assert!(
        !coord.join("stack.yaml").exists(),
        "stack.yaml should be deleted after pop to depth 1"
    );
}

/// The `state` subcommand is the shell boundary LLM phase prompts call
/// into. A unit-level test proves the handlers work in-process, but
/// the argv shape, exit code, and on-disk effect via a real subprocess
/// are what the prompts actually depend on — so pin them end-to-end.
#[test]
fn state_set_phase_and_push_plan_via_binary() {
    let tmp = TempDir::new().unwrap();
    let coord = tmp.path().join("coord");
    let child = tmp.path().join("child");
    fs::create_dir_all(&coord).unwrap();
    fs::create_dir_all(&child).unwrap();
    fs::write(coord.join("phase.md"), "work").unwrap();
    fs::write(child.join("phase.md"), "work").unwrap();

    let bin = env!("CARGO_BIN_EXE_ravel-lite");

    let out = Command::new(bin)
        .args(["state", "set-phase"])
        .arg(&coord)
        .arg("analyse-work")
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "set-phase exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(coord.join("phase.md")).unwrap().trim(),
        "analyse-work"
    );

    let out = Command::new(bin)
        .args(["state", "push-plan"])
        .arg(&coord)
        .arg(&child)
        .args(["--reason", "kick the child"])
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "push-plan exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stack_yaml = fs::read_to_string(coord.join("stack.yaml")).unwrap();
    assert!(stack_yaml.contains("coord"), "root frame missing: {stack_yaml}");
    assert!(stack_yaml.contains("child"), "target frame missing: {stack_yaml}");
    assert!(
        stack_yaml.contains("kick the child"),
        "reason missing: {stack_yaml}"
    );
}

#[test]
fn state_set_phase_rejects_invalid_phase_via_binary() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "set-phase"])
        .arg(plan)
        .arg("analyze-work") // American spelling — invalid
        .output()
        .expect("binary must spawn");
    assert!(!out.status.success(), "invalid phase must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Invalid phase"), "stderr missing diagnostic: {stderr}");
    // On-disk phase.md unchanged.
    assert_eq!(fs::read_to_string(plan.join("phase.md")).unwrap().trim(), "work");
}
