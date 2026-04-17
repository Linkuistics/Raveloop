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
