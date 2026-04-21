# Backlog

## Tasks

### Extend `create-plan` to produce coordinator plans when decomposition is appropriate

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** `ravel-lite state` subcommand (✓ done — provides `push-plan` CLI verb)

**Description:**

Extend the shipped `create-plan.md` prompt so that when the LLM decides a
user's request is too large to track as a single leaf plan, it decomposes
the work into N child leaf plans plus one coordinator parent plan — and
the target directory the user passed to `ravel-lite create <path>`
becomes the coordinator, with the N children written as siblings under
the same `LLM_STATE/`.

Background: the `ravel-lite state push-plan` CLI verb landed in a prior
task but has no in-repo caller. The only coordinator-shaped prompt today
lives out-of-repo at `{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/`
and was hand-crafted. `create-plan` currently produces only leaf plans,
so no user running `ravel-lite create` can produce a coordinator —
push-plan is born unused from any LLM authored by ravel-lite's shipped
prompts. This task closes that gap and gives the CLI verb a first-class
caller whose argv shape lives under prompt-contract drift-check tests.

**Design settled in the predecessor discussion (record in git at commit
`87a9309` Results block; detailed transcript cited by the user in the
follow-on session):**

**Q1 — How is the coordinator's `prompt-work.md` produced? → Option (b):
LLM authorship + shared boilerplate fragment.**

Ship a new embedded default `defaults/coordinator-work-boilerplate.md`
that holds the invariant blocks a coordinator prompt must contain:

- **OVERRIDE NOTICE block.** Explicit "this REPLACES the generic work
  workflow" preamble. Enumerates what a coordinator does NOT do:
  no reading `backlog.md` for implementation tasks, no picking a task
  and implementing it, no advancing `phase.md` on a pivot (the driver
  does it), no session log / source edits. Reference implementation:
  `{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md:1-17`.
- **"Never leave both stack and phase unchanged" rule.** The driver
  halts with a "phase did not advance" error if the coordinator's work
  phase exits with `phase.md` still at `work` AND `stack.yaml`
  unmodified. Reference: same file `:132-136`.
- **`ravel-lite state push-plan` usage block.** Canonical argv form
  with all flags documented inline:

  ```
  ravel-lite state push-plan {{PLAN}} <target-plan-dir> [--reason <s>]
  ```

  Explicitly replaces the prose case-analysis that currently appears
  in Ravel's out-of-repo coordinator (lines 91-110) about
  "if stack.yaml does not exist, create with root+target; if it has
  only the root, rewrite with both; if multi-frame, append." That logic
  now lives once in `src/state.rs::run_push_plan` + `pivot::validate_push`;
  the boilerplate just tells the LLM to invoke the verb.

The extended `defaults/create-plan.md` instructs the LLM to include
this fragment verbatim in the coordinator's `prompt-work.md` and
customize only: (a) the authoritative children list, (b) the memory
section (e.g. Gates-style cross-plan state the coordinator tracks),
(c) any coordinator-specific routing guidance (recommendation grid,
inspect-subagent pattern, bottleneck watch — all optional elaborations
on top of the invariants).

**Q2 — How does a coordinator enumerate its children? → Authoritative
list in the prompt, NOT filesystem-derived.**

Rationale (from user): sibling plans under the same `LLM_STATE/` may
exist that are NOT part of this coordinator's scope (archived plans,
unrelated work). The coordinator needs editorial control over which
siblings it routes to. Filesystem-derived discovery would scoop those
in. The boilerplate should include a note template like "If this list
drifts from the real filesystem, update this prompt — the list is
authoritative for what you consider" (matching Ravel's pattern).

**Q3 — What does decomposition look like in the `create-plan` flow?**

Concrete shape:

1. User runs `ravel-lite create <path>`.
2. LLM asks scope/goal questions (existing flow, unchanged).
3. LLM judges: leaf or coordinator? If the work has internal
   structure that would exceed a single plan's healthy backlog size
   (rough heuristic — the judgment is the LLM's), propose decomposition.
4. LLM proposes a decomposition to the user: "I think this should be N
   sub-plans: X, Y, Z. Do you agree?" One at a time, conversationally
   — not a batched approval flow.
5. On user agreement: LLM writes the N child leaf plans FIRST as
   siblings to `<path>` (all under the same `LLM_STATE/` parent),
   then writes `<path>` as the coordinator plan. Children-first
   ordering has a safety property — if the LLM bails out mid-creation
   the partially-created result is N orphan leaves, each independently
   usable; creating the coordinator first would leave an
   instant-coordinator with an empty authoritative-children list,
   broken by construction.
6. **No seeded backlog.** The coordinator's initial `backlog.md` is
   header-only. The coordinator's first work cycle is itself the
   first routing decision — "pick the next child to run." Matches
   Ravel's pattern; avoids duplicating "run child X for first cycle"
   tasks that add no information on top of the authoritative list.
7. Coordinator plan dir layout is otherwise identical to a leaf plan
   (phase.md=`work`, memory.md/session-log.md header-only,
   dream-baseline=`0`) — only `prompt-work.md` differs.

**Scoping correction vs. prior hand-off:**

The predecessor discussion suggested Task B "lives in `ravel-lite-config`,
not this repo." That scoping is wrong for the init pipeline and caused
the reporting gap the user hit: `init --force` scaffolds FROM
`defaults/` in this repo INTO the user's config dir. Files that exist
only in `ravel-lite-config/` are hand-edited and cannot propagate to
new users or to `init --force` refreshes. Correct scoping:

1. `defaults/create-plan.md` — extended in-place with the new
   decomposition flow, Q&A, and instructions to include the boilerplate.
2. `defaults/coordinator-work-boilerplate.md` — new file, embedded.
3. `src/init.rs::EMBEDDED_FILES` — register the new boilerplate entry
   (next to the existing `create-plan.md` entry at line 38).
4. Optional but recommended: a drift-guard test analogous to
   `every_default_coding_style_file_is_embedded` (in `src/init.rs`
   tests) that asserts `coordinator-work-boilerplate.md` is both on
   disk in `defaults/` and registered in `EMBEDDED_FILES`. The current
   coding-style drift guard only scans `coding-style-*.md`; broaden it
   or add a parallel check.

**Reference example (read before implementing):**

`{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md` is a
working coordinator prompt in production. Use it to shape the
boilerplate's invariant blocks, but:

- REPLACE the inline stack.yaml case-analysis (Ravel's lines 91-110)
  with the `ravel-lite state push-plan` call — the CLI verb now owns
  that logic.
- The Gates memory convention, Bottleneck watch section, and
  inspect-subagent pattern in Ravel's prompt are coordinator-specific
  elaborations — do NOT put them in the boilerplate. They belong to
  the per-coordinator customization layer the LLM writes around the
  boilerplate.

**Deliverables:**

1. `defaults/coordinator-work-boilerplate.md` (new, embedded).
2. `defaults/create-plan.md` extended with decomposition flow.
3. `src/init.rs::EMBEDDED_FILES` — register (1).
4. `src/init.rs` tests — a new drift-guard test (or broadened
   existing one) for the new file.
5. Consider: update `shipped_pi_prompts_have_no_dangling_tokens`-style
   guard to cover the boilerplate, ensuring no `{{…}}` tokens drift.
6. Smoke-test: scaffold a temp config with `ravel-lite init <tmp>`
   and verify `<tmp>/coordinator-work-boilerplate.md` exists and is
   byte-identical to `defaults/coordinator-work-boilerplate.md`.

**Out of scope (do NOT do as part of this task):**

- Modifying `ravel-lite create`'s Rust code. The judgment "leaf vs
  coordinator" and the children-first write order happen inside the
  LLM executing `create-plan.md` — nothing in `src/create.rs` needs
  to change. If that assumption breaks during implementation,
  surface it and re-scope.
- Auto-seeding the coordinator's backlog with routing tasks. Settled:
  no seeding.
- Filesystem-derived children discovery. Settled: authoritative list.
- Modifying Ravel's out-of-repo `ravel-orchestrator/prompt-work.md`.
  That's the reference-example input, not a migration target.

**Results:** _pending_

---

### Preserve hand-off rationale across the analyse-work → triage boundary

**Category:** `prompt-tuning`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Hand-offs — forward-looking design notes that a work session settles but
doesn't itself implement — currently lose their rationale between
analyse-work and triage, and sometimes vanish entirely. Root cause
surfaced during the coordinator-plan-creation investigation
(`2026-04-21` session, user-quoted transcript): a ~250-line design
discussion compressed to ~15 lines in analyse-work's latest-session.md
and Results-block note, then was dropped entirely by the subsequent
triage phase. Only the terminal-title hand-off from the same session
survived; the coordinator-plan one did not.

Two mechanisms compound:

**1. Authoring side — `defaults/phases/analyse-work.md` has no guidance
on how to record hand-offs.** The prompt's `latest-session.md` format
(analyse-work.md:80-88) specifies "What was attempted / What worked /
What to try next / Key learnings" — none of which maps cleanly to "a
designed-but-unimplemented follow-on task." There is also no
prescribed convention for adding notes to an existing task's Results
block, though sessions have been doing so ad-hoc. Consequence:
hand-offs land in whichever free-form prose section the LLM chose,
compressed to one or two sentences, with settled design decisions
(Q&A outcomes, chosen trade-offs, rejected alternatives) stripped.

**2. Consuming side — `defaults/phases/triage.md` cannot see
latest-session.md** (triage.md:17-18 explicitly lists it under `Do NOT
read`), and it **deletes completed tasks outright** (triage.md:34-39
step 3) without mining their Results blocks for forward-looking
items. So any hand-off parked in a completed task's Results block is
trashed along with the task — even if it contained the only surviving
record of a settled design decision.

**Fix (both prompts, coupled):**

A. **`defaults/phases/analyse-work.md`:**

   - Add a prescribed hand-off convention. When the work session settled
     a design for a task that isn't in scope to implement this cycle,
     analyse-work MUST either (i) add a new standalone task to
     `backlog.md` with status `not_started` and a description that
     inlines the settled design (not a one-liner pointer), or
     (ii) record the hand-off in a clearly-labelled `## Hand-offs`
     section of `latest-session.md` AND add a `[HANDOFF]`-style note
     to the completing task's Results block that names the hand-off
     by title. Option (i) is preferred when design is concrete enough
     to backlog directly; option (ii) is the fallback for partially-
     settled designs.

   - Require hand-off notes to inline (at minimum): the problem being
     solved, the chosen design decisions with one-sentence rationale
     each, pointers to any reference examples (file paths, line
     numbers), and the dependencies. Target verbosity: enough that
     triage can decide to promote without rereading the whole diff.
     Acceptable size: 10-40 lines per hand-off. Not a one-liner.

   - Cross-reference: `defaults/phases/triage.md` step 3 mines these
     before deletion.

B. **`defaults/phases/triage.md`:**

   - Before deleting a completed task (step 3), scan its Results block
     for `[HANDOFF]` markers or a `Followups:` / `Hand-offs:` section.
     For each, either promote to a new top-level backlog task (if
     concrete) or move to memory.md as a design-intent entry (if
     strategic and not yet concrete). Only AFTER this extraction is
     the completed task safe to delete.

   - Add `[HANDOFF]` and `[PROMOTED]` / `[ARCHIVED]` to the output
     format vocabulary (triage.md:80-87) so the action shows up in
     the triage summary.

   - Keep `Do NOT read latest-session.md` — this task explicitly does
     NOT propose lifting that restriction; cross-phase context should
     flow through memory.md and the backlog, not through session
     transcripts. The fix is to make those two channels carry enough
     signal, not to widen triage's input set.

**Non-goals:**

- Making triage read `latest-session.md`. Rejected: the separation of
  "durable learnings in memory.md" vs "session-local transcript in
  latest-session.md" is load-bearing.
- Retroactively recovering the lost coordinator-plan hand-off. That's
  already been done out-of-band — the user quoted the transcript into
  the current session and the task is now in backlog. This task is
  prophylactic for future sessions.
- Changing `reflect`'s behaviour. Reflect consumes latest-session.md
  and writes memory.md; it already has the opportunity to capture
  hand-off rationale if the session record includes it. This task's
  analyse-work changes feed reflect as a side-effect — explicit
  reflect prompt changes are out of scope unless investigation
  reveals a separate gap.

**Deliverables:**

1. `defaults/phases/analyse-work.md` — new hand-off convention
   section; latest-session.md format amended.
2. `defaults/phases/triage.md` — pre-deletion hand-off extraction
   step; output-format vocabulary extended.
3. Optional: an integration test in `tests/integration.rs` that runs
   a synthetic analyse-work → triage cycle with a hand-off planted in
   a completing task's Results block and asserts the hand-off ends up
   as a new `not_started` task post-triage (not deleted with the
   completed parent). The `ContractMockAgent` pattern already used
   by `phase_contract_round_trip_writes_expected_files` is the likely
   starting point.
4. Memory update: once the convention is live, add a memory.md entry
   recording the convention so future work phases know to expect it.

**Measurement:** the next settled-but-not-implemented hand-off should
survive a full cycle without manual intervention.

**Results:** _pending_

---

### Narrow `warn_if_project_tree_dirty` to work-agent-touched files only

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`warn_if_project_tree_dirty` at `phase_loop.rs:94` is pathspec-unscoped
— it fires on any dirty file in the project tree. In monorepos with
multiple plans the check can still produce false positives from sibling
plans' in-flight writes, even after the atomic phase-transition fix.

Narrow the check to: compute `git diff --name-only <work_baseline>`
(files changed since the work baseline) intersected with the current
dirty list, so the warning only fires on files the active work agent
could plausibly have touched. This is a defense-in-depth refinement;
no correctness regression possible since the current check is strictly
more noisy, never more accurate.

Note: work-baseline is seeded atomically in the triage commit
(`git_save_work_baseline` in `GitCommitTriage`), so the baseline SHA
is reliably available when the dirty check runs.

**Results:** _pending_

---

### Clean up stale `ravel-lite-config/skills/` detritus from renamed defaults location

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`ravel-lite-config/skills/` is stale detritus from the former
`defaults/skills/` location, which was renamed to
`agents/pi/subagents/`. The `init --force` command does not delete
files that have been removed from `EMBEDDED_FILES`, so the stale
`skills/` directory persists in existing config dirs after an
`init --force` refresh.

Decision to make: either (a) document this as expected behaviour
("init never deletes, only scaffolds") with a clear note in the
README or init help text, or (b) add a cleanup mode to `init --force`
that removes files no longer registered in `EMBEDDED_FILES`.

Surfaced as a followup during the coordinator-plan-creation discussion
(`2026-04-21`).

**Deliverables:**

1. Decision: document-only or add cleanup mode.
2. Either: a note in README/help text about init's non-destructive
   semantics, or a `--prune` flag implementation in `src/init.rs`
   that removes unregistered embedded files.

**Results:** _pending_

---

### Research: expose plan-state markdown as structured data via `ravel-lite state <file> <verb>` CLI

**Category:** `research`
**Status:** `not_started`
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
- `stack.yaml` — already has `push-plan`; sufficient.
- Any file under `fixed-memory/` — those are static documentation,
  not plan state.

**Related context:**

- Memory entry `Phase prompts invoke 'ravel-lite state set-phase'`
  records the convention this task generalises.
- The "Preserve hand-off rationale" task above (if landed first)
  would change the shape of Results-block authorship, which feeds
  directly into Q6. Sequence matters: this research task benefits
  from landing after that one, so the Results-block convention is
  stable.

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
