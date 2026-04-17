use std::fs;
use tempfile::TempDir;

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    assert!(!raveloop_cli::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(100)).unwrap();
    raveloop_cli::dream::update_dream_baseline(plan);

    fs::write(plan.join("memory.md"), "word ".repeat(200)).unwrap();
    assert!(!raveloop_cli::dream::should_dream(plan, 1500));

    fs::write(plan.join("memory.md"), "word ".repeat(2000)).unwrap();
    assert!(raveloop_cli::dream::should_dream(plan, 1500));

    raveloop_cli::dream::update_dream_baseline(plan);
    assert!(!raveloop_cli::dream::should_dream(plan, 1500));
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

    let shared = raveloop_cli::config::load_shared_config(config_root).unwrap();
    assert_eq!(shared.agent, "claude-code");
    assert_eq!(shared.headroom, 1500);

    let agent = raveloop_cli::config::load_agent_config(config_root, "claude-code").unwrap();
    assert_eq!(agent.models.get("work").unwrap(), "claude-sonnet-4-6");
    assert!(agent.params.get("work").unwrap().get("dangerous").is_some());

    let tokens = raveloop_cli::config::load_tokens(config_root, "claude-code").unwrap();
    assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
}
