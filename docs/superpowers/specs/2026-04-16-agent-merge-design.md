# Agent Merge Design Spec

Merge LLM_CONTEXT (Claude Code) and LLM_CONTEXT_PI (Pi) into a single TypeScript project supporting both agents. This repo (LLM_CONTEXT_PI) becomes the unified project. The original LLM_CONTEXT remains untouched as a working fallback.

## Motivation

Both projects implement the same four-phase backlog-driven development cycle but diverge on agent harness. Maintaining two forks is unsustainable. Additionally:

- Pi lacks subagent and skills support that Claude Code has via superpowers
- Claude Code's built-in system prompt and auto-memory are opaque and can change — Pi's explicit prompt injection is preferable but only possible for Pi
- The bash orchestrator (`run-plan.sh`) is hitting complexity limits with agent profiles, token substitution, YAML parsing, and conditional prompt injection

## Key Decisions

1. **TypeScript rewrite** — bash is outgrowing its welcome. Pi is TypeScript-native, has an SDK, and its extension system is TypeScript. Real types, real YAML parsing, real test framework.

2. **Shared parameterized phase files** — one set of phase markdown files with `{{TOOL_*}}` tokens resolved per-agent. No duplication.

3. **Prompt injection is agent-specific** — Pi gets custom `system-prompt.md` and `memory-prompt.md` injected (transparent, version-controlled). Claude Code relies on its opaque built-in behavior (accepted trade-off since the whole Claude Code path is already opaque).

4. **YAML-manifest subagent dispatch for triage** — triage writes `subagent-dispatch.yaml`, the orchestrator parses and dispatches. No inline subagent dispatch during triage.

5. **Native subagent mechanisms** — Pi uses its native subagent extension with custom agent definitions. Claude Code uses its native Agent tool. Phase files describe subagent needs generically.

6. **Skills as harness-independent markdown** — superpowers content extracted into `skills/`. Pi maps these to subagent agent definitions. Claude Code continues using its native plugin system.

7. **This repo is the target** — `../LLM_CONTEXT` is not touched. No migration strategy needed.

## Phase Chain

Incorporates all recent changes from `../LLM_CONTEXT`:

```
work → analyse-work → git-commit-work → [continue?] →
reflect → git-commit-reflect → [dream trigger?] →
dream → git-commit-dream →
triage → git-commit-triage → [continue?] → work
```

- `analyse-work`: headless LLM phase examining git diff to produce authoritative session log and commit message (replaces work phase self-reporting)
- `dream`: renamed from `compact` — lossless memory compaction, triggered by word-growth exceeding HEADROOM
- `git-commit-*`: script-managed phases for per-phase audit trail
- `[continue?]`: user control points for clean exit

## Project Structure

```
LLM_CONTEXT_PI/
├── src/
│   ├── index.ts                    # CLI entry point (--agent claude-code|pi)
│   ├── phase-loop.ts               # Core phase cycle orchestrator
│   ├── prompt-composer.ts          # Load phase files, substitute tokens, compose prompts
│   ├── subagent-dispatch.ts         # Parse subagent-dispatch.yaml, dispatch to targets
│   ├── dream.ts                    # Dream trigger check (word count vs baseline + headroom)
│   ├── session-log.ts              # Manage latest-session.md
│   ├── git.ts                      # Per-phase git commit logic
│   ├── config.ts                   # Typed config loading (shared + agent-specific)
│   ├── types.ts                    # Shared types and enums
│   │
│   └── agents/
│       ├── agent.ts                # Agent interface definition
│       ├── claude-code/
│       │   ├── index.ts            # ClaudeCodeAgent implements Agent
│       │   ├── stream-parser.ts    # Parse claude's JSON stream output
│       │   └── config.ts           # Default config (models)
│       └── pi/
│           ├── index.ts            # PiAgent implements Agent
│           ├── stream-parser.ts    # Parse pi's JSONL output
│           ├── config.ts           # Default config (models, thinking, provider)
│           └── setup.ts            # Auto-install subagent extension, generate agent defs
│
├── phases/                         # Shared, parameterized markdown
│   ├── work.md
│   ├── analyse-work.md
│   ├── reflect.md
│   ├── dream.md
│   └── triage.md
│
├── agents/                         # Agent-specific resources (not code)
│   ├── claude-code/
│   │   └── tokens.yaml             # { TOOL_READ: "Read", TOOL_WRITE: "Write", ... }
│   └── pi/
│       ├── tokens.yaml             # { TOOL_READ: "read", TOOL_WRITE: "write", ... }
│       └── prompts/
│           ├── system-prompt.md
│           └── memory-prompt.md
│
├── skills/                         # Extracted superpowers (harness-independent markdown)
│   ├── brainstorming.md
│   ├── writing-plans.md
│   ├── tdd.md
│   ├── systematic-debugging.md
│   └── ...
│
├── fixed-memory/
│   ├── coding-style.md
│   ├── coding-style-rust.md
│   └── memory-style.md
│
├── test/                           # vitest
├── package.json
├── tsconfig.json
├── create-plan.md
├── README.md
└── LICENSE
```

## Agent Interface

```typescript
enum LLMPhase {
  Work = 'work',
  AnalyseWork = 'analyse-work',
  Reflect = 'reflect',
  Dream = 'dream',
  Triage = 'triage',
}

enum ScriptPhase {
  GitCommitWork = 'git-commit-work',
  GitCommitReflect = 'git-commit-reflect',
  GitCommitDream = 'git-commit-dream',
  GitCommitTriage = 'git-commit-triage',
}

type Phase = LLMPhase | ScriptPhase

interface PlanContext {
  planDir: string
  projectDir: string
  devRoot: string
  relatedPlans: string
}

interface AgentConfig {
  models: Record<LLMPhase, string>
  thinking?: Record<LLMPhase, string>   // Pi only
  provider?: string                      // Pi only
}

interface SharedConfig {
  headroom: number
  agent: string
}

interface Agent {
  /** Launch interactive session (work phase) */
  invokeInteractive(prompt: string, ctx: PlanContext): Promise<void>

  /** Launch headless session (analyse-work, reflect, dream, triage) */
  invokeHeadless(prompt: string, ctx: PlanContext): Promise<string>

  /** Dispatch a subagent to a target plan (triage cross-plan updates) */
  dispatchSubagent(prompt: string, targetPlan: string): Promise<string>

  /** Agent-specific token mappings */
  tokens(): Record<string, string>
}
```

## Phase Loop

```typescript
async function phaseLoop(agent: Agent, plan: PlanContext, config: SharedConfig) {
  while (true) {
    const phase = readPhase(plan)

    if (isScriptPhase(phase)) {
      await handleScriptPhase(phase, plan)
    } else if (phase === LLMPhase.Work) {
      await agent.invokeInteractive(composePrompt(phase, plan, agent), plan)
    } else {
      await agent.invokeHeadless(composePrompt(phase, plan, agent), plan)
    }

    if (phase === ScriptPhase.GitCommitReflect) {
      if (!shouldDream(plan, config.headroom)) {
        writePhase(plan, ScriptPhase.GitCommitDream)
      }
    }
  }
}
```

Script phases handle:
- `git-commit-*`: read `commit-message.md` (if exists), commit plan directory changes
- After `git-commit-work` and `git-commit-triage`: prompt user to continue or exit

## Token Substitution

Phase files use these placeholders, resolved at prompt composition time:

### Path tokens (shared)
- `{{DEV_ROOT}}` — absolute path to dev root
- `{{PROJECT}}` — absolute path to project root (.git)
- `{{PLAN}}` — absolute path to plan directory
- `{{RELATED_PLANS}}` — synthesized block of related plan paths
- `{{ORCHESTRATOR}}` — absolute path to this project's root (directory-name-independent)

### Tool tokens (per-agent via tokens.yaml)
- `{{TOOL_READ}}` — Read / read
- `{{TOOL_WRITE}}` — Write / write
- `{{TOOL_EDIT}}` — Edit / edit
- `{{TOOL_GREP}}` — Grep / grep
- `{{TOOL_GLOB}}` — Glob / find
- `{{TOOL_BASH}}` — Bash / bash
- `{{TOOL_LS}}` — LS / ls

## Prompt Injection

### Pi
`PiAgent.invokeInteractive()` and `invokeHeadless()` append:
- `agents/pi/prompts/system-prompt.md` — fresh-context mandate, tool etiquette, verification discipline
- `agents/pi/prompts/memory-prompt.md` — auto-memory instructions (work phase only, read-only index for headless phases)

These are version-controlled, visible, and under our control.

### Claude Code
`ClaudeCodeAgent` does not inject custom prompts. Relies on Claude Code's built-in system prompt and auto-memory. Accepted trade-off — the entire Claude Code path is already opaque.

## Subagent Mechanism

### Pi
Uses Pi's native subagent extension. The orchestrator **automatically handles all Pi setup** — no manual Pi configuration required.

**Automatic setup (`src/agents/pi/setup.ts`):**

1. **Subagent extension install** (one-time, idempotent):
   - Check if `@mjakl/pi-subagent` is already installed (inspect `~/.pi/agent/settings.json`)
   - If not, run `pi install npm:@mjakl/pi-subagent` — this registers the `subagent` tool that Pi's LLM can invoke
   - This is a global install (Pi's extension system requires it) but only runs once

2. **Agent definition generation** (every run):
   - Read `skills/*.md` from this project
   - For each skill, generate a Pi agent definition (markdown with YAML frontmatter)
   - Write to `{projectDir}/.pi/agents/` — project-local, so they don't pollute the user's global Pi config
   - `.pi/agents/` is gitignored in the target project (generated artifact)
   - Pi's subagent extension discovers them automatically from the project directory

3. **Prerequisites check**:
   - Verify `pi` binary exists on PATH
   - Verify `ANTHROPIC_API_KEY` is set (or appropriate provider key)
   - Fail fast with clear error messages if anything is missing

**Generated agent definition example** (from `skills/brainstorming.md`):
```markdown
---
name: brainstormer
description: Explores ideas and designs through collaborative dialogue
tools: [read, grep, find, ls, bash]
model: claude-sonnet-4-6
---

[content from skills/brainstorming.md]
```

Agent definitions support single, parallel (up to 8 concurrent), and chain execution modes. The work phase can dispatch research and design subagents during brainstorming.

**Skill metadata**: Each `skills/*.md` file includes YAML frontmatter with the fields Pi needs (name, description, tools, model). The orchestrator reads this frontmatter and passes the content through to the generated agent definition. This keeps skills as the single source of truth.

### Claude Code
Uses native `Agent` tool. Phase files describe subagent needs generically — Claude Code interprets and dispatches using its built-in mechanism.

**Prerequisites check**: Verify `claude` binary exists on PATH. No additional setup needed — Claude Code handles its own configuration.

### Triage subagent dispatch (both agents)
Triage writes `subagent-dispatch.yaml`:
```yaml
dispatches:
  - target: /absolute/path/to/related/plan
    kind: child
    summary: |
      One to three paragraphs describing the learnings to apply.
```

The orchestrator parses this after triage exits and dispatches via `agent.dispatchSubagent()` for each entry.

## Skills

Superpowers skill content extracted into `skills/` as plain markdown files. Each skill is a self-contained workflow description.

### Pi consumption
The orchestrator generates Pi agent definitions from skills at startup (see Subagent Mechanism above). Each skill's frontmatter provides the Pi-compatible metadata (name, tools, model). During work phase brainstorming, Pi dispatches a `brainstormer` subagent whose system prompt is the extracted skill content.

### Claude Code consumption
Claude Code continues using its native superpowers plugin. The `skills/` directory serves as the canonical source of truth — the superpowers plugin could eventually read from here.

### Skills to extract (initial set)
- `brainstorming.md` — collaborative idea exploration and design
- `writing-plans.md` — implementation plan creation
- `tdd.md` — test-driven development workflow
- `systematic-debugging.md` — structured bug investigation
- `verification-before-completion.md` — evidence-before-assertions discipline

## Configuration

### Shared (`config.yaml` at project root)
```yaml
headroom: 1500
agent: claude-code  # default agent; CLI --agent flag overrides this
```

### Per-agent (`agents/<name>/config.yaml`)
```yaml
# agents/claude-code/config.yaml
models:
  work: ""                    # harness default (Opus)
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6

# agents/pi/config.yaml
provider: anthropic
models:
  work: claude-opus-4-6
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6
thinking:
  work: medium
  analyse-work: ""
  reflect: ""
  dream: ""
  triage: ""
```

## Implementation Stages

### Stage 1: Core orchestrator
TypeScript project setup (package.json, tsconfig.json, vitest). Phase loop, prompt composition, token substitution. Both agent profiles (ClaudeCodeAgent, PiAgent). Parameterized phase files incorporating latest changes from `../LLM_CONTEXT` (analyse-work, dream, git-commit phases, continue prompts).

Exit criteria: `npx llm-context --agent pi <plan>` and `npx llm-context --agent claude-code <plan>` both run the full phase chain.

### Stage 2: Triage subagent dispatch
YAML-manifest subagent dispatch with typed parsing. Triage phase file updated for YAML output. Dispatch via `agent.dispatchSubagent()`.

Exit criteria: triage writes `subagent-dispatch.yaml`, orchestrator parses and dispatches.

### Stage 3: Pi subagent integration
Automatic Pi agent definition generation from `skills/` at startup. Skills include YAML frontmatter with Pi-compatible metadata. Work phase instructions updated for subagent dispatch. No manual Pi configuration required.

Exit criteria: running with `--agent pi` auto-generates `.pi/agents/` in the target project; during work phase, subagents can be dispatched for research and brainstorming.

### Stage 4: Skills extraction
Extract superpowers skill content into `skills/`. Map to Pi agent definitions. Work phase instructions reference skills generically.

Exit criteria: Pi can invoke brainstorming, writing-plans, and TDD workflows via subagent extension.

### Stage 5: Testing
Vitest suite: prompt composition, token substitution, subagent dispatch parsing, dream trigger, config loading. Integration tests with dry-run phase cycles.

Exit criteria: `npm test` passes. Both agent paths verified.
