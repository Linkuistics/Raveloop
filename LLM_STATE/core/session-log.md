# Session Log

### Session 1 (2026-04-21T08:03:01Z) — Runner-owned dream-baseline seeding and build metadata

- **What was attempted:** Three-layer self-healing for `dream-baseline`; build metadata in `--version`/`version`; `cargo-release` workflow; README Releasing section.
- **What worked:** All deliverables shipped. `seed_dream_baseline_if_missing` changed from "seed to current word count" to "seed to 0" — eliminating the bootstrap delay on pre-existing plans whose baseline had drifted above threshold. Seed now called from three layers: `run_create` (post-session scaffolding), `run_set_phase` (every LLM phase transition including coordinators), and `GitCommitReflect` (original layer). `build.rs` emits `BUILD_TIMESTAMP`, `GIT_DESCRIBE`, `GIT_SHA`; `main.rs` concatenates them into a `VERSION` constant for both `--version` and the `version` subcommand. `release.toml` configures `cargo-release` with `publish=false`, `push=false`. Removed dream-baseline authorship prose from `defaults/create-plan.md`. 44 tests pass, clippy clean. Also reset `LLM_STATE/core/dream-baseline` from 1019 → 0.
- **What to try next:** Ravel's coordinator plans (`ravel-orchestrator`, `sub-D-coordination`) will auto-heal missing `dream-baseline` on next `ravel-lite state set-phase` call after binary reinstall. Initial `v0.1.0` tag not yet created — `cargo release patch --execute` or manual `git tag -a v0.1.0` seeds it.
- **Key learnings:** Seeding to 0 ("never dreamed") rather than current word count is the correct sentinel — seeding to current count silently delays the first dream by `headroom` words on populated plans. Three-layer approach ensures no single unreachable code path can leave a plan without a baseline.

### Session 2 (2026-04-21T09:52:55Z) — pre-reflect gate removal, dirty-tree narrowing, retired-path pruning, hand-off convention

- Worked four backlog tasks to completion in a single session: (1) collapse the pre-reflect gate, (2) narrow the dirty-tree warning, (3) prune stale `skills/` paths, (4) preserve hand-offs across the analyse-work → triage boundary.
- All four tasks shipped with tests; the suite was green at end of session.
- The gate removal exposed two test failures: the `pi_phase_cycle` fake-pi script looped because it always wrote `git-commit-work` regardless of phase (fixed with a phase-aware case statement), and `pivot_run_stack_short_circuit_pivot` errored on missing `reflect.md` config (fixed by seeding all five phase configs).
- `ContractMockAgent::invoke_headless` for `Triage` was changed from overwrite to append so the safety-net test can observe analyse-work's status flips after the full cycle completes.
- The hand-off convention (analyse-work.md + triage.md) is now live in shipped prompts; the next real session that produces a hand-off is the first end-to-end exercise.
- No implementation work began on the two larger tasks still `not_started`: coordinator-plan creation and the structured-state research task.

### Session 3 (2026-04-21T12:53:55Z) — survey-restructure sub-plan close-out: 5a–5d delivered

- Ran the survey-restructure sub-plan through four full work cycles, delivering tasks 5a, 5b, 5c, and 5d as distinct commits against main.
- **5a** (`5711fac`): Structured YAML output for `ravel-lite survey`; `survey-format` subcommand for human rendering; `input_hash` field seeded in Rust post-parse. `src/survey/schema.rs` gained `Serialize` derives and `schema_version` marker.
- **5b** (`5e295f4`): Incremental survey via `--prior` and `--force`. New `src/survey/delta.rs` owns hash-comparison and delta-merge logic. `src/survey/invoke.rs` refactored into `compute_survey_response` (in-memory) + `run_survey` (CLI wrapper). `defaults/survey-incremental.md` added as the delta-path prompt template.
- **5c** (`fdaeb02`): Multi-plan `run` mode with survey-driven routing. New `src/multi_plan.rs` (539 lines) implements `build_plan_dir_map`, `options_from_response`, `select_plan_interactive`, and `run_multi_plan`. `ravel-lite run` now accepts `1..N` plan dirs; `--survey-state` required for N > 1. Design rationale captured in `docs/survey-pivot-design.md`.
- **5d** (`06ce874`): Removed `src/pivot.rs`, `push-plan` CLI verb, `run_stack` logic, and `stack.yaml` infrastructure. `src/state.rs` trimmed from ~230 to ~80 lines. `src/phase_loop.rs` de-pivoted.
- Sub-plan close-out triage (`19ad808`, `7735d3f`): propagated results to core backlog and closed the survey-restructure plan. Deleted `LLM_STATE/survey-restructure/` directory in source commit `080c9e6`.
- `tests/integration.rs` overhauled throughout (1197 lines changed) to cover the new survey/incremental/multi-plan paths and remove obsolete pivot/stack tests.

What worked: linear dependency chain 5a → 5b → 5c held; sub-plan broke work into per-cycle chunks that each compiled and tested green before committing.

What this suggests trying next: the "Migrate Ravel orchestrator off removed push-plan verb" task is now unblocked and urgent — the orchestrator will break on next invocation. The structured-data research task (backlog CLI verbs) is next highest value but not urgent.

### Session 4 (2026-04-21T23:13:58Z) — Scope git queries to subtree root for monorepo support

- Implemented the "Make git operations subtree-scoped so ravel-lite can run inside a monorepo" backlog task in full.
- Replaced `find_project_root` (`.git`-walkup) with `project_root_for_plan` — pure path derivation `<plan>/../..`, no disk walk, decoupled from `.git` location.
- Added `-- <project_dir>` pathspec to all three git query functions: `working_tree_status`, `paths_changed_since_baseline`, and `work_tree_snapshot`. `git_commit_plan` intentionally left unchanged (its `git add .` at `plan_dir` CWD is already scoped to plan-state files).
- Updated all four callers: `src/main.rs`, `src/multi_plan.rs`, `src/agent/common.rs`, `src/survey/discover.rs`.
- Added a monorepo scoping integration test (`git_queries_are_scoped_to_subtree_in_monorepo`) that synthesises an outer repo with a sibling subtree and asserts all three query functions see only the ravel-lite subtree's changes.
- Added three `project_root_for_plan` unit tests: correct derivation, shallow-path error, non-existent-path ok (pure math).
- Updated five integration tests in `tests/integration.rs` and one in `src/multi_plan.rs` to use the three-level `<project>/LLM_STATE/<plan>` layout matching ravel-lite convention. Removed the obsolete `load_plan_errors_when_no_git_above_plan` test — the invariant no longer holds under path-math derivation.
- Added README "Project layout" section documenting `<project>/<state-dir>/<plan>` convention and a "Monorepo subtrees" subsection covering pathspec scoping semantics and commit-message-prefix answer.
- All 215 lib tests + 23 integration tests pass. Task marked `done` in backlog with full results, open design-question answers, and verification record.
- What this suggests next: the "Research: expose plan-state markdown as structured data" task is now unblocked and is the natural next candidate — it depends on nothing else and its completion unblocks the task-count extraction task.

### Session 5 (2026-04-21T23:38:12Z) — Continuation-line rendering for dream/triage output

- Implemented `→ …` continuation-line support in `format_result_text` (`src/format.rs`): lines matching `^\s*→\s*(.*)` immediately after an action marker are re-indented to the detail column and styled with the preceding action's intent. Blank lines, insight blocks, and all other non-continuation lines clear the association.
- Added `PROMOTED` and `ARCHIVED` action tags to `ACTION_INTENTS` for triage hand-off markers that emit new backlog tasks or memory entries.
- Updated `defaults/phases/dream.md` output-format spec to describe the new two-line entry layout (label + `→` continuation) so the dream LLM emits output the renderer can align.
- Updated `defaults/phases/work.md` step 10 to allow multiple tasks per session when the user explicitly requests them, while preserving the single-task-per-phase default.
- Five tests added to `src/format.rs`: `PROMOTED`/`ARCHIVED` recognition, continuation alignment, intent inheritance, orphan-arrow fallthrough, and blank-line chain-breaking.
- The triage phase (run before this work session) deleted two tasks: the `done` monorepo subtree-scoping task (cleaned up) and the `not_started` Ravel orchestrator migration task (dropped).

What worked: the `last_action_intent: Option<Option<Intent>>` state variable cleanly threads the preceding action's intent through to continuation lines without adding a new pass over the text. The double-Option encodes "no prior action" (outer None) vs "prior action with no intent" (Some(None)) unambiguously.

What to try next: run the updated dream phase on a real plan to confirm the two-line entries render as intended in the TUI.

### Session 6 (2026-04-21T23:54:09Z) — add integration tests for [HANDOFF] convention

- Implemented two new integration tests (`handoff_marker_in_analyse_work_is_promoted_by_triage` and `handoff_marker_in_analyse_work_is_archived_by_triage`) covering the full analyse-work → git-commit-work → reflect → git-commit-reflect → triage → git-commit-triage cycle for `[HANDOFF]` marker handling.
- Extended `ContractMockAgent` with an opt-in `handoff_injection: Option<HandoffInjection>` field and `with_handoff_injection()` builder. The injection simulates: (a) analyse-work's fallback path appending a `[HANDOFF] <title>\n<body>` block to a completing task's Results block in `backlog.md` and mirroring it into `latest-session.md` under `## Hand-offs`; (b) triage's mining step — scans done tasks, extracts markers via `extract_handoff_from_block()`, and either promotes to a new `not_started` backlog task or archives to `memory.md` per the `HandoffDisposition` field.
- Added two helper functions at module scope: `inject_handoff_into_task_block` (analyse-work side) and `extract_handoff_from_block` (triage side), both splitting on the `\n---` block separator convention.
- Fixed two pre-existing clippy lints: six `doc_lazy_continuation` violations in `src/survey/schema.rs` (resolved by splitting the `input_hash` doc into paragraphs separated by blank `///` lines) and one `useless_format` in `tests/integration.rs:352` (replaced `format!(...)` with `.to_string()`).
- All 25 integration tests and 220 unit tests pass; `cargo clippy --all-targets -- -D warnings` is clean.
- The one existing struct-literal `ContractMockAgent` call site in `analyse_work_receives_snapshot_and_commits_uncommitted_source` gained an explicit `handoff_injection: None` field to preserve struct-exhaustiveness.
- What worked: splitting the mock into analyse-work and triage arms with clearly separated responsibilities made it easy to verify ordering (safety-net flip before injection), mining (block-level scan), and disposal (done task deletion) independently. Pinning the promote-vs-archive judgement in the injection struct kept tests deterministic without hardcoding LLM reasoning.
- What this suggests next: the two tests are green against the current prompts, so the `[HANDOFF]` convention is now CI-protected. If a real session surfaces a multi-block or nested-code-block hand-off body, widen the helpers then. Clippy is clean under `-D warnings`; a future maintenance task could add a CI gate to keep it that way.

### Session 7 (2026-04-22T02:15:48Z) — Structured plan-state design and R1 implementation plan

- **Attempted:** Complete the "Research: expose plan-state markdown as structured data via `ravel-lite state <file> <verb>` CLI" task — answer Q1–Q8 with a design decision and deliver the prototype PoC deliverable. Mid-session the user requested an upgrade from prototype PoC to a full production R1 implementation plan.
- **What worked:** Brainstorming skill drove the design iteratively (one question at a time), catching four key design moves that would have been missed in a single sketch: projects-not-plans for the global edge list, name-indexed shareable edge list, migration simplification (no cross-plan orchestrator), and latest-session as structured YAML. An Explore agent audit against the proposed verb surface found two real gaps (`backlog init`, `session-log set-latest`/`show-latest`) and correctly rejected two false positives.
- **What didn't:** Initial related-plans design was per-plan vertex-centric; took three clarifying exchanges to land on global-edge-list-by-name — could have been caught earlier by asking "should this be shareable?" up front. First migration section was too thin; user had to flag the need for a complete tool (atomicity, idempotency, dry-run, validation round-trip) before the full contract was written.
- **Deliverables landed:** `docs/structured-plan-state-design.md` (full design doc, Q1–Q8) and `docs/structured-backlog-r1-plan.md` (13-task TDD-by-task R1 plan covering full `state backlog` verb surface + backlog-scoped migrate + integration tests). No Rust code shipped.
- **What this suggests next:** Triage should promote the R1–R7 hand-offs from the research task's Results block into concrete backlog tasks. R1 is immediately actionable (plan already written at `docs/structured-backlog-r1-plan.md`); R4 is also unblocked. R2, R3, R5, R6 depend on R1. R7 is large and needs its own design pass.

## Hand-offs

### R1–R7 rollout tasks (promote from research task Results [HANDOFF] block)

- Problem: The GO decision on structured plan-state requires seven follow-on implementation tasks (R1–R7) to be tracked as concrete backlog entries.
- The full specifications for each task are inlined in the research task's `[HANDOFF]` block in `backlog.md` (Results section, after the `---` separator). Triage should mine and promote all seven as `not_started` tasks with the dependencies and descriptions already specified there.
- R1 has an existing implementation plan at `docs/structured-backlog-r1-plan.md`; its task description should reference this file.
- Dependencies: R2, R3 depend on R1; R5 depends on R4; R6 depends on R1–R5; R7 depends on R5. R1 and R4 are immediately ready.

### Session 8 (2026-04-22T03:44:10Z) — Implement state projects catalog (R4)

- Implemented R4: `state projects` catalog mapping project names to absolute paths
- Created `src/projects.rs` with `ProjectsCatalog` struct (schema_version: 1, projects list), atomic save, `auto_add` pure logic returning `AlreadyCatalogued`/`Added`/`NameCollision`, and `ensure_in_catalog_interactive` generic over `Read + Write`
- Wired CLI verbs `list`/`add`/`remove`/`rename` in `main.rs` under `StateCommands::Projects`
- Added `register_projects_from_plan_dirs` in `main.rs`, called before TUI startup in `Commands::Run`, so collision prompts reach a real tty before Ratatui's alternate-screen takeover
- `add` rejects relative paths (catalog is path-anchored; relative paths resolve differently from different CWDs)
- `rename` is scoped to catalog only — R5 adds the `related-projects.yaml` cascade
- 18 unit tests in module + 2 CLI integration tests (round-trip add→list→rename→remove; relative-path rejection); full suite 238+27 green; clippy clean
- All changes committed; R4 status already correctly marked `done` in backlog

### Session 9 (2026-04-22T05:07:12Z) — Implement R1 structured backlog verb surface

- Attempted and completed all 13 tasks from `docs/structured-backlog-r1-plan.md` in a single work phase.
- Tasks 7–10 (backlog CRUD verbs) and Task 11 (migrate) ran concurrently via dispatched subagents; Tasks 12 (CLI wire-up) and 13 (integration tests) ran in the main context.
- `src/state.rs` was restructured into `src/state/` with `phase.rs`, `backlog/` (schema, yaml_io, parse_md, verbs, mod), and `migrate.rs` submodules.
- All 11 backlog verbs and the `state migrate` verb were wired into `main.rs` alongside existing `SetPhase` and `Projects` dispatch via new `dispatch_state`/`dispatch_backlog` helpers.
- 313 total tests pass (281 lib + 27 legacy integration + 5 new `tests/state_backlog.rs`). Clean release build under `warnings = "deny"`.

What worked:
- Parallel subagents for the verb family vs migrate (independent files, no merge conflicts).
- Parse-first migration with idempotency via structural equivalence check.
- `find_task` centralising id-lookup reused by every mutation verb.

What didn't / surprises:
- Plan defect: `split_into_task_blocks` used `---` as a task-block terminator, which incorrectly split `[HANDOFF]` blocks. Fixed by boundary-splitting on `### ` headings instead.
- Plan predated R4's `Projects` variant in `StateCommands`; merge was required, not a straight replace.
- Post-wiring dead-code cleanup: four items exported by library modules but not referenced by `main.rs` had to be removed.

What this suggests next:
- R2 (state memory) can start immediately — it mirrors the R1 structure exactly and all the patterns (schema, yaml_io, migrate) are now established.
- The "Move per-plan task-count extraction from LLM into Rust" task is now unblocked; the structured parser exists.

### Session 10 (2026-04-22T06:14:36Z) — Implement state memory verb surface (R2)

- Implemented `src/state/memory/` module with `schema.rs` (`MemoryFile { entries: Vec<MemoryEntry { id, title, body }> }` + `#[serde(flatten)] extra`), `yaml_io.rs` (atomic temp-file rename), `parse_md.rs` (strict `^## ` heading splitter, errors on empty-body entries), `verbs.rs` (list/show/add/init/set-body/set-title/delete). `allocate_id` and `slug_from_title` reused from `state::backlog::schema`.
- Refactored `migrate.rs` from a flat single-path function into a two-phase planner: `plan_backlog_migration` and `plan_memory_migration` each return `Option<PendingMigration>`; the top-level `run_migrate` collects both, errors if the set is empty, then writes all targets only after all parses succeed. Parse failure on either file aborts before any disk write.
- Wired `MemoryCommands` enum and `dispatch_memory` through `main.rs`; `parse_memory_format` mirrors `parse_output_format`.
- Added 4 end-to-end CLI integration tests in `tests/state_memory.rs` and 9 lib unit tests in `state::migrate` (both files, idempotency, force, parse-failure atomicity, empty-plan error). Total suite: 342 tests, 0 failures.
- `cargo run -- state migrate LLM_STATE/core --dry-run` reports 7 records (backlog) + 63 records (memory) — the live core plan migrates cleanly.
- R2 task was already marked `done` in backlog.md with a full Results block; no safety-net flip required.

What worked:
- The R1 module pattern (schema / yaml_io / parse_md / verbs) transferred directly to memory with minimal adaptation.
- Two-phase planner (`plan_*` → `PendingMigration` enum) cleanly separates parse from write; extending to R3 session-log adds a third variant with no structural change.

What this suggests next:
- R3 (`state session-log`) slots straight in: add `plan_session_log_migration` returning a `PendingMigration::SessionLog` variant; the parse-all-then-write-all contract extends without surgery.

### Session 11 (2026-04-22T06:46:49Z) — session-log YAML verbs + Rust task-count injection

- Implemented R3: `src/state/session_log/` module (schema.rs, yaml_io.rs, parse_md.rs, verbs.rs, mod.rs) providing `SessionRecord` / `SessionLogFile` types, id-based idempotent `append_record`, and full CLI verb surface (`state session-log list/show/append/set-latest/show-latest`).
- Rewired `phase_loop::append_session_log` to use `session_log::append_latest_to_log`; missing `latest-session.yaml` is a graceful no-op; crash-retry idempotency via session id (strictly stronger than prior tail-string check).
- Extended `state migrate` with two new `PendingMigration` variants (`SessionLog`, `LatestSession`) via `plan_session_log_migration` / `plan_latest_session_migration`; parse-all-then-write-all invariant preserved. Dry-run against live `LLM_STATE/core/` parsed 7 backlog + 65 memory + 10 sessions + 1 latest cleanly.
- Implemented "Move per-plan task-count extraction from LLM survey prompt into Rust": added `TaskCounts { total, not_started, in_progress, done, blocked }` to `state::backlog::schema` with `BacklogFile::task_counts()`; wired through `PlanSnapshot.task_counts`, `PlanRow.task_counts`, `inject_task_counts` in `survey/schema.rs`, and both cold and incremental survey invoke paths via `collect_task_counts`. Both `defaults/survey.md` and `defaults/survey-incremental.md` updated to forbid LLM from emitting `task_counts`.
- Fixed pre-existing `iter_cloned_collect` clippy lint in `backlog/parse_md.rs:227` (replaced with `body_lines.join("\n")`).
- All 347 lib + 27 integration + 13 CLI tests pass; `clippy --all-targets -- -D warnings` clean.

What worked: additive `task_counts` field on `PlanRow` (rather than replacing LLM-inferred `unblocked`/`blocked`/`done`) preserved backward compatibility — downstream renderers migrate at their leisure. Session-id idempotency for `append_record` is cleaner than tail-string check.

What didn't: no issues encountered; both features shipped as designed.

Suggests next: R5 (global `state related-projects` edge list) is the next unblocked task. Manual migration of `LLM_STATE/core/{session-log,latest-session}.{md→yaml}` is a 1-command step (`ravel-lite state migrate`) safe to run any time.

### Session 12 (2026-04-22T07:14:29Z) — Migrate plan-state files to YAML

- Ran `ravel-lite state migrate` against `LLM_STATE/core/` to convert all
  markdown plan-state files to structured YAML (`backlog.md → backlog.yaml`,
  `memory.md → memory.yaml`, `session-log.md → session-log.yaml`,
  `latest-session.md → latest-session.yaml`).
- Dry-run confirmed identical parse counts before committing: 6 backlog, 68
  memory, 11 sessions, 1 latest. Round-tripped each area via `state <area>
  list` / `show-latest` to verify end-to-end integrity.
- Created annotated tag `pre-structured-state` at `8ce34ba` and pushed it plus
  41 pending commits to `origin` before writing any YAML.
- Backlog reorganised: completed tasks archived, R7 split into R7-design (new
  research task) + R7 implementation (now depends on R7-design), one stale
  task removed.

What worked: `state migrate` parsed all production markdown cleanly. The
`--keep-originals` default left `.md` files in place, which is the correct
choice given that phase prompts still read/write `.md` directly.

What didn't / caveat: the YAML files are **preview data, not operational data**
until R6 lands. Phase prompts continue to mutate the `.md` files; the `.yaml`
files are frozen at migration-time content and will diverge. Triage should
annotate R6's description: either re-migrate immediately before the prompt
rewrite or run `state migrate --force --delete-originals` atomically as part
of R6.

Also noted: the task's deliverable documented `--plan-dir` as a named flag, but
the real CLI takes `PLAN_DIR` as a positional argument — `--plan-dir` fails.
Docs should be corrected.

What to try next: R5 (global `related-projects` edge list) unblocked; R6
(migrate phase prompts to CLI verbs) waiting on R1–R5.

No uncommitted source-file paths. All changes are plan-state files inside
`LLM_STATE/core/` reserved for the plan-state commit.

### Session 13 (2026-04-22T07:39:48Z) — Implement R5: global related-projects edge list

- Implemented `src/related_projects.rs` (1 030 lines): global `related-projects.yaml`
  edge list with `sibling` / `parent-of` kinds, direction-aware canonical dedup,
  `Edge::validate` (rejects self-loops, non-pair participant counts), full CRUD,
  and `run_migrate_related_projects` that parses legacy `related-plans.md` and
  merges derived edges idempotently.
- Extended `src/main.rs` (+122 lines) with two new `StateCommands` variants:
  `RelatedProjects { command }` (subcommand tree) and `MigrateRelatedProjects`.
  `dispatch_related_projects` and `print_migration_report` handle dispatch and output.
- `src/lib.rs` gained the `pub mod related_projects` declaration.
- 27 unit tests inside the module + 4 end-to-end integration tests in
  `tests/state_related_projects.rs` (using `CARGO_BIN_EXE_ravel-lite`).
  All 418 cargo tests pass; `cargo clippy --all-targets` is clean.
- Deliberately left unchanged (by design, R5 scope): phase prompts still read
  `related-plans.md`; `projects::run_rename` does not cascade into the yaml file.
  Both are R6 territory.

What worked: parse-all-then-write-all strategy (catalog and edge file only touched
after every peer resolves) means the command is atomic with respect to partial
failures. Sibling dedup uses sorted participants (order-insensitive); parent-of
keeps participant order as part of the identity — this distinction is load-bearing
for correctness.

What didn't: nothing significant. R5 was completed in one session as designed.

What to try next: R6 (migrate all phase prompts to CLI verbs) is the highest-leverage
follow-up; it closes the .md/.yaml divergence. R7-design is also unblocked.

### Session 14 (2026-04-22T08:13:12Z) — R6: migrate phase prompts to CLI verbs + run_rename cascade

- **What was attempted:** Complete R6 — rewrite all phase prompts to use `ravel-lite state <verb>` calls instead of direct Read/Edit of plan-state files, and wire `projects::run_rename` cascade into `related-projects.yaml` (an R6 sub-deliverable listed in the backlog description).
- **What worked:** All six phase/create-plan files successfully migrated (`work.md`, `analyse-work.md`, `reflect.md`, `dream.md`, `triage.md`, `create-plan.md`). The `{{TOOL_READ}}` token was correctly scoped to `work.md` only — removing it from headless phases (analyse-work, reflect, dream) fixed six integration-test failures that would have blocked CI. Four TDD tests for `rename_project_in_edges` all green; full suite (422 tests) passes.
- **Scope deviation — deliberate:** The backlog told us to pass `--delete-originals` at cutover; we did not. A grep found four Rust readers of `.md` plan-state files (`src/dream.rs`, `src/survey/discover.rs`, `src/multi_plan.rs`, `src/main.rs`). Deleting `.md` would silently regress those subsystems. The clean fix is R8 (added to backlog this session): migrate the Rust readers, then delete originals atomically.
- **What this suggests next:** R8 should also synthesize the `{{RELATED_PLANS}}` block from the global `related-projects.yaml` instead of per-plan `related-plans.md`, completing the edge-store consolidation R5 started.

### Session 15 (2026-04-22T08:46:52Z) — R8: Rust plan-state readers migrated from .md to .yaml

- **Attempted:** Migrate all four Rust readers of legacy `.md` plan-state files to the typed YAML API so `ravel-lite state migrate --delete-originals` becomes safe to run.
- **What worked:**
  - `src/dream.rs`: `memory.md` word-count read replaced with `state::memory::read_memory`; word count now sums titles + bodies across parsed entries. New regression test `update_baseline_counts_entry_titles_and_bodies` pins the double-field contract.
  - `src/survey/discover.rs`: `backlog.md` and `memory.md` reads replaced with `read_backlog` / `read_memory`; `task_counts` now derived from the already-parsed `BacklogFile`, eliminating the redundant `parse_backlog_markdown` round-trip. `PlanSnapshot::backlog` / `.memory` now carry re-serialised YAML strings.
  - `src/survey/compose.rs`: section headers updated from `backlog.md`/`memory.md` to `backlog.yaml`/`memory.yaml`; content wrapped in ```yaml fences so the LLM receives structured input with explicit framing.
  - `defaults/survey.md` + `defaults/survey-incremental.md`: prose updated to match new YAML-labelled payload and code-fence wrapping.
  - `src/related_projects.rs`: added `read_related_plans_markdown(plan_dir)` — the single canonical access point for plan-local `related-plans.md` prose. `src/main.rs` and `src/multi_plan.rs` both route through it (was duplicated `fs::read_to_string(...).unwrap_or_default()`). Future swap to rendering from `related-projects.yaml` is now a one-function change.
  - `tests/integration.rs`: added `write_memory_yaml_with_word_count` and `write_backlog_yaml_with_marker` helpers; migrated dream and survey integration tests from raw `.md` writes to typed-YAML writes via the state API.
  - All tests pass: 379 unit + 27 integration + 17 state-focused = 423 total.
- **Scope narrowing (deliberate):** `src/multi_plan.rs` and `src/main.rs` only read `related-plans.md` (not deleted by `--delete-originals`) and check `phase.md` existence (not a migration target). Neither breaks when originals are deleted; they were consolidated behind `read_related_plans_markdown` for canonical-path hygiene, not for correctness.
- **What this suggests next:** Run `ravel-lite state migrate --delete-originals` against `LLM_STATE/core` to retire the legacy `.md` files and verify the full cycle. The remaining `related-plans.md`-to-`related-projects.yaml` rendering swap is a future task.
- **Key learnings:** Swapping `backlog.md` content for YAML-serialised `backlog.yaml` changes the LLM payload layout — visible to the LLM on first run (one-time behavioural shift). Existing `dream-baseline` values differ slightly under the new counting rule (titles + bodies, no markdown punctuation); `update_dream_baseline` self-corrects after the first successful dream.
