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
