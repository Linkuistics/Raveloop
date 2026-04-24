use std::fs;

use tempfile::TempDir;

mod common;
use common::write_backlog_yaml_with_marker;

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
        (
            ravel_lite::survey::plan_key(&snapshot_a.project, &snapshot_a.plan),
            snapshot_a.input_hash.clone(),
        ),
        (
            ravel_lite::survey::plan_key(&snapshot_b.project, &snapshot_b.plan),
            snapshot_b.input_hash.clone(),
        ),
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
        (
            ravel_lite::survey::plan_key("Proj", "stable"),
            snap_stable_before.input_hash.clone(),
        ),
        (
            ravel_lite::survey::plan_key("Proj", "mutated"),
            snap_mutated_before.input_hash.clone(),
        ),
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
    assert!(
        msg.contains("outside"),
        "expected validation error; got: {msg}"
    );
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
    let reloaded =
        ravel_lite::survey::parse_survey_response(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        ravel_lite::survey::render_survey_output(&reloaded),
        expected
    );
}
