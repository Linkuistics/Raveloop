# Backlog

## Tasks

### R1 — Implement structured `state backlog` verb surface + backlog-scoped `state migrate`

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

Execute the plan at `docs/structured-backlog-r1-plan.md`. Ships every
`state backlog <verb>` command (list/show/add/init/set-status/set-results/
set-handoff/clear-handoff/set-title/reorder/delete), the backlog-scoped
migration verb (`state migrate <plan-dir>`), and integration tests.

Plan is a 13-task TDD-by-task sequence: each task writes a failing test,
implements to green, then commits. Does not touch phase prompts — prompt
migration is R6.

See `docs/structured-plan-state-design.md` for Q1–Q8 design decisions that
govern this implementation.

**Results:**

**All 13 tasks complete in a single work phase.** Tasks 7–10 and Task 11
executed in parallel via two dispatched subagents; Tasks 12 (CLI wire-up)
and 13 (integration tests) completed in the main context afterwards.

**Final verification:** 281 lib tests + 27 legacy integration tests + 5
new `tests/state_backlog.rs` integration tests = **313 total, all green**.
`cargo build --release` clean under `warnings = "deny"`.

**Full per-task outcome:**

**Task 1 — Restructure `state.rs` into `state/` module:** Moved
`src/state.rs` verbatim to `src/state/phase.rs` (5 tests travelled with it;
the plan doc said "four", minor drift). Created `src/state/mod.rs` that
re-exports `phase::run_set_phase` and declares `backlog` + `migrate`
submodules. Dropped the old file.

**Task 2 — Schema types + slug generation:** `BacklogFile`, `Task`,
`Status` serde types in `src/state/backlog/schema.rs`; `slug_from_title`
and `allocate_id` (collision-suffixed, first attempt has no suffix, second
is `-2`, third `-3`, ...). `BacklogFile::extra` uses `#[serde(flatten)]`
over `IndexMap` so future R2+ top-level keys survive an R1 read/write
cycle. 6 tests.

**Task 3 — Atomic YAML read/write:** `src/state/backlog/yaml_io.rs`
with `backlog_path`, `read_backlog`, `write_backlog`. Tmp-file + rename
pattern mirrors `state::phase::atomic_write` (deliberately duplicated,
not hoisted — contextual error messages differ). Missing backlog.yaml
errors point at `state migrate`. Block-scalar emission for multi-line
descriptions pinned by a round-trip test. 3 tests.

**Task 4 — Markdown parser for migration:** `src/state/backlog/parse_md.rs`
strict-parses the legacy `backlog.md` format into `BacklogFile`.

  **Plan defect fixed:** the plan's `split_into_task_blocks` treated any
  `---` line as a task-block terminator — incorrect, because the
  `[HANDOFF]` convention prepends `---\n[HANDOFF]` *inside* a task block.
  The original splitter would close the block at that `---` and drop the
  handoff content, which `parses_task_with_results_and_handoff` caught
  immediately.

  Rewrote the splitter to boundary-only on `### ` headings (the single
  unambiguous start-of-task marker). A new `normalise_block` helper
  strips a truly-trailing `---` separator from the block's tail but
  leaves a mid-block `---\n[HANDOFF]` intact. Both memory contracts
  (`Task blocks delimited by \n---` and `[HANDOFF] integration tests
  guard promote-vs-archive cycle`) are now honoured.

  Live backlog test (`parses_live_core_backlog_without_error`) parses the
  real `LLM_STATE/core/backlog.md` and confirms feasibility on production
  data. 7 tests.

**Task 5 — `state backlog list` verb:** Pure `task_matches` predicate +
`ListFilter` struct with `status`, `category`, `ready`, `has_handoff`,
`missing_results` fields. `--ready` = `not_started AND every dep is done`.
`run_list` emits filtered YAML or JSON to stdout. 4 tests.

**Task 6 — `state backlog show` verb:** `find_task` centralises
id-lookup with a descriptive error (id echoed back for
self-correction); every subsequent mutation verb will reuse it.
`run_show` wraps the matched task in a `BacklogFile` projection for
shape consistency with `list`. 2 tests.

**Task 7 — `add` + `init` verbs** (subagent A): 4 new tests. `run_add`
validates dependency ids up-front to prevent dangling references; `run_init`
refuses a non-empty backlog.

**Task 8 — `set-status` + `set-results` verbs** (subagent A): 4 new tests.
`set-status` requires `--reason <text>` when setting `blocked`; clears
`blocked_reason` when flipping away from blocked.

**Task 9 — `set-handoff` + `clear-handoff` verbs** (subagent A): 2 new
tests. Typed `handoff:` field replaces the old `\n---\n[HANDOFF]` prose
convention.

**Task 10 — `set-title` + `reorder` + `delete` verbs** (subagent A): 5
new tests. `reorder` takes `before`/`after` + target id. `delete` refuses
when the task is a dependency of another unless `--force`.

**Task 11 — `state migrate` verb** (subagent B): 6 new tests.
`MigrateOptions { dry_run, original_policy: Keep|Delete, force }`.
Parse-first (a parse failure aborts before any write, so `--force` cannot
mask malformed input). Idempotent via structural equivalence check
(`backlogs_equivalent` ignores the `extra` IndexMap since a legacy-md
parse always leaves it empty). Validation round-trip re-reads the file
after write and asserts equivalence before any delete.

**Task 12 — Main.rs CLI dispatch wire-up** (main context):

  **Plan merge needed:** the plan predated R4's `state projects` verbs,
  so the "replace StateCommands" instruction couldn't be applied
  verbatim. Merged instead: preserved `SetPhase` and `Projects`
  variants, added `Backlog` and `Migrate`. Added `dispatch_state` +
  `dispatch_backlog` helper functions; moved the existing projects
  dispatch into `dispatch_state` so the single top-level
  `Commands::State` arm delegates everything through the helper.

  Added `parse_output_format` and `resolve_body` helpers.
  `resolve_body` consolidates the `--body-file <path>` vs
  `--body <value>` vs `--body -` (stdin) resolution that every
  body-accepting verb needs; library functions take already-resolved
  `&str` bodies so stdio-vs-file stays out of their signatures.

  **Attribute lift:** removed the scoped
  `#[allow(dead_code, unused_imports)]` from `src/state/mod.rs` on
  `pub mod backlog;` and `pub mod migrate;`, and dropped the
  `#[allow(unused_imports)]` on the `pub use migrate::{...};` line.

  **Follow-up dead-code trim:** post-wiring, four dead-code / unused-
  import errors surfaced in the bin build. All from plan code that
  exported items `main.rs` didn't end up calling. Removed: the
  `pub use migrate::{...}` re-export from `src/state/mod.rs` (main.rs
  uses `state::migrate::*` directly); the `Task`, `allocate_id`,
  `slug_from_title`, `backlog_path` re-exports from
  `src/state/backlog/mod.rs`; and the unused `Status::as_cli_str`
  method (the inverse of `Status::parse`, with no current caller).

**Task 13 — End-to-end CLI integration tests** (main context): Created
`tests/state_backlog.rs` with 5 tests shelling out via
`CARGO_BIN_EXE_ravel-lite` — migrate round-trip + `list --ready`, dry-run
no-write, idempotent re-run, parse-failure leaves filesystem untouched,
add→set-status→set-results→show round-trip through the `--body` resolver.
All pass.

**Execution strategy note:** Parallel subagents were a net win here — the
verb family (Tasks 7-10, all in `verbs.rs`) serialised inside one agent
while migrate (Task 11, independent file) ran concurrently. Task 12 had
to merge both agents' outputs with the existing projects-dispatch code,
plus trim the broader pub re-exports the plan assumed would all be used.

**Memory candidates for the reflect phase to consider:**

- Plan defect in `split_into_task_blocks`: using `---` as an
  unconditional task-block terminator silently drops `[HANDOFF]`
  content; boundary-split on `### ` headings is the correct rule.
- `R1 plan predates R4`: any plan-driven mod changes to `StateCommands`
  must be merged with R4's `Projects` variant, not replaced.
- Dual-crate dead-code lifecycle: new pub items in modules shared by
  both `lib.rs` and `main.rs` require either a caller in `main.rs` or
  a scoped `#[allow(dead_code)]`. The attribute is a temporary scaffold
  during staged rollouts, not a pattern to leave behind.

**Unblocks:** R2 (state memory), R3 (state session-log), R5 reuses
migrate patterns, R6 (phase-prompt migration to CLI verbs), and the
"move per-plan task-count extraction from LLM survey prompt into Rust"
task (the structured parser it needed now exists).

---

### R2 — Implement structured `state memory` verb surface + memory migration

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1 (establishes the schema / yaml_io / migrate patterns the memory submodule reuses)

**Description:**

Mirrors the R1 structure for `memory.yaml`. Extends `state migrate` to cover
`memory.md` → `memory.yaml`. CLI: `state memory list / show / add / delete`.

**Results:** _pending_

---

### R3 — Implement `state session-log` + `latest-session.yaml` + GitCommitWork rewire

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1

**Description:**

Adds `state session-log` verbs (list, show, append, set-latest, show-latest),
makes `latest-session.yaml` a typed file (same record shape as session-log
entries), rewires `phase_loop::GitCommitWork` to parse the new YAML + append
to `session-log.yaml`'s `sessions:` list with session-id idempotency. Extends
`state migrate` to cover session-log + latest-session.

**Results:** _pending_

---

### R5 — Implement global `state related-projects` edge list + `migrate-related-projects`

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R4 (done — catalog exists; names ↔ paths resolution is now available)

**Description:**

Global `../ravel-lite-config/related-projects.yaml` edge list (sibling /
parent-of), name-indexed, shareable between users. CLI: `state related-projects
list [--plan <path>]`, `add-edge`, `remove-edge`. `state migrate-related-projects
<plan-dir>` one-shot merges a plan's legacy `related-plans.md` into the global
file, creating it on first call and deduping by (kind, participants).

**Results:** _pending_

---

### Move per-plan task-count extraction from LLM survey prompt into Rust

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1 — requires the structured backlog parser R1 will land before task counts can be derived in Rust.

**Description:**

The survey LLM currently infers per-plan task counts from the raw markdown in
`backlog.md`. Once the structured backlog parser from R1 exists, task counts
(total, not_started, in_progress, done) can be computed directly in Rust and
injected as pre-populated tokens into the survey prompt — removing an
unnecessary inference burden from the LLM.

Do not schedule until R1 resolves; R1's completion is the trigger to revisit
scope here.

**Deliverables:**

1. Extend the structured backlog parser to expose a `task_counts() -> TaskCounts`
   method.
2. In `src/survey/discover.rs`, compute task counts from the parsed backlog
   and inject them into `PlanRow` (replacing the LLM-inferred field).
3. Update `defaults/survey.md` to remove the instruction asking the LLM
   to count tasks; add a note that counts are pre-populated.
4. Test: assert counts are correct for a plan with tasks in each status.

**Results:** _pending_

---

### R6 — Migrate all phase prompts to use CLI verbs

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1, R2, R3, R4, R5 (all verbs must exist before prompts can invoke them)

**Description:**

Replace direct `Read` / `Edit` of plan-state files with `ravel-lite state <verb>`
calls across `defaults/phases/work.md`, `analyse-work.md`, `reflect.md`,
`dream.md`, `triage.md`, `create-plan.md`, `defaults/survey.md`,
`defaults/survey-incremental.md`. ~5–15 instruction rewrites per file. Prompts
keep the `{{RELATED_PLANS}}` token (projection shape preserves plan paths).

**Results:** _pending_

---

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5

**Description:**

Feature design + implementation. Given a set of projects, dispatch LLM
subagents in parallel to analyse each project's README / backlog / memory and
propose sibling / parent-of edges. SHA-based cache (keyed on per-project
content hash) avoids re-analysing unchanged projects. Output merges into the
global `related-projects.yaml`.

Large — probably needs its own design-ish pass (brainstorm → spec → plan)
before implementation.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround once version exceeds 2.1.116

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`invoke_interactive` in `src/agent/claude_code.rs` passes
`--debug-file /tmp/claude-debug.log` as a workaround for a TUI
rendering failure in Claude Code ≤2.1.116. The root cause was not
found; debug mode happens to mask it via an unknown upstream mechanism.

When the installed `claude` binary is updated past 2.1.116, remove both
`args.push` lines adding `--debug-file` and `/tmp/claude-debug.log`.
Verify that the Work phase TUI renders correctly without the flag
before closing.

**Results:** _pending_

---
