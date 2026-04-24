//! End-to-end coverage for the real `PiAgent` impl. The earlier mocks
//! substitute for the `Agent` trait; nothing here exercised the concrete
//! pi spawn/stream/dispatch path. That gap is how the `{{MEMORY_DIR}}`
//! regression — pi prompts loaded via ad-hoc `str::replace`, bypassing
//! `substitute_tokens` — escaped into a real session without a single
//! failing test.
//!
//! These tests run a fake `pi` binary on PATH (`pi` is a small shell
//! script the test writes into a tempdir). Three behaviours are pinned:
//! the phase-cycle entry point (token substitution + stream-event
//! fan-out + audit commit), the stderr-tail surfacing on non-zero exit,
//! and the dispatch-subagent argv contract.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use tempfile::TempDir;

use ravel_lite::agent::pi::PiAgent;
use ravel_lite::agent::Agent;
use ravel_lite::phase_loop::phase_loop;
use ravel_lite::types::{AgentConfig, LlmPhase, PlanContext, SharedConfig};
use ravel_lite::ui::{UIMessage, UI};

mod common;
use common::init_test_repo;

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
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

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
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

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
