# Architecture

`raveloop` is a single Rust binary with a Ratatui TUI. It orchestrates
a phase loop for LLM-driven development by spawning a Claude Code or Pi
subprocess per phase, reading its JSON stream output, and rendering
progress.

## Principles

- **No magic.** All config, prompts, phase state, and memory live as
  readable files on disk. The binary reads them at runtime. Default
  content is embedded in the binary only so that `init` can write it to
  disk — nothing is read from embedded content during `run`.
- **Visible, auditable, adjustable.** Every input the orchestrator uses
  is a file the user can inspect and edit. Every state transition
  writes to the filesystem.
- **Agents are subprocesses.** The orchestrator spawns `claude` or `pi`
  CLI processes, reads their JSON stream output, and renders progress.
  It never calls LLM APIs directly.
- **Configuration drives everything.** There are no CLI flags for
  agent selection, model choice, or permissions. Everything comes
  from YAML files under the config directory.

## Overview

```
┌─────────────────────────────────────────────────────┐
│  main()                                              │
│  Parse CLI args, load config, build PlanContext      │
│  Create agent, create TUI, run phase_loop            │
└──────────────────────┬───────────────────────────────┘
                       │
          ┌────────────▼────────────┐
          │      phase_loop         │
          │  Reads phase.md         │
          │  Drives state machine   │
          │  Calls agent methods    │
          │  Sends events to TUI    │
          └────┬───────────┬────────┘
               │           │
    ┌──────────▼──┐   ┌────▼──────────┐
    │   Agent     │   │     TUI       │
    │  (trait)    │   │  (Ratatui)    │
    │             │   │               │
    │ Spawns CLI  │   │ Renders log,  │
    │ Parses JSON │   │ progress,     │
    │ Emits events│   │ status bar    │
    └─────────────┘   └───────────────┘
```

## Message Model

All communication to the TUI flows through a single
`mpsc::UnboundedSender<UIMessage>` channel. Both agents and the phase
loop send messages on the same channel.

```rust
pub enum UIMessage {
    // ── From agents ──────────────────────────────────────

    /// Overwritable progress for an agent (latest tool call).
    Progress { agent_id: String, text: String },

    /// Permanent output — appended to the scrolling log.
    Persist { agent_id: String, text: String },

    /// Agent finished. Remove its progress group from the live area.
    AgentDone { agent_id: String },

    // ── From the phase loop ──────────────────────────────

    /// Permanent log entry (phase headers, commit messages, etc.)
    Log(String),

    /// Register an agent group in the live area with a header line.
    RegisterAgent { agent_id: String, header: String },

    /// Prompt the user for y/n. Reply via the oneshot sender.
    Confirm { message: String, reply: oneshot::Sender<bool> },

    /// Suspend the TUI (leave raw mode) for interactive phase.
    Suspend,

    /// Resume the TUI (re-enter raw mode) after interactive phase.
    Resume,
}
```

- `Progress` — the TUI shows only the latest one per `agent_id`,
  indented under that agent's header in the live area.
- `Persist` — highlight labels (`★ Updating memory`), result text with
  action markers, tool errors. Appended to the log, never overwritten.
- `AgentDone` — the agent's progress group is removed from the live
  area.
- `RegisterAgent` — creates a group in the live area with a header.
  Subsequent `Progress` events for that `agent_id` appear indented
  under the header.
- `Log` — permanent output from the phase loop (not from an agent).
- `Suspend` / `Resume` — bracket the interactive work phase.

## Agent Trait

```rust
pub type UISender = mpsc::UnboundedSender<UIMessage>;

#[async_trait]
pub trait Agent: Send + Sync {
    /// Interactive phase — agent owns the terminal.
    /// TUI must be suspended before calling this.
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()>;

    /// Headless phase — agent streams events for the TUI to render.
    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        tx: UISender,
    ) -> Result<()>;

    /// Dispatch a subagent to a target plan. Streams events with its
    /// own agent_id so concurrent subagents render as separate groups.
    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        tx: UISender,
    ) -> Result<()>;

    fn tokens(&self) -> HashMap<String, String>;

    async fn setup(&self, _ctx: &PlanContext) -> Result<()> {
        Ok(())
    }
}
```

Each headless invocation and each concurrent subagent gets a clone of
the same `UISender`. Agents only send `Progress`, `Persist`, and
`AgentDone` variants. The TUI holds the single receiver.

### Agent implementations

- **ClaudeCodeAgent** — spawns `claude` with
  `--output-format stream-json`, reads stdout line by line via
  `tokio::io::BufReader`, parses each line as JSON. `assistant` events
  with `tool_use` blocks become `Progress`; writes to highlight-matched
  paths (memory.md, backlog.md) and `result` events become `Persist`.
- **PiAgent** — spawns `pi` with `--mode json`. Different JSON event
  schema but the same output-event mapping.

Both agents share formatting logic (action marker parsing, highlight
rules, tool name cleaning) via the `format` module. Agent
implementations do not format strings themselves.

For `invoke_interactive`, both agents spawn their CLI with inherited
stdio (the TUI has been suspended, so the terminal is available).

## TUI Layout

The terminal is divided into three regions via Ratatui's `Layout`:

```
┌──────────────────────────────────────────────────┐
│  Scrolling log (permanent output)                │ ← fills available space
│                                                  │
│  ────────────────────────────────────────────     │
│    ◆  REFLECT  ·  mnemosyne-orchestrator         │
│    Distil session learnings into durable memory   │
│  ────────────────────────────────────────────     │
│    ★  Updating memory                            │
│                                                  │
│    ADDED      New memory entry — description      │
│    SHARPENED  Existing entry — what changed       │
│                                                  │
│    ⚙  COMMIT · reflect  ·  run-plan: reflect     │
│                                                  │
│  ▶ Dispatching 3 subagent(s)...                  │
│    ✓ sub-B-phase-cycle                           │
├──────────────────────────────────────────────────┤
│  Live progress area (per-agent groups)           │ ← sized to content
│                                                  │
│    → child: sub-F-phase-cycle                    │
│        · Edit backlog.md                         │
│    → child: sub-H-phase-cycle                    │
│        · Read memory.md                          │
├──────────────────────────────────────────────────┤
│  Mnemosyne · mnemosyne-orchestrator · triage     │ ← 1 line, fixed
│  · claude-code                                   │
└──────────────────────────────────────────────────┘
```

### Log area (top)

A scrolling paragraph widget showing all permanent output. New entries
are appended at the bottom and the view auto-scrolls. The log is
stored as a `Vec<String>`.

Phase headers, commit messages, persist events, subagent completion
messages, and error banners all live here.

### Live progress area (middle)

Sized dynamically to fit the current number of active agents. Each
agent group has a header line (e.g. `→ child: sub-F-phase-cycle`, or
the phase header for the main agent) and an indented progress line
showing the latest tool call. When an agent completes, its group is
removed and the area shrinks. When no agents are active, this area is
zero height.

```rust
struct AgentProgress {
    header: String,
    progress: Option<String>,
}

// Keyed by agent_id, insertion-ordered
progress_groups: IndexMap<String, AgentProgress>,
```

### No status bar

Earlier iterations carried a persistent status bar at the bottom of the
viewport. It was removed in favour of a linear-scrolling model: log
lines (phase headers, commits, persisted summaries) flow into native
terminal scrollback, and the inline viewport is reserved for transient
state only (current tool call, optional confirm prompt). Scrolling the
terminal or `tee`ing to a file recovers the full history.

## Phase Loop

The phase loop is an async function that drives the state machine. It
does not own the TUI — it communicates through a `UI` handle that
wraps the same `UISender` the agents use:

```rust
pub struct UI {
    tx: UISender,
}

impl UI {
    pub fn log(&self, text: &str) { ... }
    pub fn register_agent(&self, agent_id: &str, header: &str) { ... }
    pub async fn confirm(&self, message: &str) -> bool { ... }
    pub fn suspend(&self) { ... }
    pub fn resume(&self) { ... }
}
```

`confirm` sends a `Confirm` message carrying a `oneshot::Sender` and
awaits the reply. The TUI renders the prompt in the live area,
captures the keypress, and sends the response back.

### Phase state machine

```
Work → AnalyseWork → GitCommitWork → Reflect → GitCommitReflect
     → [if should_dream: Dream → GitCommitDream]
     → Triage → GitCommitTriage → Work → ...
```

Script phases (the `GitCommit*` steps) are handled inline — no
subprocess, just `git add` + `git commit` via `std::process::Command`.
The dream guard (`should_dream`) runs in the `GitCommitReflect`
handler; when dream is skipped, `GitCommitDream` is skipped too — a
`GitCommit*` phase only runs when its paired LLM phase produced work.

### Interactive phase handling

The work phase uses `invoke_interactive`, which needs the terminal:

1. `ui.suspend()` — TUI leaves raw mode, restores the normal terminal.
2. `invoke_interactive` runs — the agent subprocess inherits stdio.
3. `ui.resume()` — TUI re-enters raw mode and repaints from log
   history.

Ratatui repaints the full screen on resume, so the user sees the log
from before the interactive phase plus any new entries.

## Concurrent Subagent Dispatch

After the triage phase, the orchestrator reads
`<plan>/subagent-dispatch.yaml` (if present) and fans out one tokio
task per entry:

```rust
pub async fn dispatch_subagents(
    agent: Arc<dyn Agent>,
    plan_dir: &Path,
    ui: &UI,
) -> Result<()> {
    let dispatches = parse_dispatch_file(plan_dir)?;
    if dispatches.is_empty() { return Ok(()) }

    ui.log(&format!("\n▶ Dispatching {} subagent(s)...", dispatches.len()));

    let mut join_set: JoinSet<(String, Result<()>)> = JoinSet::new();

    for dispatch in &dispatches {
        let agent_id = basename(&dispatch.target);
        let tx = ui.sender();  // clone of the UISender

        ui.register_agent(
            &agent_id,
            &format!("  → {}: {}", dispatch.kind, dispatch.target),
        );

        let agent = Arc::clone(&agent);
        let prompt = build_prompt(dispatch);
        let target = dispatch.target.clone();
        let id = agent_id.clone();

        join_set.spawn(async move {
            let result = agent.dispatch_subagent(&prompt, &target, tx).await;
            (id, result)
        });
    }

    while let Some(Ok((agent_id, result))) = join_set.join_next().await {
        match result {
            Ok(()) => ui.log(&format!("  ✓ {}", agent_id)),
            Err(e) => ui.log(&format!("  ✗ {}: {}", agent_id, e)),
        }
    }

    fs::remove_file(plan_dir.join("subagent-dispatch.yaml"))?;
    Ok(())
}
```

Subagents all send on the same channel, distinguished by `agent_id`,
so the TUI renders them side-by-side as separate groups in the live
area.

## Formatting Module

Pure functions, no terminal writes, no mutable state:

```rust
pub struct FormattedOutput {
    pub text: String,
    pub persist: bool,
}

pub fn format_tool_call(tool: &ToolCall, phase: Option<LlmPhase>) -> FormattedOutput;
pub fn format_result_text(text: &str) -> String;
pub fn extract_edit_context(old: Option<&str>, new: Option<&str>) -> Option<String>;
pub fn clean_tool_name(name: &str) -> String;
```

Agent implementations call these to produce `FormattedOutput`, then map
to `UIMessage` variants. Phase highlight rules (`PHASE_HIGHLIGHTS`),
action marker styles (`ACTION_STYLES`), and phase info (`PHASE_INFO`)
are static data in this module.

Highlight deduplication (the `shown_highlights` set) lives in the
agent's headless invocation scope — reset per phase, checked before
emitting a `Persist` event.

## File Layout

### Source tree

```
raveloop/
├── Cargo.toml
├── defaults/                   # embedded by include_str!, written by init
│   ├── config.yaml
│   ├── agents/…
│   ├── phases/…
│   ├── fixed-memory/…
│   └── skills/…
└── src/
    ├── main.rs
    ├── config.rs               # YAML config loading
    ├── types.rs                # LlmPhase, ScriptPhase, PlanContext, etc.
    ├── agent/
    │   ├── mod.rs              # Agent trait
    │   ├── claude_code.rs      # ClaudeCodeAgent + stream parser
    │   └── pi.rs               # PiAgent + stream parser
    ├── format.rs               # Pure formatting functions
    ├── phase_loop.rs           # Phase state machine
    ├── subagent.rs             # Dispatch + concurrent execution
    ├── git.rs                  # git commit, baseline save
    ├── dream.rs                # should_dream, update_baseline
    ├── prompt.rs               # Template loading + token substitution
    ├── init.rs                 # `init` command — writes defaults
    ├── survey.rs               # `survey` command — multi-root LLM status
    ├── create.rs               # `create` command — interactive plan scaffold
    └── ui.rs                   # Ratatui TUI, UI handle, rendering
```

Crate dependencies are defined in `Cargo.toml`.

### Config directory (created by `init`)

```
<config-dir>/
├── config.yaml                 # agent, headroom
├── agents/
│   ├── claude-code/
│   │   ├── config.yaml         # per-phase model/param config
│   │   └── tokens.yaml         # template tokens
│   └── pi/
│       ├── config.yaml
│       ├── tokens.yaml
│       └── prompts/
│           ├── system-prompt.md
│           └── memory-prompt.md
├── phases/                     # phase prompt templates
│   ├── work.md
│   ├── analyse-work.md
│   ├── reflect.md
│   ├── dream.md
│   └── triage.md
├── fixed-memory/               # shared style guides
│   ├── coding-style.md
│   ├── coding-style-rust.md
│   └── memory-style.md
├── skills/
│   ├── brainstorming.md
│   ├── tdd.md
│   └── writing-plans.md
├── survey.md                   # prompt template for `survey` subcommand
└── create-plan.md              # prompt template for `create` subcommand
```

The config directory can live inside a project repo (versioned with
the code), shared across multiple projects, or kept standalone. Its
location is not tied to any project.

`init` skips files that already exist — rerunning it after an upgrade
picks up new defaults without overwriting user customisations.

### Plan directories (anywhere on disk)

```
my-plan/
├── phase.md
├── backlog.md
├── memory.md
├── dream-baseline
├── session-log.md
├── related-plans.md
└── …
```

The project directory for a plan is found by walking up from the plan
directory until a `.git` is found.

## CLI and Invocation

The user interacts with `raveloop` directly once to create a
config directory, then drives the phase cycle with `raveloop run`.
There is no trampoline — the binary resolves its config directory via
an explicit precedence chain.

### `raveloop init <dir>`

Creates the config directory at `<dir>` with the default structure.
Default file contents are embedded in the binary at compile time via
`include_str!`.

After scaffolding, `init` prints guidance on how to make the binary
find that directory: either set `RAVELOOP_CONFIG=<dir>` or pass
`--config <dir>` on each invocation. When `<dir>` is the default
location (`dirs::config_dir()/raveloop/`), no setup is needed.

### Config discovery

Every `raveloop` subcommand that needs config resolves the config
directory in this order:

1. `--config <path>` CLI flag
2. `RAVELOOP_CONFIG` environment variable
3. Default location: `<dirs::config_dir()>/raveloop/`
4. Hard error with a pointer to `raveloop init`

No walk-up, no registry, no implicit project root. The first source
that resolves to an existing directory wins; if that directory doesn't
exist, `raveloop` errors with the candidate path and the source
that produced it.

### `raveloop run [--config <dir>] <plan-directory>`

The main phase loop. Takes an optional config root (resolved via the
discovery chain if omitted) and a plan directory.

### `raveloop create [--config <dir>] <plan-dir>`

Interactively scaffolds a new plan directory. Validates that
`<plan-dir>` does not already exist and that its parent does, then
loads the prompt template at `<config-dir>/create-plan.md`, appends
the target path, and spawns a headful `claude` session with inherited
stdio. The user drives the conversation directly; Raveloop's only job
is path validation, prompt composition, and post-hoc confirmation
that `phase.md` was created.

The session reuses the configured work-phase model
(`models.work` in the agent config) — plan creation is work-phase-like
reasoning and deserves the same budget. Claude is launched with
`--add-dir <parent>` to scope its tool access to the target parent
directory; interactive tool-approval prompts still fire as normal,
which is appropriate for a headful session. Supports `claude-code`
only in v1.

### `raveloop survey [--config <dir>] --root <path> [--root <path> ...] [--model <m>]`

Produces an LLM-driven multi-project plan-status overview. For each
`--root` (a directory whose immediate subdirectories are plans),
discovers plans by `phase.md` presence and derives each plan's
project from the nearest ancestor directory containing `.git`. Bundles
each plan's `phase.md`, `backlog.md`, and `memory.md` into a single
fresh-context `claude` invocation alongside the prompt template at
`<config-dir>/survey.md`.

The **LLM emits YAML** matching a fixed schema (per-plan counts,
cross-plan blockers, parallel streams, recommended invocation
order); the tool parses the response and renders the final output
deterministically. The per-plan summary is grouped by project with a
compact `U/B/D/R` counts column per plan and notes rendered as a
wrapped body line below; the three prose sections (blockers, streams,
recommendations) use a header-then-body shape so wrap continuations
are never confused with new logical lines. This split means column
alignment, blank-line spacing, and wrap-width consistency are
guaranteed by the tool rather than hoped for from the LLM. Raw stdout
from claude is preserved in the error message when YAML parsing
fails, so malformed responses surface clearly.

Survey is single-shot and read-only by construction: no tool use, no
session persistence, no file writes. Model precedence is `--model`
flag → `models.survey` in the agent config → `DEFAULT_SURVEY_MODEL`
(currently `claude-haiku-4-5`). Supports `claude-code` only in v1.

### Configuration

```yaml
# <config-dir>/config.yaml
agent: claude-code
headroom: 1500
```

```yaml
# <config-dir>/agents/claude-code/config.yaml
models:
  work: claude-sonnet-4-6
  analyse-work: claude-haiku-4-5-20251001
  reflect: claude-haiku-4-5-20251001
  dream: claude-haiku-4-5-20251001
  triage: claude-haiku-4-5-20251001
params:
  work:
    dangerous: true
  analyse-work:
    dangerous: true
  reflect:
    dangerous: true
  dream:
    dangerous: true
  triage:
    dangerous: true
```

Per-phase `params` maps contain agent-specific CLI flags. For
`claude-code`, `dangerous: true` adds `--dangerously-skip-permissions`.
This keeps the `Agent` trait generic — the orchestrator doesn't need
to know what flags each agent supports.

`raveloop run --dangerous <plan_dir>` mutates the loaded
`AgentConfig` at startup, setting `dangerous: true` for every LLM
phase before the agent is constructed — so the agent itself still
reads a single source of truth (`config.params`), and no parallel
override channel is needed (claude-code only; ignored with a warning
for other agents).

All `claude-code` invocations (interactive and headless) pass
`--add-dir <plan_dir>` so Claude's sandbox permits writes into the
plan directory. Without it, `memory.md`, `latest-session.md`,
`backlog.md`, and `subagent-dispatch.yaml` writes would be denied
because `plan_dir` lives outside the project tree (`current_dir` is
`project_dir`).
