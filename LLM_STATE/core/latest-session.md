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
