# Survey-Driven Multi-Plan Run Mode — Architectural Pivot

**Status:** Proposed
**Date:** 2026-04-21

## Context

Two paths for orchestrating work across multiple plans exist or are
proposed in the repo today:

1. **LLM-authored coordinator plans** (`core/backlog.md` task #1 —
   now superseded by this design). A coordinator is itself a plan
   with a specialised `prompt-work.md` that reads its `backlog.md`,
   picks a child plan to dispatch, and calls `ravel-lite state push-plan`
   to pivot. Routing intelligence lives in prompt-space.

2. **`ravel-lite survey`** — a one-shot, read-only CLI producing a
   markdown overview across plan roots. The structured data the LLM
   emits is parsed into a typed `SurveyResponse` and then rendered to
   markdown; the YAML itself is consumed and discarded.

The coordinator path has a latent gap: the `push-plan` CLI verb has
no in-repo caller shipped with defaults, so a user running
`ravel-lite create` cannot today produce a coordinator plan. Task #1
was set up to close that gap by extending `create-plan.md` with a
decomposition flow and an embedded coordinator-work boilerplate.

## Decision

Replace the coordinator-plan concept with a code-driven, survey-routed
multi-plan `run` mode. Routing intelligence moves from prompt-space
into Rust.

## Rationale

- Matches the project's "invariants in code" bias. `ravel-lite
  state set-phase` is a standing example: phase transitions moved
  from prompt-authored writes to a CLI verb once the validation
  surface was well-defined. Routing across N plans is the next
  such surface — inputs (survey output) and outputs (selected
  plan) are well-defined enough to encode in Rust directly,
  without ever passing through prompt-space.
- Eliminates three coupled failure modes a prompt-authored
  coordinator introduces: LLM drift on children enumeration, LLM
  drift on pivot mechanics, and the maintenance cost of a prompt
  surface whose invariants largely re-state what Rust enforces.
- Puts `ravel-lite survey` on the critical path. Survey was a
  diagnostic tool; turning it into orchestration infrastructure
  forces it to become incremental — otherwise per-cycle invocation
  cost scales with plan count.

## Design

The pivot decomposes into four items in a new plan at
`LLM_STATE/survey-restructure/`. Three of the items (5a → 5b → 5c)
form a linear dependency chain that builds the new routing
infrastructure; the fourth (5d) is an independent cleanup that
removes the old infrastructure and may run at any point in the
sequence.

### 5a — Structured YAML output for `survey`

**Scope:**

- Add `serde::Serialize` to `SurveyResponse` and its children.
  Emit canonical YAML via re-parse-and-serialize from the typed
  struct — every emission is proven parseable by round-tripping.
- Replace root-walked args with positional plan-directory args.
  `discover_plans`' `fs::read_dir` walk collapses into "load the
  exact plan named." Breaking change to the `survey` CLI surface.
- New subcommand `ravel-lite survey-format <file>`: reads a YAML
  survey, calls the existing `render_survey_output`, emits markdown
  to stdout. Separates persistence from presentation.
- Forward-compat field: add `input_hash: String` to `PlanRow`,
  computed over `phase.md` + `backlog.md` + `memory.md` +
  `related-plans.md` (explicitly NOT `session-log.md`, which is
  append-only and would defeat change detection). The hash is
  computed in Rust and **injected into each `PlanRow` after
  parsing the LLM's YAML response**, matched by project+plan
  identifier. The LLM never sees or handles the hash — echoing is
  mechanical work that belongs in code, not in a prompt round-trip.
  No prompt change is needed in 5a for the hash field.

**Approach:**

Re-emission over pass-through. Guarantees the emitted YAML
round-trips through the typed struct; two identical parses produce
byte-identical output.

`discover_plans` simplification erases the walk machinery. The
per-plan load logic (phase/backlog/memory reads,
`find_project_root`-based project-name derivation) stays, unwrapped
from the loop.

**Dependencies:** None.

### 5b — Incremental survey via `--prior`

**Scope:**

- `--prior <file>` flag parses a prior survey YAML, diffs per-plan
  `input_hash` against freshly-computed hashes, classifies each
  plan as unchanged / changed / removed / added.
- Delta-aware `render_survey_input`: only changed+added plans go
  into the LLM payload. The prior survey is carried in full as
  context so the LLM can revisit cross-plan blockers and parallel
  streams if the deltas affect them.
- Merge logic: LLM response carries only the delta set; `run_survey`
  composes the final `SurveyResponse` by merging unchanged rows
  from the prior with the LLM's delta. Validation refuses a delta
  that mutates a plan outside the declared changed set.
- `--force` bypass flag for debugging and schema-bump paths.

**Open decisions (settle at implementation):**

- **Prompt strategy.** Lean: two prompts — `defaults/survey.md`
  (cold) and `defaults/survey-incremental.md` (warm). Two small
  clear prompts beat one with conditional branches. Trade-off:
  duplication of the YAML schema description; mitigated by a
  shared boilerplate file if the duplication becomes load-bearing.
- **Schema version marker.** Include `schema_version: 1` at the
  top of emitted YAML. A struct-incompatible change bumps the
  marker; `--prior` with a mismatched version either fails fast
  (with a remediation hint) or auto-falls-back to `--force`.
- **Hash algorithm.** SHA-256 via the `sha2` crate. Blake3 is
  faster but adds a dependency for no measurable win on the input
  sizes involved.

**Dependencies:** 5a.

### 5c — Multi-plan `run` mode with survey-driven routing

**Scope:**

- `ravel-lite run` accepts N positional plan-dir args (today: 1).
  Single-plan invocation (`N == 1`) remains exactly as it is today
  — no survey, no routing, no state file. Multi-plan (`N > 1`)
  adds the survey-routed selection loop described below.
- New required flag when `N > 1`: `--survey-state <path>`. The
  survey result is the single piece of persistent data the
  multi-plan runner needs between cycles; making the path
  user-specified keeps it visible, inspectable, and diffable. The
  file is both output (written at cycle end) and input (read as
  `--prior` on the next cycle, via 5b's incremental-survey path).
  `--survey-state` is rejected when `N == 1` — it has no meaning
  without multiple plans.
- The multi-plan run loop's shape is: **survey → select → dispatch
  one cycle → repeat**. Survey is the first operation of the run
  loop — there is no separate cold-start branch. The survey
  invocation internally chooses cold vs incremental based on
  whether the `--survey-state` file already exists (pass it as
  `--prior` if yes; omit if no). Write the merged result back to
  the same path.
- Code-driven selection — not LLM-driven. Parse
  `recommended_invocation_order` from the (now-canonical) survey
  YAML and present top-ranked plans to the user.
- **Selection UI: minimal.** A plain stdout prompt listing the
  top-ranked plans with their ordinals, plan identifiers, and
  rationales; a single stdin read for the user's numeric choice.
  No ratatui widget. A richer TUI selection experience can be a
  separate enhancement later; the first multi-plan runner ships
  with the simplest UX that works.
- Dispatch means: run exactly one phase cycle of the selected plan
  (i.e. a single invocation of the existing `phase_loop` for that
  plan directory), then return to the top of the run loop to
  re-survey. One cycle per selection; the user re-picks every
  iteration.
- No coordinator-plan concept and no prompt surface for routing
  decisions. `stack.yaml` and `push-plan` — today's infrastructure
  for LLM-driven plan pivots — are removed outright in item 5d.

**Dependencies:** 5b.

### 5d — Remove `stack.yaml`, `push-plan` CLI, `pivot.rs`, and `run_stack`

**Scope:**

- Delete the `ravel-lite state push-plan` subcommand from
  `src/main.rs`; remove `run_push_plan` from `src/state.rs` along
  with its tests.
- Delete `src/pivot.rs` in its entirety: `validate_push`,
  `push_timestamp`, `decide_after_work`, `decide_after_cycle`, and
  the `Frame`/`Stack` types. If `push_timestamp`'s timestamp format
  is genuinely needed elsewhere after the deletion, extract it
  into a small utility module; otherwise delete it too.
- Collapse `run_stack` in `src/phase_loop.rs` back to a simple
  wrapper that loops `phase_loop` with the existing
  continue-or-exit user prompt. Rename appropriately (e.g.
  `run_single_plan`) — "stack" terminology is no longer meaningful.
- Remove all `stack.yaml` I/O: reads, writes, validation,
  sync-to-disk logic, the file-format parser.
- Remove tests for pivot state machines, stack serialisation, and
  push-plan validation.
- `LLM_STATE/core/memory.md` entries about stack/pivot behaviour
  (e.g., "run_stack replaces phase_loop as top-level entry point",
  "Pivot state machines are purely functional", "push_timestamp()
  in pivot.rs is canonical", "Pivot tests require all five phase
  configs seeded") become obsolete and should be pruned — but that
  is reflect/triage work consequent to the deletion, not part of
  5d's implementation.

**External impact:** the out-of-repo Ravel orchestrator at
`{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/` is the only
known consumer of `push-plan` and `stack.yaml`. Deleting them
breaks that consumer's current flow. The user accepts this as part
of the pivot; migration of that external project to the
survey-routed multi-plan `run` mode is separate from this pivot.

**Dependencies:** None. The code being deleted has no in-repo
caller (`core/backlog.md` task #1's supersession confirms this).
5d can run at any point relative to 5a/5b/5c. Recommended ordering:
do 5d before 5c, so 5c is built against a clean runner architecture
with no residual stack/frame logic to coexist with.

## Supersessions

- **`core/backlog.md` #1 (coordinator plans):** Status → `done`.
  Results block records that the LLM-authored coordinator concept
  has been superseded by this design. The infrastructure #1 was
  built on — `stack.yaml`, `push-plan`, `pivot.rs`, `run_stack` —
  is itself removed in item 5d.
- **`core/backlog.md` #5 (original incremental survey):** Status →
  `done`. Results block explains the split into items 5a/b/c/d in
  the new `survey-restructure` plan.

## Principle applied: do nothing in an LLM that code can do

The design deliberately keeps machine work in machine hands:

- `input_hash` is computed, stored, and read entirely in Rust. The
  LLM is not asked to echo it. Any mechanical value the code
  already knows should be injected post-parse, not round-tripped
  through a prompt.
- Delta classification (unchanged / changed / removed / added) in
  5b is a hash comparison — Rust.
- Merge of LLM delta with prior unchanged rows in 5b is a
  dictionary lookup — Rust.
- Selection in 5c is a list traversal of the parsed
  `recommended_invocation_order` — Rust.

The LLM's role is narrow and reasoning-only: infer cross-plan
blockers, group parallel streams, produce a recommended invocation
order, and write per-plan notes. Everything else is code.

### Candidate for a future follow-on

The LLM currently computes per-plan task counts (`unblocked`,
`blocked`, `done`, `received`) by reading each backlog and tallying
`Status:` lines. This is mechanical and deterministic — a clear
candidate for moving into Rust. It is deliberately not in this
pivot's scope because it depends on a robust backlog parser, which
is exactly what `core/backlog.md` task #3 (research: expose
plan-state markdown as structured data) evaluates. Once #3 settles,
a follow-on task can move counts from LLM to Rust using whatever
parser #3 recommends.

## Out of scope

- Archival of the existing out-of-repo `ravel-orchestrator`
  coordinator prompt. That was always a reference example; its
  fate is orthogonal. Migrating it off the removed `push-plan` API
  is a separate concern for that project.
- Changes to the analyse-work / reflect / triage / dream phase
  prompts. Routing lives in the runner, not in per-phase prompts.
- A "decomposer" CLI that takes a user goal and emits N child
  plans. Separate design if it becomes useful later.
- Moving task-count computation from LLM to Rust — see
  "Candidate for a future follow-on" above.
