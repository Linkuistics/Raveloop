use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::TempDir;

use ravel_lite::agent::Agent;
use ravel_lite::phase_loop::phase_loop;
use ravel_lite::state::backlog::schema::{BacklogFile, Status, Task};
use ravel_lite::state::backlog::write_backlog;
use ravel_lite::state::memory::schema::{MemoryEntry, MemoryFile};
use ravel_lite::state::memory::write_memory;
use ravel_lite::types::{LlmPhase, PlanContext, SharedConfig};
use ravel_lite::ui::{UI, UIMessage, UISender};

/// Seed `memory.yaml` so that `dream`'s word counter sees exactly
/// `target_words` words of content (one entry, empty title, body of
/// that many tokens). Lets tests focus on threshold behaviour without
/// wiring up the whole memory schema by hand.
fn write_memory_yaml_with_word_count(plan: &Path, target_words: usize) {
    let body = if target_words == 0 {
        String::new()
    } else {
        vec!["word"; target_words].join(" ")
    };
    let memory = MemoryFile {
        entries: vec![MemoryEntry {
            id: "test-entry".into(),
            title: String::new(),
            body,
        }],
        extra: Default::default(),
    };
    write_memory(plan, &memory).unwrap();
}

/// Seed `backlog.yaml` with a single task whose title embeds `marker`.
/// The marker surfaces in the serialised YAML so tests can assert on
/// rendered survey output.
fn write_backlog_yaml_with_marker(plan: &Path, marker: &str) {
    let backlog = BacklogFile {
        tasks: vec![Task {
            id: "marker-task".into(),
            title: marker.into(),
            category: "maintenance".into(),
            status: Status::NotStarted,
            blocked_reason: None,
            dependencies: vec![],
            description: "Marker body.\n".into(),
            results: None,
            handoff: None,
        }],
        extra: Default::default(),
    };
    write_backlog(plan, &backlog).unwrap();
}

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    write_memory_yaml_with_word_count(plan, 100);
    ravel_lite::dream::update_dream_baseline(plan);

    write_memory_yaml_with_word_count(plan, 200);
    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    write_memory_yaml_with_word_count(plan, 2000);
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
    write_memory_yaml_with_word_count(&plan_dir, 300);
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

    let survey_incremental = target.join("survey-incremental.md");
    assert!(
        survey_incremental.exists(),
        "missing incremental survey prompt: {}",
        survey_incremental.display()
    );
    let incremental_body = fs::read_to_string(&survey_incremental).unwrap();
    assert!(
        !incremental_body.trim().is_empty(),
        "empty incremental survey prompt"
    );
    let loaded_incremental =
        ravel_lite::survey::load_survey_incremental_prompt(&target).unwrap();
    assert_eq!(loaded_incremental, incremental_body);

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
        write_backlog_yaml_with_marker(plan_dir, &format!("backlog-marker-{plan_name}"));
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
    assert!(
        rendered.contains("backlog-marker-plan-alpha"),
        "alpha's backlog marker must surface in the rendered survey: {rendered}"
    );
    assert!(rendered.contains("### memory.yaml\n(missing)"));
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
    let plan_a = project.join("LLM_STATE").join("plan-a");
    let plan_b = project.join("LLM_STATE").join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    write_backlog_yaml_with_marker(&plan_a, "plan-a-backlog");
    fs::write(plan_b.join("phase.md"), "triage").unwrap();
    write_backlog_yaml_with_marker(&plan_b, "plan-b-backlog");

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
fn survey_incremental_merge_reuses_unchanged_rows_and_includes_llm_delta() {
    // End-to-end merge: prior has two plans; one of them is mutated
    // on disk, the other is unchanged. Classification should mark
    // the untouched plan as unchanged (and its prior row reused),
    // the mutated plan as changed (and the LLM's new row merged in).
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_stable = project.join("LLM_STATE").join("stable");
    let plan_mutated = project.join("LLM_STATE").join("mutated");
    fs::create_dir_all(&plan_stable).unwrap();
    fs::create_dir_all(&plan_mutated).unwrap();
    fs::write(plan_stable.join("phase.md"), "work").unwrap();
    write_backlog_yaml_with_marker(&plan_stable, "original-stable-backlog");
    fs::write(plan_mutated.join("phase.md"), "work").unwrap();
    write_backlog_yaml_with_marker(&plan_mutated, "original-mutated-backlog");

    let snap_stable_before = ravel_lite::survey::load_plan(&plan_stable).unwrap();
    let snap_mutated_before = ravel_lite::survey::load_plan(&plan_mutated).unwrap();

    // Construct a prior as it would appear on disk: cold-path LLM
    // output + hash injection.
    let llm_cold = "plans:\n  \
         - project: Proj\n    plan: stable\n    phase: work\n    unblocked: 1\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n  \
         - project: Proj\n    plan: mutated\n    phase: work\n    unblocked: 2\n    blocked: 0\n    done: 0\n    received: 0\n    notes: pre-mutation\n"
        .to_string();
    let mut prior = ravel_lite::survey::parse_survey_response(&llm_cold).unwrap();
    let prior_hashes: std::collections::HashMap<String, String> = [
        (ravel_lite::survey::plan_key("Proj", "stable"), snap_stable_before.input_hash.clone()),
        (ravel_lite::survey::plan_key("Proj", "mutated"), snap_mutated_before.input_hash.clone()),
    ]
    .into_iter()
    .collect();
    ravel_lite::survey::inject_input_hashes(&mut prior, &prior_hashes).unwrap();

    // Mutate the second plan's backlog on disk, re-snapshot.
    write_backlog_yaml_with_marker(&plan_mutated, "NEW-mutated-backlog");
    let snap_stable_after = ravel_lite::survey::load_plan(&plan_stable).unwrap();
    let snap_mutated_after = ravel_lite::survey::load_plan(&plan_mutated).unwrap();
    assert_eq!(
        snap_stable_after.input_hash, snap_stable_before.input_hash,
        "stable plan's hash should not have changed"
    );
    assert_ne!(
        snap_mutated_after.input_hash, snap_mutated_before.input_hash,
        "mutated plan's hash must differ"
    );

    let current = vec![snap_stable_after, snap_mutated_after];
    let classification = ravel_lite::survey::PlanClassification::classify(&prior, &current);
    assert_eq!(classification.unchanged_rows.len(), 1);
    assert_eq!(classification.unchanged_rows[0].plan, "stable");
    assert_eq!(classification.changed.len(), 1);
    assert_eq!(classification.changed[0].plan, "mutated");
    assert!(classification.added.is_empty());
    assert!(classification.removed_keys.is_empty());
    assert!(!classification.is_noop());

    // Simulated LLM delta response: only the changed plan, with a
    // refreshed note.
    let llm_delta_yaml =
        "plans:\n  - project: Proj\n    plan: mutated\n    phase: work\n    unblocked: 2\n    \
         blocked: 0\n    done: 1\n    received: 0\n    notes: post-mutation\n";
    let mut delta = ravel_lite::survey::parse_survey_response(llm_delta_yaml).unwrap();
    let delta_hashes: std::collections::HashMap<String, String> = [(
        ravel_lite::survey::plan_key("Proj", "mutated"),
        current[1].input_hash.clone(),
    )]
    .into_iter()
    .collect();
    ravel_lite::survey::inject_input_hashes(&mut delta, &delta_hashes).unwrap();

    let merged = ravel_lite::survey::merge_delta(classification, delta).unwrap();
    assert_eq!(merged.plans.len(), 2, "merged must contain both plans");
    let stable_row = merged.plans.iter().find(|p| p.plan == "stable").unwrap();
    let mutated_row = merged.plans.iter().find(|p| p.plan == "mutated").unwrap();
    assert_eq!(
        stable_row.input_hash, snap_stable_before.input_hash,
        "unchanged row's hash carries forward from prior"
    );
    assert_eq!(
        mutated_row.input_hash, current[1].input_hash,
        "changed row's hash is the freshly-computed one"
    );
    assert_eq!(mutated_row.done, 1, "changed row reflects LLM delta values");
    assert_eq!(mutated_row.notes, "post-mutation");
}

#[test]
fn survey_incremental_is_noop_when_no_files_changed() {
    // Scenario: plan directory was never touched since the prior
    // survey was produced. Classification should flag the whole set
    // as noop, letting the runner carry prior forward with no LLM call.
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_dir = project.join("LLM_STATE").join("unchanged");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("phase.md"), "work").unwrap();
    write_backlog_yaml_with_marker(&plan_dir, "original");

    let snap = ravel_lite::survey::load_plan(&plan_dir).unwrap();
    let llm_yaml = "plans:\n  - project: Proj\n    plan: unchanged\n    phase: work\n    \
                    unblocked: 1\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n";
    let mut prior = ravel_lite::survey::parse_survey_response(llm_yaml).unwrap();
    ravel_lite::survey::inject_input_hashes(
        &mut prior,
        &[(
            ravel_lite::survey::plan_key("Proj", "unchanged"),
            snap.input_hash.clone(),
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();

    let re_snap = ravel_lite::survey::load_plan(&plan_dir).unwrap();
    let classification =
        ravel_lite::survey::PlanClassification::classify(&prior, std::slice::from_ref(&re_snap));
    assert!(classification.is_noop());
    assert_eq!(classification.unchanged_rows.len(), 1);
}

#[test]
fn survey_incremental_rejects_llm_delta_outside_changed_set() {
    // Validation mirror to inject_input_hashes: the LLM cannot
    // smuggle a row for a plan it wasn't asked about.
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_a = project.join("LLM_STATE").join("a");
    fs::create_dir_all(&plan_a).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    write_backlog_yaml_with_marker(&plan_a, "original");

    let snap_before = ravel_lite::survey::load_plan(&plan_a).unwrap();
    let prior_yaml = "plans:\n  - project: Proj\n    plan: a\n    phase: work\n    \
                      unblocked: 1\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n";
    let mut prior = ravel_lite::survey::parse_survey_response(prior_yaml).unwrap();
    ravel_lite::survey::inject_input_hashes(
        &mut prior,
        &[(
            ravel_lite::survey::plan_key("Proj", "a"),
            snap_before.input_hash.clone(),
        )]
        .into_iter()
        .collect(),
    )
    .unwrap();

    write_backlog_yaml_with_marker(&plan_a, "changed");
    let snap_after = ravel_lite::survey::load_plan(&plan_a).unwrap();
    let classification =
        ravel_lite::survey::PlanClassification::classify(&prior, std::slice::from_ref(&snap_after));

    // LLM response returns the valid row AND a hallucinated extra.
    let bad_delta = "plans:\n  \
                     - project: Proj\n    plan: a\n    phase: work\n    unblocked: 1\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n  \
                     - project: Proj\n    plan: hallucinated\n    phase: work\n    unblocked: 9\n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n";
    let delta = ravel_lite::survey::parse_survey_response(bad_delta).unwrap();
    let err = ravel_lite::survey::merge_delta(classification, delta).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("outside"), "expected validation error; got: {msg}");
    assert!(msg.contains("hallucinated"), "got: {msg}");
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
            let matches = block
                .lines()
                .any(|line| line.trim_end() == target_heading);
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
    let handoff_body =
        "Problem: three callers duplicate the block-split parse. \
         Decision: introduce `parse_backlog_blocks()` in `src/backlog.rs`. \
         References: tests/integration.rs:570 (flip_stale_task_statuses).".to_string();

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
    let handoff_body =
        "Not concrete enough for a backlog task. Keep as memory: the \
         markdown-first approach has friction points we should revisit \
         once three more plans are in flight.".to_string();

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

/// 5c CLI validation: with two or more plan_dirs, `--survey-state` is
/// required. The pre-flight check must fire BEFORE the binary tries to
/// load configs or spawn agents — proving the validation lives in the
/// CLI dispatch layer where multi-plan vs single-plan branches.
#[test]
fn run_multi_plan_requires_survey_state_flag() {
    let tmp = TempDir::new().unwrap();
    let plan_a = tmp.path().join("plan-a");
    let plan_b = tmp.path().join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_b.join("phase.md"), "work").unwrap();

    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["run", "--config"])
        .arg(&config_root)
        .arg(&plan_a)
        .arg(&plan_b)
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "multi-plan run without --survey-state must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--survey-state"),
        "stderr should name the missing flag: {stderr}"
    );
    assert!(
        stderr.contains("required"),
        "stderr should explain why: {stderr}"
    );
}

/// 5c CLI validation: with exactly one plan_dir, `--survey-state` has
/// no meaning and is rejected. Catches accidental misuse where a user
/// adds the flag to a single-plan invocation expecting it to be
/// ignored — silently ignoring would mask their mistake.
#[test]
fn run_single_plan_rejects_survey_state_flag() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path().join("solo");
    fs::create_dir_all(&plan).unwrap();
    fs::write(plan.join("phase.md"), "work").unwrap();

    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let state_path = tmp.path().join("survey.yaml");

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["run", "--config"])
        .arg(&config_root)
        .arg("--survey-state")
        .arg(&state_path)
        .arg(&plan)
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "single-plan run with --survey-state must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--survey-state"),
        "stderr should name the offending flag: {stderr}"
    );
    assert!(
        stderr.contains("multiple plan"),
        "stderr should explain when the flag is meaningful: {stderr}"
    );
    // The state file must not have been written — the validation
    // fires before any survey work happens.
    assert!(
        !state_path.exists(),
        "validation must short-circuit before touching the state file"
    );
}

/// 5c integration: the multi-plan run loop relies on round-tripping
/// the survey YAML through `--survey-state`. This test bypasses the
/// claude spawn (which we can't mock cheaply) and verifies the pieces
/// of the loop that DO live in Rust:
///   - `build_plan_dir_map` correctly indexes discovered plans by the
///     same `project/plan` key the survey rows carry.
///   - `select_plan_from_response` resolves a recommendation key back
///     to the right plan directory.
///   - The persisted YAML round-trips (parse → emit → parse) via the
///     same path the next cycle's incremental survey will follow.
#[test]
fn multi_plan_round_trip_preserves_selection_mapping() {
    use std::io::Cursor;

    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_a = project.join("LLM_STATE").join("plan-a");
    let plan_b = project.join("LLM_STATE").join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_b.join("phase.md"), "triage").unwrap();

    let map = ravel_lite::multi_plan::build_plan_dir_map(&[plan_a.clone(), plan_b.clone()])
        .unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("Proj/plan-a"), Some(&plan_a));
    assert_eq!(map.get("Proj/plan-b"), Some(&plan_b));

    // Simulate what compute_survey_response returns + persists.
    let response_yaml = "schema_version: 1\n\
        plans:\n  \
        - project: Proj\n    plan: plan-a\n    phase: work\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n  \
        - project: Proj\n    plan: plan-b\n    phase: triage\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n\
        recommended_invocation_order:\n  \
        - plan: Proj/plan-b\n    order: 1\n    rationale: Triage first to unblock A\n  \
        - plan: Proj/plan-a\n    order: 2\n    rationale: Then resume work\n";

    // First cycle: parse, emit (what run_multi_plan writes to --survey-state),
    // then parse again (what the next cycle's --prior load would do).
    let response = ravel_lite::survey::parse_survey_response(response_yaml).unwrap();
    let emitted = ravel_lite::survey::emit_survey_yaml(&response).unwrap();
    let reparsed = ravel_lite::survey::parse_survey_response(&emitted).unwrap();
    assert_eq!(response, reparsed, "round-trip through --survey-state must preserve the response");

    // User picks the top-ranked plan (#1 = Proj/plan-b).
    let mut output = Vec::new();
    let mut input = Cursor::new("1\n");
    let picked = ravel_lite::multi_plan::select_plan_from_response(
        &reparsed,
        &map,
        &mut output,
        &mut input,
    )
    .unwrap();
    assert_eq!(
        picked,
        Some(plan_b.clone()),
        "ordinal 1 (top-ranked Proj/plan-b) must resolve back to plan_b's PathBuf"
    );

    // User picks the second-ranked plan (#2 = Proj/plan-a).
    let mut output2 = Vec::new();
    let mut input2 = Cursor::new("2\n");
    let picked2 = ravel_lite::multi_plan::select_plan_from_response(
        &reparsed,
        &map,
        &mut output2,
        &mut input2,
    )
    .unwrap();
    assert_eq!(picked2, Some(plan_a));
}

/// End-to-end through the CLI binary: `state projects add` persists to
/// `<config>/projects.yaml`, `list` round-trips, `rename` mutates in
/// place, and `remove` deletes. Guards the CLI dispatch layer wiring
/// (clap subcommand enum → projects module handlers).
#[test]
fn state_projects_add_list_rename_remove_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let project = tmp.path().join("some-project");
    fs::create_dir_all(&project).unwrap();

    // add
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "some-project", "--path"])
        .arg(&project)
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(cfg.join("projects.yaml").exists(), "projects.yaml should exist after add");

    // list (stdout is YAML)
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "list", "--config"])
        .arg(&cfg)
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "list failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("some-project"), "list should mention the project: {stdout}");
    assert!(stdout.contains("schema_version"), "list should emit schema_version: {stdout}");

    // rename
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "rename", "--config"])
        .arg(&cfg)
        .args(["some-project", "renamed-project"])
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "rename failed: {}", String::from_utf8_lossy(&out.stderr));
    // Parse the YAML because the path still contains "some-project" as
    // its basename; only the `name:` field should have changed.
    let after_rename: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let names: Vec<&str> = after_rename["projects"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["renamed-project"], "only the name should have changed");

    // remove
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "remove", "--config"])
        .arg(&cfg)
        .arg("renamed-project")
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "remove failed: {}", String::from_utf8_lossy(&out.stderr));
    let after_remove: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let remaining = after_remove["projects"].as_sequence().unwrap();
    assert!(remaining.is_empty(), "projects list should be empty after remove: {remaining:?}");
}

/// `state projects add` accepts a relative path and stores it as
/// absolute, resolved against the child process's CWD. Pins the
/// canonicalisation at the user-facing CLI, not just the internal
/// helper. `Command::current_dir` scopes the CWD change to the child
/// so this test is safe under parallel execution.
#[test]
fn state_projects_add_canonicalises_relative_path_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    // Target project directory must exist relative to the spawn CWD.
    let workdir = tmp.path().join("workdir");
    fs::create_dir_all(workdir.join("rel-target")).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .current_dir(&workdir)
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "rel", "--path", "rel-target"])
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));

    let catalog: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let stored_path = catalog["projects"][0]["path"].as_str().unwrap();
    assert!(
        stored_path.starts_with('/'),
        "stored path must be absolute, got {stored_path}"
    );
    assert!(
        stored_path.ends_with("workdir/rel-target"),
        "stored path must reflect CWD resolution, got {stored_path}"
    );
}

/// `state projects add` accepts `--path` with no `--name`, defaulting
/// the name to the resolved path's basename.
#[test]
fn state_projects_add_defaults_name_to_basename_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let project = tmp.path().join("derived-name");
    fs::create_dir_all(&project).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .arg("--path")
        .arg(&project)
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));

    let catalog: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    assert_eq!(catalog["projects"][0]["name"].as_str().unwrap(), "derived-name");
}
