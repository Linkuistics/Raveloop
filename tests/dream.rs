use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;

use ravel_lite::phase_loop::phase_loop;
use ravel_lite::types::{LlmPhase, PlanContext, SharedConfig};
use ravel_lite::ui::{UIMessage, UI};

mod common;
use common::{init_test_repo, write_memory_yaml_with_word_count, MockAgent};

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
