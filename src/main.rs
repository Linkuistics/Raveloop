use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use ravel_lite::agent::claude_code::ClaudeCodeAgent;
use ravel_lite::agent::pi::PiAgent;
use ravel_lite::agent::Agent;
use ravel_lite::config::{load_agent_config, load_shared_config, resolve_config_dir};
use ravel_lite::git::project_root_for_plan;
use ravel_lite::ontology::cli::{parse_edge_kind, parse_evidence_grade, parse_lifecycle_scope};
use ravel_lite::types::{AgentConfig, LlmPhase, PlanContext};
use ravel_lite::ui::{run_tui, UI};
use ravel_lite::{
    create, init, multi_plan, phase_loop, projects, related_components, state, survey,
};

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
    /// Manage the per-user projects catalog (`<config-dir>/projects.yaml`)
    /// that maps component names to absolute paths. The shared
    /// component-relationship graph (`related-components.yaml`)
    /// references components by name; this catalog is the per-user
    /// resolver. Auto-populated on `ravel-lite run` when a new project
    /// is encountered.
    Projects {
        #[command(subcommand)]
        command: ProjectsCommands,
    },
    /// Backlog CRUD verbs. Every prompt-side mutation of backlog.yaml
    /// goes through one of these.
    Backlog {
        #[command(subcommand)]
        command: BacklogCommands,
    },
    /// Memory CRUD verbs. Dream rewrites memory.yaml per-entry through
    /// these verbs rather than bulk-swapping the file.
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
    /// Session-log verbs. `latest-session.yaml` is a single-record file
    /// written by analyse-work; `session-log.yaml` is the append-only
    /// history. `phase_loop::GitCommitWork` appends latest → log
    /// programmatically between phases.
    SessionLog {
        #[command(subcommand)]
        command: SessionLogCommands,
    },
    /// Single-plan conversion of legacy .md files into typed .yaml
    /// siblings. Covers backlog.md, memory.md, session-log.md and
    /// latest-session.md (each written when present).
    Migrate {
        plan_dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
        /// Keep the .md originals on disk after migration (default).
        #[arg(long, conflicts_with = "delete_originals")]
        keep_originals: bool,
        /// Delete the .md originals only after write and validation both succeed.
        #[arg(long)]
        delete_originals: bool,
        /// Overwrite an existing backlog.yaml that differs from the re-migration output.
        #[arg(long)]
        force: bool,
    },
    /// Global component-relationship graph at
    /// `<config-dir>/related-components.yaml`. Edges follow the
    /// component-ontology v2 schema (see docs/component-ontology.md);
    /// participants reference components by name (resolved per-user via
    /// the projects catalog), so the file is shareable between users.
    RelatedComponents {
        #[command(subcommand)]
        command: RelatedComponentsCommands,
    },
    /// Stage 2 discovery emits each edge through `add-proposal` instead
    /// of writing `discover-proposals.yaml` directly. A hallucinated
    /// `--kind` is rejected by clap with the full valid vocabulary in
    /// the error message, so the LLM retries that single call rather
    /// than nuking the whole file.
    DiscoverProposals {
        #[command(subcommand)]
        command: DiscoverProposalsCommands,
    },
}

#[derive(Subcommand)]
enum BacklogCommands {
    /// Emit tasks matching the given filters.
    List {
        plan_dir: PathBuf,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        category: Option<String>,
        /// Shorthand for `status=not_started AND every dep is done`.
        #[arg(long)]
        ready: bool,
        /// Match tasks that carry a hand-off block.
        #[arg(long)]
        has_handoff: bool,
        /// Match done tasks missing a Results block.
        #[arg(long)]
        missing_results: bool,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single task by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new task.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        category: String,
        #[arg(long, value_delimiter = ',')]
        dependencies: Vec<String>,
        /// Path to a file containing the markdown description body.
        #[arg(long, conflicts_with = "description")]
        description_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the description inline.
        #[arg(long)]
        description: Option<String>,
    },
    /// One-shot bulk initialisation for create-plan. Refuses a non-empty backlog.
    Init {
        plan_dir: PathBuf,
        #[arg(long)]
        body_file: PathBuf,
    },
    /// Update a task's status. `--reason <text>` is required when setting to `blocked`.
    SetStatus {
        plan_dir: PathBuf,
        id: String,
        status: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Set a task's Results block from a file or stdin.
    SetResults {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Rewrite a task's Description (the brief authored at `add` time).
    ///
    /// Use when external references in the body — e.g. doc section
    /// anchors or file paths — have moved and the brief needs to catch
    /// up. For recording what a completed task produced, use
    /// `set-results` instead; for promote-vs-archive hand-offs use
    /// `set-handoff`.
    SetDescription {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Set a task's hand-off block from a file or stdin.
    SetHandoff {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Clear a task's hand-off block (triage uses after promote/archive).
    ClearHandoff {
        plan_dir: PathBuf,
        id: String,
    },
    /// Update a task's title. Id is preserved.
    SetTitle {
        plan_dir: PathBuf,
        id: String,
        new_title: String,
    },
    /// Replace a task's dependency list. Validates ids, rejects self-reference and cycles.
    SetDependencies {
        plan_dir: PathBuf,
        id: String,
        /// Comma-separated list of task ids. Pass `--deps ""` to clear all deps.
        #[arg(long, value_delimiter = ',')]
        deps: Vec<String>,
    },
    /// Move a task before or after another in the backlog list.
    Reorder {
        plan_dir: PathBuf,
        id: String,
        position: String,
        target_id: String,
    },
    /// Delete a task. Refuses if the task is a dependency of another unless `--force`.
    Delete {
        plan_dir: PathBuf,
        id: String,
        #[arg(long)]
        force: bool,
    },
    /// Report drift between prose task-id mentions in task descriptions
    /// and the structured `dependencies:` field. Read-only; reconciliation
    /// is still done via `set-dependencies`.
    LintDependencies {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Repair stale task statuses: flip `in_progress` tasks with
    /// non-empty `results` to `done`, and flip `blocked` tasks whose
    /// structural dependencies are all `done` back to `not_started`.
    /// Emits a report and (unless `--dry-run`) writes the repaired
    /// backlog. Exit code: 0 if no repairs applied, 1 if any repairs
    /// applied (scripting signal).
    RepairStaleStatuses {
        plan_dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// Emit every memory entry.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single memory entry by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new memory entry.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        title: String,
        /// Path to a file containing the markdown body.
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the body inline.
        #[arg(long)]
        body: Option<String>,
    },
    /// One-shot bulk initialisation for create-plan. Refuses a non-empty memory.
    Init {
        plan_dir: PathBuf,
        #[arg(long)]
        body_file: PathBuf,
    },
    /// Rewrite an entry's body from a file or stdin.
    SetBody {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Update an entry's title. Id is preserved.
    SetTitle {
        plan_dir: PathBuf,
        id: String,
        new_title: String,
    },
    /// Delete an entry by id.
    Delete {
        plan_dir: PathBuf,
        id: String,
    },
}

#[derive(Subcommand)]
enum SessionLogCommands {
    /// List sessions from session-log.yaml (id + timestamp + phase + body).
    List {
        plan_dir: PathBuf,
        /// Truncate output to the last N sessions (newest-kept).
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Show a single session record by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a session record to session-log.yaml. Idempotent on id:
    /// a record with the same id already present is a no-op.
    Append {
        plan_dir: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        timestamp: String,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Overwrite latest-session.yaml with a new single record. Used by
    /// analyse-work to hand the session to git-commit-work.
    SetLatest {
        plan_dir: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        timestamp: String,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Emit latest-session.yaml's record.
    ShowLatest {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum ProjectsCommands {
    /// Emit the catalog as YAML on stdout (empty catalog is valid output).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Add an entry mapping `--name` to `--path`. Relative paths are
    /// resolved against the current working directory. If `--name` is
    /// omitted, the basename of the resolved path is used. Rejects
    /// duplicate names and duplicate paths.
    Add {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Project name. Defaults to the basename of `--path`.
        #[arg(long)]
        name: Option<String>,
        /// Project directory. Absolute, or relative to the current
        /// working directory.
        #[arg(long)]
        path: PathBuf,
    },
    /// Remove the entry for the given name. Errors if no such entry exists.
    Remove {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        name: String,
    },
    /// Rename an existing entry. Cascades into
    /// `<config-dir>/related-components.yaml` (every edge participant
    /// matching `<old>` is rewritten to `<new>`) and into the discover
    /// surface cache at `<config-dir>/discover-cache/<name>.yaml`.
    Rename {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        old: String,
        new: String,
    },
}

#[derive(Subcommand)]
enum RelatedComponentsCommands {
    /// Emit the file as YAML. With `--plan`, filter to edges that
    /// involve the component derived from the plan dir. `--kind` and
    /// `--lifecycle` compose with `--plan` (all filters AND-combine).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict output to edges involving the component that owns
        /// `<plan>` (derived as `<plan>/../..`).
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Only emit edges whose kind matches this ontology v2 kebab-case
        /// name (e.g. `generates`, `co-implements`).
        #[arg(long)]
        kind: Option<String>,
        /// Only emit edges whose lifecycle matches this ontology v2
        /// kebab-case scope (e.g. `runtime`, `codegen`, `dev-workflow`).
        #[arg(long)]
        lifecycle: Option<String>,
    },
    /// Add an edge with the full ontology v2 field set. `kind` and
    /// `lifecycle` are positional; every other field is a flag.
    /// Symmetric kinds are participant-order-insensitive; directed
    /// kinds use canonical order from docs/component-ontology.md §6.
    /// Refuses unknown component names.
    AddEdge {
        #[arg(long)]
        config: Option<PathBuf>,
        /// One of the v2 kebab-case kinds (see
        /// docs/component-ontology.md §5).
        kind: String,
        /// One of the v2 kebab-case lifecycles (see
        /// docs/component-ontology.md §3.2).
        lifecycle: String,
        /// First participant. For directed kinds, the canonical-order
        /// "from" component.
        a: String,
        /// Second participant. For directed kinds, the canonical-order
        /// "to" component.
        b: String,
        /// Evidence grade: `strong`, `medium`, or `weak`. `strong`/`medium`
        /// require at least one `--evidence-field`; `weak` may omit.
        #[arg(long)]
        evidence_grade: String,
        /// Surface-field path that justifies this edge (e.g.
        /// `Ravel-Lite.produces_files`). Repeat for multiple fields.
        #[arg(long = "evidence-field", value_name = "FIELD")]
        evidence_fields: Vec<String>,
        /// One-paragraph human justification. Required; non-empty.
        #[arg(long)]
        rationale: String,
    },
    /// Remove the unique edge matching `(kind, lifecycle, canonicalised
    /// participants)`. Errors if no match. A v1-style invocation
    /// omitting `lifecycle` is rejected by clap's required-arg check.
    RemoveEdge {
        #[arg(long)]
        config: Option<PathBuf>,
        kind: String,
        lifecycle: String,
        a: String,
        b: String,
    },
    /// Run the two-stage LLM discovery pipeline over all catalogued
    /// components (or just `--project <name>`). Writes proposals to
    /// `<config-dir>/discover-proposals.yaml` for user review.
    Discover {
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict Stage 1 re-analysis to a single component; Stage 2
        /// still operates over the full catalog's cached surfaces.
        #[arg(long)]
        project: Option<String>,
        /// Maximum parallel Stage 1 subagents. Default 4.
        #[arg(long)]
        concurrency: Option<usize>,
        /// Skip the review gate: run `discover-apply` immediately after
        /// proposals are written.
        #[arg(long)]
        apply: bool,
    },
    /// Merge a previously-produced `discover-proposals.yaml` into
    /// `related-components.yaml`. Idempotent; reports and rejects
    /// directional conflicts without aborting.
    DiscoverApply {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DiscoverProposalsCommands {
    /// Append a Stage 2 edge proposal to `<config-dir>/discover-proposals.yaml`.
    /// Every validation is enforced here rather than at batch-parse time —
    /// clap rejects an unknown `--kind`/`--lifecycle`/`--evidence-grade`,
    /// `Edge::validate()` rejects self-loops and empty-evidence misuse, and
    /// the catalog check rejects participants not in `projects.yaml`.
    AddProposal {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location.
        #[arg(long)]
        config: Option<PathBuf>,
        /// One of the v2 kebab-case kinds (see
        /// docs/component-ontology.md §5).
        #[arg(long)]
        kind: String,
        /// One of the v2 kebab-case lifecycles (see
        /// docs/component-ontology.md §3.2).
        #[arg(long)]
        lifecycle: String,
        /// Component name (repeat twice). For directed kinds, the first
        /// `--participant` is the canonical-order "from" component and
        /// the second is the "to" component. Symmetric kinds are
        /// participant-order-insensitive; the verb canonicalises to
        /// sorted order before storage.
        #[arg(long = "participant", value_name = "NAME")]
        participants: Vec<String>,
        /// Evidence grade: `strong`, `medium`, or `weak`. `strong`/`medium`
        /// require at least one `--evidence-field`; `weak` may omit.
        #[arg(long)]
        evidence_grade: String,
        /// Surface-field path that justifies this edge (e.g.
        /// `Alpha.surface.produces_files`). Repeat for multiple fields.
        #[arg(long = "evidence-field", value_name = "FIELD")]
        evidence_fields: Vec<String>,
        /// One-paragraph human justification citing specific surface
        /// fields from the input. Required; non-empty.
        #[arg(long)]
        rationale: String,
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
            register_projects_from_plan_dirs(&config_root, &plan_dirs)?;
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
        Commands::State { command } => dispatch_state(command).await,
    }
}

async fn dispatch_state(command: StateCommands) -> Result<()> {
    match command {
        StateCommands::SetPhase { plan_dir, phase } => {
            state::run_set_phase(&plan_dir, &phase)
        }
        StateCommands::Projects { command } => match command {
            ProjectsCommands::List { config } => {
                let config_root = resolve_config_dir(config)?;
                projects::run_list(&config_root)
            }
            ProjectsCommands::Add { config, name, path } => {
                let config_root = resolve_config_dir(config)?;
                projects::run_add(&config_root, name.as_deref(), &path)
            }
            ProjectsCommands::Remove { config, name } => {
                let config_root = resolve_config_dir(config)?;
                projects::run_remove(&config_root, &name)
            }
            ProjectsCommands::Rename { config, old, new } => {
                let config_root = resolve_config_dir(config)?;
                projects::run_rename(&config_root, &old, &new)
            }
        },
        StateCommands::Backlog { command } => dispatch_backlog(command),
        StateCommands::Memory { command } => dispatch_memory(command),
        StateCommands::SessionLog { command } => dispatch_session_log(command),
        StateCommands::Migrate {
            plan_dir,
            dry_run,
            keep_originals: _,
            delete_originals,
            force,
        } => {
            let options = state::migrate::MigrateOptions {
                dry_run,
                original_policy: if delete_originals {
                    state::migrate::OriginalPolicy::Delete
                } else {
                    state::migrate::OriginalPolicy::Keep
                },
                force,
            };
            state::migrate::run_migrate(&plan_dir, &options)
        }
        StateCommands::RelatedComponents { command } => dispatch_related_components(command).await,
        StateCommands::DiscoverProposals { command } => dispatch_discover_proposals(command),
    }
}

fn dispatch_discover_proposals(command: DiscoverProposalsCommands) -> Result<()> {
    match command {
        DiscoverProposalsCommands::AddProposal {
            config,
            kind,
            lifecycle,
            participants,
            evidence_grade,
            evidence_fields,
            rationale,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            let evidence_grade = parse_evidence_grade(&evidence_grade)?;
            let req = state::discover_proposals::AddProposalRequest {
                kind,
                lifecycle,
                participants: &participants,
                evidence_grade,
                evidence_fields,
                rationale,
            };
            state::discover_proposals::run_add_proposal(&config_root, &req)
        }
    }
}

async fn dispatch_related_components(command: RelatedComponentsCommands) -> Result<()> {
    match command {
        RelatedComponentsCommands::List { config, plan, kind, lifecycle } => {
            let config_root = resolve_config_dir(config)?;
            let kind = kind.as_deref().map(parse_edge_kind).transpose()?;
            let lifecycle = lifecycle.as_deref().map(parse_lifecycle_scope).transpose()?;
            let filter = related_components::ListFilter {
                plan: plan.as_deref(),
                kind,
                lifecycle,
            };
            related_components::run_list(&config_root, &filter)
        }
        RelatedComponentsCommands::AddEdge {
            config,
            kind,
            lifecycle,
            a,
            b,
            evidence_grade,
            evidence_fields,
            rationale,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            let evidence_grade = parse_evidence_grade(&evidence_grade)?;
            let req = related_components::AddEdgeRequest {
                kind,
                lifecycle,
                a: &a,
                b: &b,
                evidence_grade,
                evidence_fields,
                rationale,
            };
            related_components::run_add_edge(&config_root, &req)
        }
        RelatedComponentsCommands::RemoveEdge {
            config,
            kind,
            lifecycle,
            a,
            b,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            related_components::run_remove_edge(&config_root, kind, lifecycle, &a, &b)
        }
        RelatedComponentsCommands::Discover {
            config,
            project,
            concurrency,
            apply: apply_flag,
        } => {
            let config_root = resolve_config_dir(config)?;
            let options = ravel_lite::discover::RunDiscoverOptions {
                project_filter: project,
                concurrency,
                apply: apply_flag,
            };
            ravel_lite::discover::run_discover(&config_root, options).await
        }
        RelatedComponentsCommands::DiscoverApply { config } => {
            let config_root = resolve_config_dir(config)?;
            ravel_lite::discover::apply::run_discover_apply(&config_root)
        }
    }
}

fn dispatch_backlog(command: BacklogCommands) -> Result<()> {
    use ravel_lite::state::backlog::{self, ListFilter, ReorderPosition, Status};

    match command {
        BacklogCommands::List {
            plan_dir,
            status,
            category,
            ready,
            has_handoff,
            missing_results,
            format,
        } => {
            let status = status
                .as_deref()
                .map(|s| {
                    Status::parse(s).ok_or_else(|| {
                        anyhow::anyhow!(
                            "invalid --status value {s:?}; expected one of not_started, in_progress, done, blocked"
                        )
                    })
                })
                .transpose()?;
            let filter = ListFilter {
                status,
                category,
                ready,
                has_handoff,
                missing_results,
            };
            let fmt = parse_output_format(&format)?;
            backlog::run_list(&plan_dir, &filter, fmt)
        }
        BacklogCommands::Show { plan_dir, id, format } => {
            let fmt = parse_output_format(&format)?;
            backlog::run_show(&plan_dir, &id, fmt)
        }
        BacklogCommands::Add {
            plan_dir,
            title,
            category,
            dependencies,
            description_file,
            description,
        } => {
            let description_body = resolve_body(description_file, description)?;
            let req = backlog::AddRequest {
                title,
                category,
                dependencies,
                description: description_body,
            };
            backlog::run_add(&plan_dir, &req)
        }
        BacklogCommands::Init { plan_dir, body_file } => {
            let text = std::fs::read_to_string(&body_file)
                .with_context(|| format!("failed to read {}", body_file.display()))?;
            let seed: backlog::BacklogFile = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {} as backlog.yaml", body_file.display()))?;
            backlog::run_init(&plan_dir, &seed)
        }
        BacklogCommands::SetStatus {
            plan_dir,
            id,
            status,
            reason,
        } => {
            let status = Status::parse(&status)
                .ok_or_else(|| anyhow::anyhow!("invalid status {status:?}"))?;
            backlog::run_set_status(&plan_dir, &id, status, reason.as_deref())
        }
        BacklogCommands::SetResults { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_results(&plan_dir, &id, &body)
        }
        BacklogCommands::SetDescription { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_description(&plan_dir, &id, &body)
        }
        BacklogCommands::SetHandoff { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_handoff(&plan_dir, &id, &body)
        }
        BacklogCommands::ClearHandoff { plan_dir, id } => {
            backlog::run_clear_handoff(&plan_dir, &id)
        }
        BacklogCommands::SetTitle { plan_dir, id, new_title } => {
            backlog::run_set_title(&plan_dir, &id, &new_title)
        }
        BacklogCommands::SetDependencies { plan_dir, id, deps } => {
            // clap parses `--deps ""` as a single empty string; normalise to
            // an empty vec so the documented clearing form works.
            let deps: Vec<String> = deps.into_iter().filter(|d| !d.is_empty()).collect();
            backlog::run_set_dependencies(&plan_dir, &id, &deps)
        }
        BacklogCommands::Reorder { plan_dir, id, position, target_id } => {
            let pos = ReorderPosition::parse(&position).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid reorder position {position:?}; expected `before` or `after`"
                )
            })?;
            backlog::run_reorder(&plan_dir, &id, pos, &target_id)
        }
        BacklogCommands::LintDependencies { plan_dir, format } => {
            let fmt = parse_output_format(&format)?;
            backlog::run_lint_dependencies(&plan_dir, fmt)
        }
        BacklogCommands::RepairStaleStatuses { plan_dir, dry_run, format } => {
            let fmt = parse_output_format(&format)?;
            let count = backlog::run_repair_stale_statuses(&plan_dir, dry_run, fmt)?;
            // Non-zero exit iff any repair would apply — scripts poll
            // this verb before a mutating run to detect status drift
            // without parsing YAML.
            if count > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        BacklogCommands::Delete { plan_dir, id, force } => {
            backlog::run_delete(&plan_dir, &id, force)
        }
    }
}

fn parse_output_format(input: &str) -> Result<ravel_lite::state::backlog::OutputFormat> {
    ravel_lite::state::backlog::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn parse_memory_format(input: &str) -> Result<ravel_lite::state::memory::OutputFormat> {
    ravel_lite::state::memory::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_memory(command: MemoryCommands) -> Result<()> {
    use ravel_lite::state::memory;

    match command {
        MemoryCommands::List { plan_dir, format } => {
            let fmt = parse_memory_format(&format)?;
            memory::run_list(&plan_dir, fmt)
        }
        MemoryCommands::Show { plan_dir, id, format } => {
            let fmt = parse_memory_format(&format)?;
            memory::run_show(&plan_dir, &id, fmt)
        }
        MemoryCommands::Add {
            plan_dir,
            title,
            body_file,
            body,
        } => {
            let body = resolve_body(body_file, body)?;
            let req = memory::AddRequest { title, body };
            memory::run_add(&plan_dir, &req)
        }
        MemoryCommands::Init { plan_dir, body_file } => {
            let text = std::fs::read_to_string(&body_file)
                .with_context(|| format!("failed to read {}", body_file.display()))?;
            let seed: memory::MemoryFile = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {} as memory.yaml", body_file.display()))?;
            memory::run_init(&plan_dir, &seed)
        }
        MemoryCommands::SetBody { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            memory::run_set_body(&plan_dir, &id, &body)
        }
        MemoryCommands::SetTitle { plan_dir, id, new_title } => {
            memory::run_set_title(&plan_dir, &id, &new_title)
        }
        MemoryCommands::Delete { plan_dir, id } => {
            memory::run_delete(&plan_dir, &id)
        }
    }
}

fn parse_session_log_format(input: &str) -> Result<ravel_lite::state::session_log::OutputFormat> {
    ravel_lite::state::session_log::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_session_log(command: SessionLogCommands) -> Result<()> {
    use ravel_lite::state::session_log;

    match command {
        SessionLogCommands::List { plan_dir, limit, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_list(&plan_dir, limit, fmt)
        }
        SessionLogCommands::Show { plan_dir, id, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_show(&plan_dir, &id, fmt)
        }
        SessionLogCommands::Append {
            plan_dir,
            id,
            timestamp,
            phase,
            body_file,
            body,
        } => {
            let body = resolve_body(body_file, body)?;
            let record = session_log::build_record_for_append(
                Some(id),
                Some(timestamp),
                phase,
                &body,
            )?;
            session_log::run_append(&plan_dir, &record)
        }
        SessionLogCommands::SetLatest {
            plan_dir,
            id,
            timestamp,
            phase,
            body_file,
            body,
        } => {
            let body = resolve_body(body_file, body)?;
            let record = session_log::build_record_for_append(
                Some(id),
                Some(timestamp),
                phase,
                &body,
            )?;
            session_log::run_set_latest(&plan_dir, &record)
        }
        SessionLogCommands::ShowLatest { plan_dir, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_show_latest(&plan_dir, fmt)
        }
    }
}

/// Resolve `--body-file <path>` vs `--body <value>` vs `--body -` (stdin).
/// Exactly one of the two arguments must be set; if neither is set,
/// returns an empty string (used for optional bodies like an add with no
/// description).
fn resolve_body(body_file: Option<PathBuf>, body: Option<String>) -> Result<String> {
    match (body_file, body) {
        (Some(path), None) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display())),
        (None, Some(value)) if value == "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read body from stdin")?;
            Ok(buf)
        }
        (None, Some(value)) => Ok(value),
        (None, None) => Ok(String::new()),
        (Some(_), Some(_)) => bail!("pass only one of --body-file or --body"),
    }
}

/// Ensure every distinct project implied by the requested plan dirs is
/// present in the catalog before the TUI takes over stdio. Runs the
/// collision-prompt interactively against the real stdin/stderr.
/// Keeping this in main.rs (rather than inside `run_phase_loop`)
/// guarantees the prompt happens while stdin is still a plain tty and
/// errors surface before any agent spawn.
fn register_projects_from_plan_dirs(config_root: &Path, plan_dirs: &[PathBuf]) -> Result<()> {
    use std::collections::BTreeSet;

    let mut project_paths: BTreeSet<PathBuf> = BTreeSet::new();
    for plan_dir in plan_dirs {
        let project = project_root_for_plan(plan_dir)?;
        project_paths.insert(PathBuf::from(project));
    }
    let stdin = std::io::stdin();
    let mut stdin_lock = stdin.lock();
    let stderr = std::io::stderr();
    let mut stderr_lock = stderr.lock();
    for project_path in project_paths {
        projects::ensure_in_catalog_interactive(
            config_root,
            &project_path,
            &mut stderr_lock,
            &mut stdin_lock,
        )?;
    }
    Ok(())
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
        related_plans: related_components::read_related_plans_markdown(plan_dir),
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
