mod agent;
mod config;
mod create;
mod dream;
mod format;
mod git;
mod init;
mod multi_plan;
mod phase_loop;
mod prompt;
mod state;
mod subagent;
mod survey;
mod term_title;
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
use crate::git::project_root_for_plan;
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

/// Version string baked in at compile time by `build.rs`. Shape:
/// `0.1.0 (v0.1.0-2-g15c2c8c-dirty, built 2026-04-21T06:42:18Z)`.
/// When no tag or no git data is available, the describe slot falls
/// back to the short SHA or literal `unknown`; the timestamp slot
/// falls back to `unknown` only if `date` is unavailable on the
/// build host.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_DESCRIBE"),
    ", built ",
    env!("BUILD_TIMESTAMP"),
    ")"
);

#[derive(Parser)]
#[command(
    name = "ravel-lite",
    about = "An orchestration loop for LLM development cycles",
    version = VERSION,
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
    /// Run the phase loop on one or more plan directories. With a
    /// single plan directory, behaviour is unchanged: the loop runs
    /// continuously, prompting between cycles. With two or more plan
    /// directories, multi-plan mode kicks in: every cycle starts with
    /// a survey across all plans, the user picks one from a numbered
    /// stdout prompt, and one phase cycle runs for the chosen plan.
    /// `--survey-state` is required for multi-plan and rejected for
    /// single-plan; it is read as `--prior` and rewritten at the end
    /// of every survey, so the file is the persistent integration
    /// point with the incremental survey path from item 5b.
    Run {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Skip Claude Code permission prompts for every phase (claude-code only).
        #[arg(long)]
        dangerous: bool,
        /// Path to the survey state file used by multi-plan mode. The
        /// file is both the incremental-survey `--prior` input and the
        /// canonical YAML output written at the end of every survey.
        /// Required when more than one plan directory is supplied;
        /// rejected when exactly one is supplied.
        #[arg(long)]
        survey_state: Option<PathBuf>,
        /// One or more plan directories. With a single directory the
        /// behaviour is the original single-plan run loop. With two or
        /// more, multi-plan mode dispatches one cycle per
        /// survey-driven user selection.
        #[arg(required = true, num_args = 1..)]
        plan_dirs: Vec<PathBuf>,
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
    /// Produce an LLM-driven plan status overview for one or more plan
    /// directories. Reads each plan's phase/backlog/memory into a single
    /// fresh-context claude session that returns a per-plan summary and
    /// a recommended invocation order, emitted as canonical YAML on
    /// stdout. Use `ravel-lite survey-format <file>` to render a saved
    /// YAML survey as human-readable markdown.
    Survey {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Plan directories (each containing phase.md). Replaces the
        /// former plan-root walk: callers now name plans individually.
        /// At least one required.
        #[arg(required = true, num_args = 1..)]
        plan_dirs: Vec<PathBuf>,
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
        /// Path to a prior survey YAML to use as the baseline for an
        /// incremental run. Plans whose `input_hash` matches the prior
        /// are carried forward verbatim; only changed and added plans
        /// are sent to the LLM. Rejected schemas and unrecognised
        /// versions produce a loud error with a remediation hint
        /// pointing at `--force`.
        #[arg(long)]
        prior: Option<PathBuf>,
        /// Re-analyse every plan regardless of whether its hash matches
        /// the prior. Has no effect without `--prior`. Intended for
        /// debugging and schema-bump remediation.
        #[arg(long)]
        force: bool,
    },
    /// Render a saved YAML survey file (as produced by `ravel-lite
    /// survey`) as human-readable markdown on stdout. Read-only; no
    /// network, no LLM call.
    SurveyFormat {
        /// Path to a YAML survey file to render.
        file: PathBuf,
    },
    /// Print the installed ravel-lite version. Equivalent to `--version`;
    /// the subcommand form matches the rest of the CLI surface.
    Version,
    /// Mutate plan state from prompts without the Read+Write tool-call
    /// overhead (and permission prompts) of writing files directly.
    /// Expose via a single `Bash(ravel-lite state *)` allowlist entry.
    State {
        #[command(subcommand)]
        command: StateCommands,
    },
}

#[derive(Subcommand)]
enum StateCommands {
    /// Rewrite `<plan-dir>/phase.md` to the given phase. Validates the
    /// phase string and requires phase.md to already exist.
    SetPhase {
        /// Path to the plan directory whose phase.md to rewrite.
        plan_dir: PathBuf,
        /// Phase name to write (e.g. `analyse-work`, `git-commit-work`).
        phase: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir, force } => {
            init::run_init(&dir, force)
        }
        Commands::Run { config, dangerous, survey_state, plan_dirs } => {
            let config_root = resolve_config_dir(config)?;
            match plan_dirs.len() {
                0 => unreachable!("clap requires at least one plan_dir"),
                1 => {
                    if survey_state.is_some() {
                        anyhow::bail!(
                            "--survey-state is only meaningful with multiple plan \
                             directories; remove it or pass two or more plan_dirs."
                        );
                    }
                    run_phase_loop(&config_root, &plan_dirs[0], dangerous).await
                }
                _ => {
                    let state_path = survey_state.ok_or_else(|| {
                        anyhow::anyhow!(
                            "--survey-state <path> is required when more than one \
                             plan directory is supplied. The file holds the survey \
                             YAML between cycles and is read as `--prior` on each \
                             subsequent survey."
                        )
                    })?;
                    multi_plan::run_multi_plan(
                        &config_root,
                        &plan_dirs,
                        &state_path,
                        dangerous,
                    )
                    .await
                }
            }
        }
        Commands::Create { config, plan_dir } => {
            let config_root = resolve_config_dir(config)?;
            create::run_create(&config_root, plan_dir).await
        }
        Commands::Survey { config, plan_dirs, model, timeout_secs, prior, force } => {
            let config_root = resolve_config_dir(config)?;
            survey::run_survey(
                &config_root,
                &plan_dirs,
                model,
                timeout_secs,
                prior.as_deref(),
                force,
            )
            .await
        }
        Commands::SurveyFormat { file } => {
            survey::run_survey_format(&file)
        }
        Commands::Version => {
            println!("ravel-lite {VERSION}");
            Ok(())
        }
        Commands::State { command } => match command {
            StateCommands::SetPhase { plan_dir, phase } => {
                state::run_set_phase(&plan_dir, &phase)
            }
        },
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
    let project_dir = project_root_for_plan(plan_dir)?;

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

    let result = phase_loop::run_single_plan(agent, ctx, &shared_config, &ui).await;

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
