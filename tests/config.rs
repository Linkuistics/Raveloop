use std::fs;

use tempfile::TempDir;

#[test]
fn config_loading_integration() {
    let dir = TempDir::new().unwrap();
    let config_root = dir.path();

    fs::write(
        config_root.join("config.yaml"),
        "agent: claude-code\nheadroom: 1500\n",
    )
    .unwrap();
    fs::create_dir_all(config_root.join("agents/claude-code")).unwrap();
    fs::write(
        config_root.join("agents/claude-code/config.yaml"),
        "models:\n  work: claude-sonnet-4-6\n  reflect: claude-haiku-4-5\nparams:\n  work:\n    dangerous: true\n",
    ).unwrap();
    fs::write(
        config_root.join("agents/claude-code/tokens.yaml"),
        "TOOL_READ: Read\n",
    )
    .unwrap();

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
            let model = cfg
                .models
                .get(phase)
                .unwrap_or_else(|| panic!("{agent_name} defaults missing model for phase {phase}"));
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
    let pi_provider = pi
        .provider
        .as_ref()
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
    assert!(
        survey.exists(),
        "missing survey prompt: {}",
        survey.display()
    );
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
    let loaded_incremental = ravel_lite::survey::load_survey_incremental_prompt(&target).unwrap();
    assert_eq!(loaded_incremental, incremental_body);

    let create_plan = target.join("create-plan.md");
    assert!(
        create_plan.exists(),
        "missing create-plan prompt: {}",
        create_plan.display()
    );
    let create_body = fs::read_to_string(&create_plan).unwrap();
    assert!(!create_body.trim().is_empty(), "empty create-plan prompt");
}
