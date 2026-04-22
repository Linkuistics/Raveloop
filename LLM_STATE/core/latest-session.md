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
