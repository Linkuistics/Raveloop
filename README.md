# LLM_CONTEXT_PI

Shared context files for LLM-assisted development across multiple
related projects, driven by the pi coding agent. Provides coding style
references, the backlog-plan multi-session work format, and the scripts
that drive it.

## Harness: pi

LLM_CONTEXT_PI uses [pi](https://github.com/mariozechner/pi-coding-agent)
(`@mariozechner/pi-coding-agent`) as its LLM harness in place of Claude
Code.

**Installation:**

```bash
npm install -g @mariozechner/pi-coding-agent
```

**Provider API keys** are supplied via environment variables. The
variable name depends on the provider — for Anthropic, set
`ANTHROPIC_API_KEY`. Pi is multi-provider; set `PROVIDER` in `config.sh`
to select the active provider (default: `anthropic`).

**Defaults:** Anthropic provider, claude-opus-4-6 for the work phase
(where the engineering happens), claude-sonnet-4-6 for the three
headless editorial phases (reflect, compact, triage).

**Key pi flags used by `run-plan.sh`:**

- `--no-session` — each phase starts with a completely fresh context window.
- `--append-system-prompt <text>` — injects `prompts/system-prompt.md` (and for
  the work phase, `prompts/memory-prompt.md`) before the phase prompt.
- `--mode json -p` — runs a phase headlessly; pi emits JSONL piped through
  `format_pi_stream` for a readable trace of tool calls and final output.
- `--provider <name>` — selects the API provider (from `config.sh`).
- `--model <id>` — selects the model per phase (from `config.sh`).
- `--thinking <level>` — enables extended thinking (`medium` for work;
  empty for headless phases).

## Auto-memory system

LLM_CONTEXT_PI includes a persistent, file-based memory system that
survives across sessions and plans. Memory is stored per-project in:

```
~/.claude-pi/projects/<path-encoded>/memory/
```

where `<path-encoded>` is the project's absolute path with every `/`
replaced by `-`. `run-plan.sh` creates this directory automatically.

### Memory types

- **user** — the user's role, goals, preferences, and expertise.
- **feedback** — corrections and confirmations about how to approach
  work. Structured with a rule line, a **Why:** line, and a
  **How to apply:** line.
- **project** — ongoing work, goals, decisions, bugs, or incidents not
  derivable from code or git history.
- **reference** — pointers to where information lives in external systems.

A `MEMORY.md` index file lists all saved memories as one-line pointers
and is the file injected into phase contexts.

### Phase access model

- **Work phase:** receives full read+write access. `prompts/memory-prompt.md`
  is appended to the system prompt, instructing the agent to save new
  memories during the session. The current `MEMORY.md` index is also
  injected.
- **Headless phases (reflect, compact, triage):** receive the
  `MEMORY.md` index as read-only context only.

### Migrating from Claude Code

```bash
cp -r ~/.claude/projects/<path>/memory/ \
      ~/.claude-pi/projects/<path>/memory/
```

Both systems use the same path-encoding convention (`/` replaced by
`-`), so the directory names match.

See `prompts/memory-prompt.md` for the full instructions governing what the
work-phase agent saves and how.

## Directory layout

```
LLM_CONTEXT_PI/
├── README.md                  # This file — canonical spec for humans
├── create-plan.md             # How to create a new backlog plan
├── run-plan.sh                # Driver for the four-phase work cycle
├── config.sh                  # Per-phase model/thinking + HEADROOM (sourced by run-plan.sh)
├── prompts/                   # Prompt artifacts injected by run-plan.sh
│   ├── system-prompt.md       # System prompt addendum injected into every phase
│   └── memory-prompt.md       # Auto-memory instructions injected into the work phase
├── phases/                    # Operational phase prompts (read by run-plan.sh)
│   ├── work.md
│   ├── reflect.md
│   ├── compact.md
│   └── triage.md
└── fixed-memory/              # Universal reference material (read when relevant)
    ├── coding-style.md
    ├── coding-style-rust.md
    └── memory-style.md
```

## Configuration

`config.sh` next to `run-plan.sh` carries the run-time knobs. It is a
plain bash file sourced at startup. Variables:

- **`PROVIDER`** — the pi provider name (e.g. `anthropic`, `openai`,
  `gemini`). Determines which environment variable supplies the API
  key and which model namespace applies.
- **`WORK_MODEL`** — model ID for the work phase. Defaults to
  `claude-opus-4-6` (engineering work; use the strongest model).
- **`REFLECT_MODEL`** — model ID for the reflect phase. Defaults to
  `claude-sonnet-4-6` (editorial; Sonnet handles it well).
- **`COMPACT_MODEL`** — model ID for the compact phase. Defaults to
  `claude-sonnet-4-6` (lossless rewrite; editorial quality, not
  reasoning depth).
- **`TRIAGE_MODEL`** — model ID for the triage phase. Defaults to
  `claude-sonnet-4-6` (task ordering + propagation list; judgment-light).
- **`WORK_THINKING`** — pi thinking level for the work phase (`off`,
  `minimal`, `low`, `medium`, `high`, `xhigh`). Defaults to `medium`.
- **`REFLECT_THINKING`** — thinking level for the reflect phase.
  Defaults to empty (let pi decide).
- **`COMPACT_THINKING`** — thinking level for the compact phase.
  Defaults to empty.
- **`TRIAGE_THINKING`** — thinking level for the triage phase.
  Defaults to empty.
- **`HEADROOM`** — words of growth past `<plan>/compact-baseline`
  before the compact phase fires. Default 1500.

Edit `config.sh` to change any of these. `run-plan.sh` carries fallback
defaults so the script keeps working if the file is missing or
partially populated.

## Fixed memory

`fixed-memory/` holds universal cross-project reference material that
LLM phases consult when relevant. Unlike plan-scoped `memory.md`, these
files are stable, shared across every project, and not curated per
session. Each file serves a specific phase:

- **`coding-style.md`** — language-agnostic coding conventions.
  `phases/work.md` instructs the work-phase agent to read this at
  the moment it is about to write or modify any code.
- **`coding-style-<lang>.md`** — language-specific conventions. Current
  contents: `coding-style-rust.md`. **To support a new language, drop
  in a `coding-style-<lang>.md` file**; no script changes needed — the
  work prompt tells the agent to look in `fixed-memory/` for a matching
  file before touching code in that language.
- **`memory-style.md`** — the rules that govern `memory.md` entries
  across all plans. Read by the reflect and compact phases when
  writing or rewriting memory content.

The coding-style reads are **just-in-time**: the work phase pulls in a
style file only at the moment it is about to write code in that
language, so a cycle that does no coding work (pure docs, planning,
triage) never incurs the cost.

## Usage

Projects invoke `run-plan.sh` with an absolute path to a plan directory
under their `LLM_STATE/`:

```bash
~/Development/LLM_CONTEXT_PI/run-plan.sh ~/Development/LLM_CONTEXT_PI/LLM_STATE/{plan-name}
```

The script self-locates `LLM_CONTEXT_PI` from its own path, walks up from
the plan directory to the project root (`.git`), and drives a four-phase
cycle.

## Creating a Multi-Session Plan

To create a new backlog plan:

1. Start a pi session in your project
2. Ask pi to read `LLM_CONTEXT_PI/create-plan.md`
3. Pi walks you through scoping the plan via conversation
4. The output is a plan directory under `LLM_STATE/`

See `create-plan.md` for details.

## Backlog Plan Format

A backlog plan is a living document for exploratory, incremental work
across multiple sessions. It replaces rigid milestone-based plans with
a mutable task backlog and a multi-phase work cycle that separates doing
from reflecting from replanning.

### Philosophy

Work is exploratory. Tasks emerge from doing the work — you discover
what needs doing by doing adjacent things. A backlog plan embraces this:
the task list is mutable, new tasks are added as they're discovered,
priorities shift based on what you learn, and each cycle starts by
choosing the best next task rather than continuing from a fixed
position.

Each cycle runs through three core phases (work, reflect, triage) and
a conditional fourth phase (compact), each in a fresh pi session.
Fresh context between phases prevents accumulated noise and lets each
phase read only what it needs without bias from the previous phase.

The work phase is **interactive** — the user steers task selection.
Reflect, compact, and triage run in **headless** mode (`pi --mode json -p`)
and auto-exit when the LLM finishes.

### Plan Directory Structure

```
LLM_STATE/{plan-name}/
├── backlog.md          # Task backlog (mutable)
├── memory.md           # Distilled learnings (prunable; rewritten losslessly by compact)
├── session-log.md      # Raw session records (append-only, never read by any LLM phase)
├── latest-session.md   # Current session entry (overwritten each cycle; reflect's only session input)
├── phase.md            # Current phase: work, reflect, compact, or triage
├── compact-baseline    # Post-compact wc -w of memory.md (script-managed bookkeeping)
├── related-plans.md    # (optional) Declared peer-project relationships
├── pre-work.sh         # (optional) Executable bootstrap hook run before each work phase
├── prompt-work.md      # (optional) Plan-specific work phase overrides
├── prompt-reflect.md   # (optional) Plan-specific reflect phase overrides
├── prompt-compact.md   # (optional) Plan-specific compact phase overrides
└── prompt-triage.md    # (optional) Plan-specific triage phase overrides
```

All `prompt-<phase>.md` files are **optional**. When present, the run
script concatenates the shared `phases/<phase>.md` with the plan-specific
content. When absent, only the shared phase file is used. Most plans
will have no prompt files (reflect/compact/triage have zero plan-specific
content) or only `prompt-work.md` (for project-specific commands and
plan-specific constraints). Coding-style references do **not** belong
in `prompt-work.md` — see "Fixed memory" above.

### backlog.md

The task backlog. Contains:

**1. Task Backlog** — a mutable list of tasks. Each task has:

- **Title** — concise description of what to do
- **Status** — `not_started`, `in_progress`, `done`, `blocked`
- **Category** — a grouping tag for triage (e.g., `[collection]`, `[apps]`). Categories are not phases — they help you decide what to work on, not what order.
- **Dependencies** — other tasks or external conditions that must be met first
- **Description** — what and why, with enough context to start work
- **Results** — filled in when done: what happened, what was learned

Done tasks are deleted, not retained. The work phase may mark a task
`done` and leave it in place for the cycle; reflect distills any
learnings into `memory.md`; triage then removes the task entirely.
`backlog.md` must not carry a standing "Completed Tasks" section
between cycles — the session log is the durable record of what was
done.

Example:

```
### C function extraction `[collection]`
- **Status:** not_started
- **Dependencies:** none
- **Description:** Extend the collector to extract C functions from framework
  headers. Currently only ObjC classes are extracted.
- **Results:** _pending_
```

### session-log.md

Append-only, timestamped entries. **Never read by any LLM phase.**
Human-facing audit trail only. The work phase writes to
`latest-session.md`; `run-plan.sh` appends it post-hoc. Format:

```
### Session N (YYYY-MM-DDTHH:MM:SSZ) — brief title
- What was attempted / what worked / what didn't
- Key learnings or discoveries
```

Timestamp is ISO 8601 UTC: `date -u '+%Y-%m-%dT%H:%M:%SZ'`.

### latest-session.md

The current session's entry. Written by the work phase, **overwriting**
any prior content. The reflect phase reads this file as its only session
input — this is how reflect is guaranteed to see only the latest entry
regardless of how long `session-log.md` grows.

`run-plan.sh` deletes `latest-session.md` before each work phase starts,
so a crashed work phase cannot leak stale content into the next reflect
phase.

### memory.md

Distilled learnings. Each entry is a markdown section with a heading.
Entries are updated, sharpened, merged, or removed as understanding
evolves — not an append-only log. Work reads it for context. Reflect
edits incrementally (may prune). Compact rewrites globally (lossless).

**Memory style rules** are in [`fixed-memory/memory-style.md`](fixed-memory/memory-style.md).

### phase.md

Single word: `work`, `reflect`, `compact`, or `triage`. Each phase
writes the next phase before stopping. Defaults to `work` if missing.
Reflect always writes `compact`; `run-plan.sh` may override to `triage`
if the compaction trigger has not fired.

### related-plans.md (optional)

Declares peer-project relationships for cross-plan propagation.
Two sections: **Parents** (projects this plan depends on) and
**Children** (projects that depend on this plan). Siblings are
auto-discovered from `$PROJECT/LLM_STATE/`. Format:

```markdown
# Related Plans

## Parents
- {{DEV_ROOT}}/Mnemosyne — orchestrator I integrate with

## Children
- {{DEV_ROOT}}/SomeApp — downstream consumer
```

`run-plan.sh` synthesizes a `{{RELATED_PLANS}}` block at composition
time and substitutes it into the work and triage phase prompts.

### pre-work.sh (optional)

An optional executable script invoked before every work phase. Used to
enforce invariants the work-phase LLM cannot reliably enforce itself
(e.g., regenerating stale artifacts, verifying a required service is
running). A non-zero exit aborts the cycle. Runs from the project root
after the defensive `rm -f latest-session.md` cleanup. Absent or
non-executable files are silently skipped.

```bash
#!/bin/sh
exec ./scripts/regenerate-stale-pipeline-artifacts
```

### prompt-*.md (optional)

Plan-specific overrides concatenated after the corresponding
`phases/<phase>.md`. Contents are overrides and extras only — no
boilerplate. Typical use: `prompt-work.md` for project-specific
commands and constraints. Do not list coding-style reads here — the
work phase pulls them from `fixed-memory/` just-in-time. Reflect,
compact, and triage overrides are almost always absent.

Path references use placeholders:

| Placeholder | Substituted with |
|---|---|
| `{{DEV_ROOT}}` | absolute path to the dev root |
| `{{PROJECT}}` | absolute path to the project root (where `.git` lives) |
| `{{PLAN}}` | absolute path to the plan directory |
| `{{RELATED_PLANS}}` | synthesized block of sibling/parent/child plan paths |

## Phase Cycle

Each cycle runs through three core phases and a conditional fourth
phase. The current phase is stored in `phase.md`. The shell script
reads this file, runs the appropriate phase, and loops. The work phase
is interactive — the user types `/exit` to advance. Reflect, compact,
and triage run headlessly and auto-exit when the LLM finishes. Ctrl+C
quits the cycle at any point.

The script exits cleanly if a phase fails to advance `phase.md` — this
is both the kill mechanism for the interactive work phase and the error
detection mechanism for the headless phases.

After each phase completes successfully (phase.md advanced), the script
auto-commits changes. The work phase commits the entire project
(`git add -A`); headless phases commit only the plan directory. This
creates a per-phase audit trail in the git history. Commits are skipped
when a phase produces no changes. The work phase prompt instructs the
LLM to review `.gitignore` before finishing, since `git add -A` will
capture anything not ignored.

### Phase 1: WORK (interactive)

Read the task backlog, distilled memory, and related-plans block. Pick
the best next task (with user input). Do the work. Record raw results.

Before launching pi, `run-plan.sh` deletes any stale
`latest-session.md` and runs `pre-work.sh` if present.

The work phase reads `backlog.md` and `memory.md` (not `session-log.md`
or `latest-session.md`), picks a task, implements it, verifies it, records
results in `backlog.md`, writes the session entry to `latest-session.md`,
writes `reflect` to `phase.md`, and stops.

After the work phase exits, `run-plan.sh` appends `latest-session.md` to
`session-log.md` (guarded on `phase.md` advancing).

### Phase 2: REFLECT (headless)

Read the latest session log entry with fresh eyes. Compare against
existing memory. Distill, sharpen, deduplicate, or prune.

Reads `latest-session.md`, `memory.md`, and
`fixed-memory/memory-style.md` (not `backlog.md` or `session-log.md`).
For each learning: add, sharpen, replace, or remove memory entries.
Prunes aggressively — memory contains only what is currently true.
Writes `compact` to `phase.md`. Stops.

### Phase 3: TRIAGE (headless)

Read the task backlog and distilled memory with fresh eyes. Adjust the
plan. Emit a structured propagation list for the driver to fan out
after this phase exits.

The triage phase:

1. Reads `backlog.md` and `memory.md`.
2. Consumes `{{RELATED_PLANS}}` for cross-plan context.
3. Reviews each task: relevance, priority, splitting.
4. Adds tasks from learnings, removes obsolete ones, reprioritizes.
5. Scans for embedded blockers and promotes them to top-level tasks.
6. For each related plan where learnings warrant propagation, writes
   one entry into `{{PLAN}}/propagation.out.yaml` — then stops.
   Triage does **not** read foreign backlogs or memories, and does
   **not** dispatch subagents.
7. Writes `work` to `phase.md`.
8. Stops.

#### Cross-plan propagation: externalized dispatch

Rather than invoking subagents from inside the triage phase (which ties
the design to a particular harness), triage writes a structured YAML
file and exits. `run-plan.sh` reads that file after triage finishes and
fans out one fresh pi process per target.

`propagation.out.yaml` format:

```yaml
propagations:
  - target: /absolute/path/to/related/plan
    kind: child         # or "parent" or "sibling"
    summary: |
      One to three paragraphs describing the learning and why it
      affects this target. The receiving pi process reads the
      target's backlog.md and memory.md and applies whatever
      updates are warranted.
```

Rules:
- Use absolute paths (the Related plans block already shows them).
- Use `|` (block scalar) for `summary` so multi-line text works.
- Omit the whole file if there are no propagations. An absent or empty
  `propagation.out.yaml` tells the driver there is nothing to fan out.

After all propagation pi processes complete, `run-plan.sh` deletes
`propagation.out.yaml`. This design is cleaner, inspectable (the YAML
is human-readable between triage exit and dispatch), and
harness-neutral — a future harness can replace pi's propagation
subprocess without touching triage logic.

### Phase 4: COMPACT (conditional, headless)

Rewrite `memory.md` losslessly in tighter form. Runs only when reflect-
phase growth exceeds the compaction headroom.

Reads `memory.md` and `fixed-memory/memory-style.md` only. Rewrites
`memory.md` in place preserving every live fact. Writes `triage` to
`phase.md`. Stops. Compact is **strictly lossless**; a bad compaction
is `git checkout memory.md` away.

#### Compaction trigger

Reflect always writes `compact` to `phase.md` as its nominal next phase.
After reflect exits, `run-plan.sh` checks whether `memory.md` has
actually grown beyond the compaction headroom. If **not**, the script
overrides `phase.md` from `compact` to `triage`, effectively skipping
compact for this cycle without invoking pi. If it **has**, `phase.md`
stays at `compact`.

The trigger is **relative, not absolute**: compact is skipped when
`wc -w memory.md <= baseline + HEADROOM`, where `baseline` is read from
`<plan>/compact-baseline` (a plain integer file) and `HEADROOM` is a
script-level constant (initially 1500 words). Relative threshold tracks
unreflected growth rather than absolute size.

**Bootstrap:** if `compact-baseline` does not exist, effective baseline
is 0, so any memory exceeding HEADROOM triggers compaction.

**Baseline update is guarded.** After compact exits, the script writes
`wc -w < memory.md` to `compact-baseline` only if the phase successfully
advanced `phase.md`. A crashed compact leaves `compact-baseline`
untouched; the next run retries.

## Files

- **README.md** — this file
- **create-plan.md** — instructions for creating a new backlog plan
- **run-plan.sh** — canonical shell driver for the four-phase cycle
- **config.sh** — per-phase model/thinking selection and HEADROOM
  (sourced by `run-plan.sh`); see "Configuration" above
- **prompts/system-prompt.md** — system prompt addendum injected into
  every phase via `--append-system-prompt`; covers fresh-context
  mandate, tool etiquette, path placeholder rules, and tone
- **prompts/memory-prompt.md** — auto-memory instructions injected into
  the work phase; documents memory types, save/access rules, and the
  index format
- **phases/work.md**, **phases/reflect.md**, **phases/compact.md**,
  **phases/triage.md** — shared operational phase prompts
- **fixed-memory/coding-style.md** — language-agnostic coding style rules
- **fixed-memory/coding-style-rust.md** — Rust-specific guidelines
- **fixed-memory/memory-style.md** — single source of truth for memory
  style rules applied by reflect and compact phases

## Tips

- **Don't over-plan upfront.** Start with the tasks you know about. More
  will emerge.
- **Split tasks that grow.** If a task turns out to be bigger than
  expected, split it rather than letting it sprawl.
- **Record why, not just what.** The session log should capture reasoning
  and surprises, not just "did X."
- **Blocked tasks are information.** When you mark a task blocked, the
  dependency tells future sessions what to unblock first.
- **Promote cross-cutting blockers to top-level tasks immediately.** When
  a task's description surfaces a blocker that affects more than just
  that task — a validation spike, a migration step, a shared dependency,
  a prerequisite investigation — lift it out into its own top-level
  backlog entry the moment it is identified.
- **Categories are lenses, not phases.** Use them to filter the backlog
  when deciding what to work on, not to enforce ordering.
- **Memory is not a log.** Prune aggressively. If something is no longer
  true or useful, remove it. The session log is the safety net.
- **One task per work phase.** Resist the urge to keep going. Fresh
  context for reflection is more valuable than momentum.
- **Inspect propagation.out.yaml before dispatch.** After triage exits
  and before the driver fans out pi processes, the file is human-
  readable. Review it if anything looks off — delete the file to cancel
  dispatch entirely.

## Successor

LLM_CONTEXT_PI is a pi-harness fork of
[LLM_CONTEXT](https://github.com/Linkuistics/LLM_CONTEXT), adapted to
use pi (`@mariozechner/pi-coding-agent`) in place of Claude Code. The
externalized propagation design, auto-memory system, and thinking-level
controls are pi-specific additions that diverge from the original.
The intent is eventual merge-back once the pi harness stabilises and
the additions have been validated in practice.

[Mnemosyne](https://github.com/Linkuistics/Mnemosyne) remains the
long-term successor to both: it reframes this system as a
harness-independent orchestrator and subsumes the backlog-plan format,
phase cycle, and fixed-memory model into a broader design. New
architectural work happens there; LLM_CONTEXT_PI remains in active use
until Mnemosyne is ready to replace it.
