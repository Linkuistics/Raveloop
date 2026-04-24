# Creating a Multi-Session Plan

Instructions for creating a new backlog plan through conversation with
the user. The output is a plan directory conforming to the format in
`README.md`.

## Process

### 0. Invariant: this session produces a plan

Your ONLY output from this session is a plan directory. Whatever the
user describes in their first turn is plan scope — not a task for you
to execute in-session.

- If the description is a **concrete problem** (a bug report, a
  specific feature, a single question), treat it as the plan's
  initial task. Capture it as one backlog entry and scaffold the
  plan around it. A single-task plan is a valid plan.
- If the description is **abstract** (a broad goal, a multi-part
  initiative), run the clarification dialogue in §1 until the scope
  is clear enough to seed initial tasks.

Do NOT attempt to do the work the user describes (e.g. fix the bug,
implement the feature, answer the question). Your job is to write
plan files, not to solve the problem described. When in doubt — even
when the user seems to be asking for the work directly — the right
response is "I'll capture that as the first task in a plan at
`<target>`; what other tasks belong alongside it?".

### 1. Clarify the scope

Ask the user:
- What is this plan for? What's the overall goal?
- What do you already know needs doing? (Initial tasks)
- What categories make sense for grouping the tasks?
- Are there known dependencies or blockers?
- Does this plan have known peer-project relationships? (Parents —
  projects it depends on; Children — projects that depend on it.
  Siblings within the same project are auto-discovered and don't
  need to be declared.)
- Any specific instructions for the work phase?
  (e.g., test commands, key constraints, domain-specific guidance)

This is a conversation — ask follow-up questions until the scope is
clear. Don't ask all questions at once; one at a time.

### 2. Draft the plan

The plan directory has already been scaffolded by `ravel-lite create`
before this session started. The following files exist and are empty
but valid:

- `{{PLAN}}/phase.md` containing exactly `work`
- `{{PLAN}}/backlog.yaml` containing `tasks: []`
- `{{PLAN}}/memory.yaml` containing `entries: []`
- `{{PLAN}}/dream-baseline` containing `0`

**Do not overwrite these files directly.** Populate them through the
state CLI only. This keeps the typed YAML as the source of truth from
the first commit and prevents format drift.

**backlog** — for each initial task the user described, run:

```bash
ravel-lite state backlog add {{PLAN}} \
  --title "<task title>" \
  --category <category> \
  --description-file <path-to-description.md>
```

Pass `--dependencies <id1,id2>` when a task depends on another. Write
the description body to a temp file; do not try to pass a multi-line
description inline.

**memory** — rarely needed for a new plan. If the user has surfaced
standing facts worth capturing up front (coding conventions, invariants,
prior-incident context), add them via:

```bash
ravel-lite state memory add {{PLAN}} \
  --title "<heading>" \
  --body-file <path-to-body.md>
```

**session-log.yaml** — does not need explicit initialisation; the first
`session-log append` creates it at the end of the first work cycle.

**related-plans.md** (optional) — only if the user declared peer-project
relationships. The file is read at phase-loop entry to seed the
`{{RELATED_PLANS}}` macro for the work-phase prompt; the global
component graph at `<config-dir>/related-components.yaml` is populated
separately by `ravel-lite state related-components discover --apply`,
not from this prose. Example body:

```markdown
# Related Plans

## Parents
Peer projects this plan depends on:
- {{DEV_ROOT}}/Ravel — orchestrator I integrate with

## Children
Peer projects that depend on this plan:
- {{DEV_ROOT}}/SomeApp — downstream consumer
```

**prompt-work.md** (optional) — only if the plan has work-phase-specific
content that isn't in the shared `phases/work.md`. Typical contents:

```markdown
Key commands:
- cargo test --workspace — run all tests
- cargo clippy --workspace — lint
- cargo +nightly fmt — format

Constraints:
- TDD: write tests first
- thiserror for library errors, anyhow for CLI
- No unwrap/expect in production code
```

**Do NOT list `fixed-memory/coding-style*.md` reads here.** The shared
`phases/work.md` instructs the work-phase agent to consult
`fixed-memory/coding-style.md` plus any matching
`fixed-memory/coding-style-<lang>.md` just-in-time when it is about
to write or modify code — there is nothing for the plan to declare.

**prompt-reflect.md, prompt-dream.md, prompt-triage.md** — almost
always **absent**. The shared phase files cover everything these phases
need for most plans. Only create them if the plan has truly unique
reflect/dream/triage content. The triage phase emits a subagent dispatch
YAML rather than dispatching subagents inline, so there is nothing
plan-specific to override.

**pre-work.sh** (optional) — only if the plan has an invariant the
work-phase agent cannot reliably enforce itself. See README.md §pre-work.sh
for the contract.

### Driving the cycle

The cycle is driven by the `ravel-lite` Rust binary. Each
user-configured profile lives in its own directory, scaffolded once
with `ravel-lite init <config-dir>`. Day-to-day usage points the
binary at that config directory via the discovery chain (`--config`
flag, then `RAVEL_LITE_CONFIG` env var, then the default location at
`<dirs::config_dir()>/ravel-lite/`):

```bash
# Most common: set once, forget
export RAVEL_LITE_CONFIG=<config-dir>
ravel-lite run ~/Development/{project}/LLM_STATE/{plan-name}

# Explicit per-invocation
ravel-lite run --config <config-dir> ~/Development/{project}/LLM_STATE/{plan-name}
```

The agent (Claude Code or Pi) is selected by the `agent:` key in
`<config-dir>/config.yaml`, not by a CLI flag. Switching agents means
either editing that key or pointing `--config` (or `RAVEL_LITE_CONFIG`)
at a different config directory.

The binary walks up from the plan directory to find the project root
(`.git`), composes each phase's prompt from the config directory's
`phases/<phase>.md` plus the optional `<plan>/prompt-<phase>.md`,
substitutes tokens, and invokes the selected agent.

### Path placeholders

Prompt files and `related-plans.md` MUST use these placeholders for any
path reference. The orchestrator substitutes them with absolute paths
before passing content to the agent.

| Placeholder | Substituted with | Example |
|---|---|---|
| `{{DEV_ROOT}}` | absolute path to the dev root | `/Users/antony/Development` |
| `{{PROJECT}}` | absolute path to the project root | `/Users/antony/Development/Project` |
| `{{PLAN}}` | absolute path to the plan directory | `/Users/antony/Development/Project/LLM_STATE/plan-name` |
| `{{ORCHESTRATOR}}` | absolute path to this project's root | `/Users/antony/Development/Ravel-Lite` |
| `{{RELATED_PLANS}}` | synthesized block of sibling/parent/child plan paths (computed) | (multi-line block) |
| `{{TOOL_READ}}` | agent-specific tool name for reading files | `Read` or `read` |

`{{RELATED_PLANS}}` is substituted only in files read by the orchestrator
via composition (i.e., the shared `phases/*.md` files and any
`prompt-*.md` overrides). Do NOT use it in `related-plans.md` itself —
that file is the input to the synthesis.

**Never** use relative paths like `../Ravel-Lite/...` in prompts or
related-plans.md. Relative paths are interpreted differently depending
on the agent's cwd resolution and tend to break for nested plans.

### 3. Review with the user

Show the draft backlog (run `ravel-lite state backlog list {{PLAN}}`
after seeding) and any optional files (`related-plans.md`,
`prompt-work.md`, `pre-work.sh`) and ask if they look right. Adjust as
needed via `ravel-lite state backlog set-title`, `set-status`,
`delete`, or `add`.

### 4. Write the files

Save to `LLM_STATE/` in a descriptively-named directory. The directory
name should make the plan's purpose obvious.

Good: `LLM_STATE/core/`, `LLM_STATE/targets/racket-oo/`,
`LLM_STATE/vision-pipeline/`

Avoid: `LLM_STATE/plan/`, `LLM_STATE/todo/`

### 5. Commit

Commit the new plan directory.

## Reference

See `README.md` for the phase cycle overview, and
`defaults/phases/*.md` for the shared phase prompts that consume
these plan files.
