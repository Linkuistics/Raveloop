mod agent;
mod config;
mod create;
mod dream;
mod format;
mod git;
mod init;
mod phase_loop;
mod pivot;
mod prompt;
mod subagent;
mod survey;
mod types;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use crate::agent::claude_code::ClaudeCodeAgent;
use crate::agent::pi::PiAgent;
use crate::agent::Agent;
use crate::config::{load_agent_config, load_shared_config, resolve_config_dir};
use crate::git::find_project_root;
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{run_tui, UI};

/// Force `dangerous: true` for every known LLM phase, overriding
/// whatever was loaded from the config file.
fn force_dangerous(config: &mut AgentConfig) {
    let phases = [
        LlmPhase::Work,
        LlmPhase::AnalyseWork,
        LlmPhase::Reflect,
        LlmPhase::Dream,
        LlmPhase::Triage,
    ];
    for phase in phases {
        let params = config.params.entry(phase.as_str().to_string()).or_default();
        params.insert("dangerous".to_string(), serde_yaml::Value::Bool(true));
    }
}

#[derive(Parser)]
#[command(
    name = "ravel-lite",
    about = "An orchestration loop for LLM development cycles",
    version,
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a config directory with default prompts and config
    Init {
        /// Target directory to create
        dir: PathBuf,
        /// Overwrite existing files (refresh prompts after upgrading)
        #[arg(long)]
        force: bool,
    },
    /// Run the phase loop on a plan directory
    Run {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Skip Claude Code permission prompts for every phase (claude-code only).
        #[arg(long)]
        dangerous: bool,
        /// Path to the plan directory
        plan_dir: PathBuf,
    },
    /// Create a new plan directory via an interactive headful claude
    /// session. Loads the create-plan prompt template from
    /// <config-dir>/create-plan.md, appends the target path, and
    /// hands off to claude with inherited stdio so the user drives
    /// the conversation directly. Reuses the configured work-phase
    /// model; passes `--add-dir <parent>` to scope claude to the
    /// target parent directory.
    Create {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to the new plan directory. Must not already exist; its
        /// parent directory must exist.
        plan_dir: PathBuf,
    },
    /// Produce an LLM-driven plan status overview across one or more
    /// plan-root directories. Reads every plan's phase/backlog/memory
    /// into a single fresh-context claude session that returns a
    /// per-plan summary and a recommended invocation order.
    Survey {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Plan root directories. Each root contributes all plans under
        /// it (directories containing phase.md). At least one required.
        #[arg(required = true, num_args = 1..)]
        roots: Vec<PathBuf>,
        /// Override the model used for the survey call. Overrides
        /// `models.survey` in agents/claude-code/config.yaml, which in
        /// turn overrides the DEFAULT_SURVEY_MODEL constant.
        #[arg(long)]
        model: Option<String>,
        /// Override the timeout (in seconds) for the `claude` subprocess
        /// call. Default is 300 seconds (5 minutes). The survey fails
        /// with a diagnostic error and a partial-stdout dump if claude
        /// does not produce a result within this window.
        #[arg(long)]
        timeout_secs: Option<u64>,
    },
    /// Print the installed ravel-lite version. Equivalent to `--version`;
    /// the subcommand form matches the rest of the CLI surface.
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir, force } => {
            init::run_init(&dir, force)
        }
        Commands::Run { config, dangerous, plan_dir } => {
            let config_root = resolve_config_dir(config)?;
            run_phase_loop(&config_root, &plan_dir, dangerous).await
        }
        Commands::Create { config, plan_dir } => {
            let config_root = resolve_config_dir(config)?;
            create::run_create(&config_root, plan_dir).await
        }
        Commands::Survey { config, roots, model, timeout_secs } => {
            let config_root = resolve_config_dir(config)?;
            survey::run_survey(&config_root, &roots, model, timeout_secs).await
        }
        Commands::Version => {
            println!("ravel-lite {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn run_phase_loop(config_root: &Path, plan_dir: &Path, dangerous: bool) -> Result<()> {
    if !plan_dir.join("phase.md").exists() {
        anyhow::bail!(
            "{}/phase.md not found. Is this a valid plan directory?",
            plan_dir.display()
        );
    }

    let shared_config = load_shared_config(config_root)?;
    let mut agent_config = load_agent_config(config_root, &shared_config.agent)?;
    let project_dir = find_project_root(plan_dir)?;

    if dangerous {
        if shared_config.agent == "claude-code" {
            force_dangerous(&mut agent_config);
        } else {
            eprintln!(
                "warning: --dangerous has no effect for agent '{}' (claude-code only)",
                shared_config.agent
            );
        }
    }

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: project_dir.clone(),
        dev_root: Path::new(&project_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        related_plans: std::fs::read_to_string(plan_dir.join("related-plans.md"))
            .unwrap_or_default(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let agent: Arc<dyn Agent> = match shared_config.agent.as_str() {
        "claude-code" => Arc::new(ClaudeCodeAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        "pi" => Arc::new(PiAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        other => anyhow::bail!("Unknown agent: {other}"),
    };

    let (tx, rx) = mpsc::unbounded_channel();
    let ui = UI::new(tx);

    let tui_handle = tokio::spawn(run_tui(rx));

    let result = phase_loop::run_stack(agent, ctx, &shared_config, &ui).await;

    if let Err(ref e) = result {
        // Show the error inside the TUI first so the user sees it in
        // context, then wait for acknowledgement before tearing down.
        ui.log("");
        ui.log(&format!("  ✗  Fatal error: {e:#}"));
        let _ = ui.confirm("Exit ravel-lite?").await;
    }

    ui.quit();
    tui_handle.await??;

    // Also emit to stderr so the error is preserved in the terminal
    // scrollback after the alternate screen has been torn down.
    if let Err(ref e) = result {
        eprintln!("\nravel-lite exited with error:\n{e:#}");
    }

    result
}
