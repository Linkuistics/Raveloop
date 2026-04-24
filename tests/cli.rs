use std::fs;
use std::io::Cursor;
use std::process::Command;

use tempfile::TempDir;

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
    assert!(
        stderr.contains("Invalid phase"),
        "stderr missing diagnostic: {stderr}"
    );
    // On-disk phase.md unchanged.
    assert_eq!(
        fs::read_to_string(plan.join("phase.md")).unwrap().trim(),
        "work"
    );
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
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_a = project.join("LLM_STATE").join("plan-a");
    let plan_b = project.join("LLM_STATE").join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_b.join("phase.md"), "triage").unwrap();

    let map =
        ravel_lite::multi_plan::build_plan_dir_map(&[plan_a.clone(), plan_b.clone()]).unwrap();
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
    assert_eq!(
        response, reparsed,
        "round-trip through --survey-state must preserve the response"
    );

    // User picks the top-ranked plan (#1 = Proj/plan-b).
    let mut output = Vec::new();
    let mut input = Cursor::new("1\n");
    let picked =
        ravel_lite::multi_plan::select_plan_from_response(&reparsed, &map, &mut output, &mut input)
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
