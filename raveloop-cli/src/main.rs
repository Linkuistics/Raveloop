mod agent;
mod config;
mod dream;
mod format;
mod git;
mod init;
mod phase_loop;
mod prompt;
mod subagent;
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
use crate::config::{load_agent_config, load_shared_config};
use crate::git::find_project_root;
use crate::types::PlanContext;
use crate::ui::{run_tui, UI};

#[derive(Parser)]
#[command(name = "raveloop-cli", about = "An orchestration loop for LLM development cycles")]
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
    },
    /// Run the phase loop on a plan directory
    Run {
        /// Path to the config directory
        #[arg(long)]
        config: PathBuf,
        /// Path to the plan directory
        plan_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir } => {
            init::run_init(&dir)
        }
        Commands::Run { config, plan_dir } => {
            run_phase_loop(&config, &plan_dir).await
        }
    }
}

async fn run_phase_loop(config_root: &Path, plan_dir: &Path) -> Result<()> {
    if !plan_dir.join("phase.md").exists() {
        anyhow::bail!(
            "{}/phase.md not found. Is this a valid plan directory?",
            plan_dir.display()
        );
    }

    let shared_config = load_shared_config(config_root)?;
    let agent_config = load_agent_config(config_root, &shared_config.agent)?;
    let project_dir = find_project_root(plan_dir)?;

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

    let result = phase_loop::phase_loop(agent, &ctx, &shared_config, &ui).await;

    ui.quit();
    tui_handle.await??;

    result
}
