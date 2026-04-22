# Backlog

## Tasks

### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`cargo clippy --all-targets -- -D warnings` is now clean (exit 0) as of the
`[HANDOFF]` integration test task. Two pre-existing lints (`doc_lazy_continuation`
in `src/survey/schema.rs` and `useless_format` in `tests/integration.rs`) were
fixed as part of that work. Currently no CI step asserts clippy cleanliness, so
drift can re-accumulate silently.

Add a clippy gate to the CI pipeline (likely `.github/workflows/ci.yml` or
equivalent) that runs `cargo clippy --all-targets -- -D warnings` and fails
the build on any new lint. Verify the gate passes against current `main` before
merging.

**Results:** _pending_

---

### Research: expose plan-state markdown as structured data via `ravel-lite state <file> <verb>` CLI

**Category:** `research`
**Status:** `done`
**Dependencies:** `ravel-lite state` subcommand (✓ done — establishes the `state` namespace and the "CLI verb replaces direct file edit" pattern)

**Description:**

Investigate whether extending the `ravel-lite state` namespace with
structured read/write verbs over the plan's markdown surfaces —
`backlog.md` first, candidates also `memory.md`, `session-log.md`,
`related-plans.md`, `subagent-dispatch.yaml` — would meaningfully reduce
LLM context / tool-call cost and improve data-discipline, or whether
the benefits don't justify the added schema surface. Deliverable is a
design decision (go / no-go, with scope), not an implementation.

**Precedent the idea builds on.** Two verbs already convert free-form
file edits into typed CLI calls: `ravel-lite state set-phase` (replaces
`Write "reflect" to phase.md`) and `ravel-lite state push-plan`
(replaces the prose case-analysis for `stack.yaml`). Both landed
because (a) the target had a small well-defined schema, and (b)
enforcing invariants in Rust was tractable and the invariants were
load-bearing. The question this task asks is whether that pattern
scales up to larger, looser surfaces like `backlog.md`.

**Concrete shape of the proposal being evaluated:**

```
ravel-lite state backlog list [--status <s>] [--category <c>] [--ready] [--format json|table]
ravel-lite state backlog show <id>
ravel-lite state backlog add --title <t> --category <c> [--dependencies <d,d>] [--description-file <path>]
ravel-lite state backlog set-status <id> <status>
ravel-lite state backlog set-results <id> <results-file>
ravel-lite state backlog delete <id>
ravel-lite state backlog reorder <id> <before|after> <target-id>

ravel-lite state memory list [--format json]
ravel-lite state memory add --title <t> --body-file <path>
ravel-lite state memory delete <id>

ravel-lite state session-log append --session-file <path>

ravel-lite state related-plans list [--kind parent|child]
```

`<id>` could be the task title (slugified) or an ordinal; the research
should settle which.

**Potential benefits (hypotheses to test, not established):**

1. **Context reduction.** An LLM in the work phase currently reads
   all of `backlog.md` to pick a task. Today's file is ~450 lines;
   `backlog list --status not_started` would return maybe 20 lines.
   Measurable win if the savings are consistent across plan sizes.
2. **Tool-call reduction.** Current mutate pattern is Read + Edit
   (Edit requires exact-string match, often preceded by a probe Read
   to find the anchor). `set-status <id> done` collapses both sides.
   Analyse-work's safety-net step — find tasks with non-empty Results
   and stale Status and flip each — becomes a single command.
3. **Schema enforcement.** Writing invalid status values (`"pending"`
   when the vocabulary is `not_started / in_progress / done /
   blocked`) becomes a parse error, not silent drift. Catches
   mistakes the current prompts' prose guidance does not.
4. **Atomic mutations.** No TOCTOU window between Read and Write.
   Multiple prompts co-editing the same file in rapid succession
   (rare today, possible with parallel subagents tomorrow) stop
   racing.
5. **Typed queries unlock tooling.** `ready` = "status=not_started
   AND dependencies are all done" is currently a prose rule the LLM
   applies by reading. A CLI verb could expose it as a query —
   triage and work both win.

**Tradeoffs / risks to evaluate:**

1. **"All state is a readable file" principle (README §Principles).**
   Today a user can open `backlog.md` in their editor and edit
   anything. A CLI-emitted file is still readable, but hand-edits
   that break the schema (even minor: a missing blank line between
   fields) become errors on the next CLI read. The fix is a
   permissive parser that canonicalises on write — but that's its
   own drift surface.
2. **Free-form description authorship is awkward as CLI args.**
   Task descriptions are multi-paragraph markdown with code blocks,
   headings, and tables. `--description-file <path>` works but
   reintroduces the file-edit loop. Evaluate whether the LLM's
   actual authoring patterns fit a CLI.
3. **Schema migration cost.** The backlog's current schema has
   grown organically (Category / Status / Dependencies / Description
   / Results). Formalising it freezes today's shape. Adding a field
   later requires either schema versioning or a parser permissive
   enough to ignore unknown fields.
4. **Parser complexity.** Markdown is easy to emit, hard to parse
   consistently. Either adopt an explicit structured sidecar
   (backlog.yaml alongside backlog.md, regenerated on write), or
   constrain the markdown to a strict subset and write a dedicated
   parser. Sidecar loses the "single readable file" property;
   dedicated parser is a maintenance burden.
5. **Partial adoption creates drift.** If only some phase prompts
   use the CLI and others keep writing markdown directly, the two
   paths must stay consistent. Requires either an all-at-once prompt
   migration or a long coexistence period with both paths tested.
6. **Opacity for the LLM in reasoning tasks.** `list --status open`
   is great for selection. But triage explicitly *reasons over* the
   full backlog to detect buried blockers (triage.md:46-50). A
   structured list would need to preserve enough narrative per task
   (or stream full descriptions on demand) for that reasoning to
   still work — else triage quality regresses.

**Research questions the design must answer:**

- **Q1 — Authoritative format.** Markdown-as-source-of-truth (CLI
  parses + rewrites) vs structured sidecar (markdown is a rendered
  view) vs canonical markdown with a strict grammar. Recommendation
  with justification required.
- **Q2 — Which files qualify?** Backlog is the strongest candidate.
  Memory is semi-structured (`##` heading + prose body). Session-log
  is append-only. Related-plans is a categorised path list.
  Rank by benefit/cost and propose an incremental rollout order.
- **Q3 — Scope of the `list` query DSL.** Must cover: open,
  by status, by category, by dependency-readiness, by
  missing-results (analyse-work's safety-net), by age. Bikeshed-prone
  — settle the minimum useful set.
- **Q4 — Output formats.** `--format table` for humans, `--format
  json` for LLMs? Or markdown? Prompts currently consume markdown
  natively; JSON changes the reasoning surface.
- **Q5 — Identity.** Slug from title, stable ordinal, UUID? Titles
  change; ordinals shift on delete; UUIDs are LLM-unfriendly. Trade-off.
- **Q6 — Results-block authorship.** The most-edited piece of a
  backlog task is the Results block, which is often a 20-100 line
  markdown document with code blocks and insight. The CLI's story
  for this has to be clear: does the LLM write a file and invoke
  `set-results <id> <file>`, stream on stdin, or stay on Read+Edit
  for this field only?
- **Q7 — User hand-edit compatibility.** How permissive is the
  parser on read? What happens if a user adds a new field like
  `**Priority:** high`? Preserve-and-pass-through, error, or
  silently drop?
- **Q8 — Migration path.** If the answer is "go," how do existing
  plans migrate? One-shot reformat command? Gradual (CLI writes
  canonical, reads permissive, files converge over time)?

**Evaluation criteria (for deciding go / partial / no-go):**

- **Context savings** estimated per phase (work, analyse-work,
  triage) — rough token-count delta for a representative plan.
- **Tool-call delta** per phase — how many Read/Edit calls removed.
- **Invariant coverage** — what classes of silent drift (invalid
  status, missing Results, dangling dependencies) become enforced
  errors.
- **Implementation cost** — rough LOC estimate for parser +
  emitter + CLI verbs for the recommended scope.
- **Prompt-update cost** — how many of the 5 shipped phase prompts
  and `create-plan.md` need revision.
- **Principle cost** — does the preferred design still satisfy
  "All config, prompts, phase state, and memory are readable files
  on disk" from README §Principles? If not, by how much?

**Deliverables (of this research task):**

1. A design doc (markdown, committed to `docs/` or in-session) that
   answers Q1-Q8 with explicit decisions and rationale.
2. Recommended rollout: either one or more concrete follow-on
   backlog tasks (sized for individual work phases), or a
   documented "no-go" with justification.
3. If the recommendation is go: a prototype proof-of-concept for
   `backlog list --status not_started --format json` that parses
   the current `LLM_STATE/core/backlog.md` without data loss, to
   validate the parser-feasibility assumption before committing to
   a full rollout.

**Out of scope of this research task:**

- Implementation of the full CLI. This task is research +
  prototype only.
- Changes to any phase prompt. Prompt migration is a downstream
  task that only happens if the research concludes go.
- Changes to agent-config files (`config.yaml`, `tokens.yaml`) or
  the `survey.md` / `create-plan.md` prompts. Those aren't
  plan-state and aren't part of the hypothesis.
- Any stack-coordinator infrastructure — `stack.yaml`, `push-plan`,
  `pivot.rs`, `run_stack` have all been removed from the codebase;
  no longer a consideration.
- Any file under `fixed-memory/` — those are static documentation,
  not plan state.

**Related context:**

- Memory entry `Phase prompts invoke 'ravel-lite state set-phase'`
  records the convention this task generalises.
- The "Preserve hand-off rationale" task (now done) means
  Q6 can rely on the `[HANDOFF]` convention in Results blocks.
  The research question is narrower as a result: the Results block
  authorship path only needs to support the now-stable convention.
- Once this task settles, it unblocks "Move per-plan task-count extraction
  from LLM survey prompt into Rust" (see task below).

**Results:**

**Recommendation: GO.** Design decision reached; detailed design doc and
R1 implementation plan both delivered. Did not ship code in this phase —
upgraded the original "prototype PoC" deliverable to a full production
implementation plan at user request mid-session.

**Deliverables landed:**

1. **Design doc:** `docs/structured-plan-state-design.md` — answers the
   original Q1–Q8 plus the mid-session reframes. Key decisions:
   - **Reframe:** plan-state files are structured YAML with markdown as
     block-scalar string content. LLMs never read/write directly; only
     through `ravel-lite state` verbs. Resolves Q1 (authoritative format)
     toward structured-native.
   - **Scope (C):** all plan-state files — per-plan (backlog, memory,
     session-log, latest-session, phase) and global (related-projects,
     projects catalog).
   - **Storage:** YAML, one file per type, serde-backed schemas. Multi-
     line markdown bodies live in `|` block scalars (readable on `cat`,
     not escaped).
   - **Identity:** slug-from-title assigned at creation, persisted as a
     field, stable across title edits. Collision → `-2`/`-3` suffix.
   - **Output:** YAML default, `--format json` on request.
   - **Body input:** inline scalars + `--body-file <path>` + `--body -`
     stdin (all three supported per verb where a body is needed).
   - **Hand-edit:** strict parse, canonical write. No comment
     preservation (rejected as needless engineering).
   - **Migration:** single-plan-scoped `state migrate <plan-dir>` +
     `state migrate-related-projects <plan-dir>`. No cross-plan
     orchestrator (simplified mid-design). Behaviour contract:
     atomicity, idempotency, validation round-trip, dry-run, force,
     keep-originals (default) vs delete-originals.
   - **Projects (global):** relationships are between **projects** (stable)
     not plans (ephemeral). Global edge list is **shareable between users**
     (names-only) via a per-user catalog that maps names to absolute paths.
     LLM-driven discovery deferred to its own follow-on task.
   - **Hand-off lifecycle:** promotes from prose `\n---\n[HANDOFF]`
     separator to a typed `handoff:` field on tasks; `set-handoff` /
     `clear-handoff` verbs.
   - **latest-session:** structured YAML (not markdown), same record
     shape as a session-log entry; GitCommitWork parses and appends
     with session-id idempotency.

2. **R1 implementation plan:** `docs/structured-backlog-r1-plan.md` —
   TDD-by-task plan covering the full `state backlog` verb surface
   (list/show/add/init/set-status/set-results/set-handoff/clear-handoff/
   set-title/reorder/delete) + `state migrate` (backlog-scoped) +
   integration tests. 13 tasks sized for bite-sized commits; each task
   writes failing test → implements → runs to green → commits.

**What worked:**
- Brainstorming skill carried the mid-session reframe cleanly.
  Progressive one-question-at-a-time refinement caught four design
  moves (projects-not-plans, name-indexed edge list, migration
  simplification, latest-session as structured) that would have been
  missed by a single up-front design sketch.
- Audit Explore agent against the proposed verb surface surfaced two
  real gaps (`backlog init`, `memory init` for create-plan bulk seed;
  `session-log set-latest` / `show-latest` for the latest-session
  handoff) plus correctly rejected two false positives.

**What didn't:**
- Initial design sketch had related-plans per-plan as vertex-centric;
  took three clarifying exchanges to land on global-edge-list-by-name.
  Catchable earlier by explicitly asking "do you want this file
  shareable?" up front.
- First migration section was too lean ("one-shot conversion");
  user had to flag that a "complete migration tool" was needed before
  the atomicity/idempotency/validation/dry-run contract got written.

**Recommended rollout (from the design doc, for triage to backlog):**

| # | Task | Size | Deps |
|---|------|------|------|
| R1 | Ship `state backlog` verb surface + backlog-scoped `state migrate` + tests — **plan already written at `docs/structured-backlog-r1-plan.md`** | medium | — |
| R2 | Ship `state memory` verb surface + memory migration | small | — |
| R3 | Ship `state session-log` + `latest-session.yaml` + GitCommitWork rewire + migration | medium | — |
| R4 | Ship `state projects` catalog + auto-add on `ravel-lite run` | small | — |
| R5 | Ship `state related-projects` global edge list + migrate-related-projects | medium | R4 |
| R6 | Migrate all 8 phase/survey prompts to CLI verbs | medium | R1–R5 |
| R7 | LLM-driven discovery for related-projects (subagent parallelism + SHA cache) — its own research-ish task | large | R5 |

R8 (existing backlog task "Move per-plan task-count extraction") remains
blocked until R1 lands the structured backlog parser in Rust; the
dependency has been re-anchored from "this research task" to R1.

---
[HANDOFF]

Promote the following rollout units as new backlog tasks — each sized
for one work phase, grounded in the design doc and (for R1) the already-
written implementation plan:

- **R1 — Implement structured `state backlog` verb surface + backlog-scoped `state migrate` (R1)**
  - Category: `enhancement`
  - Dependencies: none (ready to pick)
  - Description: Execute the plan at `docs/structured-backlog-r1-plan.md`.
    Ships every `state backlog <verb>` command, the backlog-scoped
    migration verb, and integration tests. Does not touch prompts.

- **R2 — Implement structured `state memory` verb surface + memory migration**
  - Category: `enhancement`
  - Dependencies: R1 (establishes the schema / yaml_io / migrate patterns
    the memory submodule reuses)
  - Description: Mirrors the R1 structure for `memory.yaml`. Extends
    `state migrate` to cover `memory.md` → `memory.yaml`.

- **R3 — Implement `state session-log` + `latest-session.yaml` + GitCommitWork rewire**
  - Category: `enhancement`
  - Dependencies: R1
  - Description: Adds `state session-log` verbs (list, show, append,
    set-latest, show-latest), makes `latest-session.yaml` a typed file
    (same record shape as session-log entries), rewires
    `phase_loop::GitCommitWork` to parse the new YAML + append to
    `session-log.yaml`'s `sessions:` list with session-id idempotency.
    Extends `state migrate` to cover session-log + latest-session.

- **R4 — Implement `state projects` catalog + auto-add on `ravel-lite run`**
  - Category: `enhancement`
  - Dependencies: none (catalog file is independent)
  - Description: Global `../ravel-lite-config/projects.yaml` catalog
    mapping project names to absolute paths. CLI: `state projects list
    / add / remove / rename`. Auto-add hook in `ravel-lite run` that
    registers a new project under its directory basename on first
    invocation (collision → explicit-name prompt).

- **R5 — Implement global `state related-projects` edge list + `migrate-related-projects`**
  - Category: `enhancement`
  - Dependencies: R4 (catalog must exist to resolve names ↔ paths)
  - Description: Global `../ravel-lite-config/related-projects.yaml`
    edge list (sibling / parent-of), name-indexed, shareable between
    users. CLI: `state related-projects list [--plan <path>]`,
    `add-edge`, `remove-edge`. `state migrate-related-projects <plan-dir>`
    one-shot merges a plan's legacy `related-plans.md` into the global
    file, creating it on first call and deduping by (kind, participants).

- **R6 — Migrate all phase prompts to use CLI verbs**
  - Category: `enhancement`
  - Dependencies: R1, R2, R3, R4, R5 (all verbs must exist before
    prompts can invoke them)
  - Description: Replace direct `Read` / `Edit` of plan-state files with
    `ravel-lite state <verb>` calls across `defaults/phases/work.md`,
    `analyse-work.md`, `reflect.md`, `dream.md`, `triage.md`,
    `create-plan.md`, `defaults/survey.md`, `defaults/survey-incremental.md`.
    ~5–15 instruction rewrites per file. Prompts keep the `{{RELATED_PLANS}}`
    token (projection shape preserves plan paths).

- **R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)**
  - Category: `research`
  - Dependencies: R5
  - Description: Feature design + implementation. Given a set of
    projects, dispatch LLM subagents in parallel to analyse each
    project's README / backlog / memory and propose sibling / parent-of
    edges. SHA-based cache (keyed on per-project content hash) avoids
    re-analysing unchanged projects. Output merges into the global
    `related-projects.yaml`. Large — probably needs its own design-ish
    pass (brainstorm → spec → plan) before implementation.

---

### Move per-plan task-count extraction from LLM survey prompt into Rust

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1 (_pending promotion by triage from the research task's hand-off_) — requires the structured backlog parser R1 will land before task counts can be derived in Rust. The upstream research task resolved GO with a full R1 plan on 2026-04-22; dependency is now on R1's implementation rather than the research itself.

**Description:**

The survey LLM currently infers per-plan task counts from the raw markdown in
`backlog.md`. Once the structured backlog parser from R1 exists, task counts
(total, not_started, in_progress, done) can be computed directly in Rust and
injected as pre-populated tokens into the survey prompt — removing an
unnecessary inference burden from the LLM.

Identified as a deferred follow-on during the 2026-04-21 survey-pivot
rescoping session. Do not schedule until R1 resolves; R1's completion
is the trigger to revisit scope here.

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
