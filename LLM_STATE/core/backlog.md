# Backlog

## Tasks

### Extend `create-plan` to produce coordinator plans when decomposition is appropriate

**Category:** `feature`
**Status:** `done`
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

**Results:**

Superseded by the survey-driven multi-plan run design captured in
`docs/survey-pivot-design.md` and decomposed into items 5a/5b/5c/5d
in the new `LLM_STATE/survey-restructure/` plan. The LLM-authored
coordinator concept moves from prompt-space into Rust: routing
intelligence lives in the runner (item 5c) rather than in a
specialised coordinator prompt. The infrastructure this task was
going to build on — `stack.yaml`, `push-plan`, `pivot.rs`,
`run_stack` — is itself removed in item 5d. No code was written
for this task before the pivot; the 2026-04-21 rescoping session
established the new direction before any coordinator-work-boilerplate
was authored.

---

### Integration test for `[HANDOFF]` convention in analyse-work → triage cycle

**Category:** `test`
**Status:** `not_started`
**Dependencies:** none — convention is live in both `defaults/phases/analyse-work.md` and `defaults/phases/triage.md`

**Description:**

Extend `ContractMockAgent` to inject `[HANDOFF]` markers into a Results
block and run a synthetic analyse-work → triage cycle. Assert that triage
correctly mines the marker and either promotes it to a new backlog task
or archives it to `memory.md`.

This was deferred from the "Preserve hand-off rationale" task. The
convention is now live in shipped prompts; the next real hand-off session
is the first end-to-end exercise, but an automated test guards the
pipeline before that.

**Deliverables:**

1. A `ContractMockAgent::invoke_headless` injection for `AnalyseWork`
   that emits a `[HANDOFF]` marker in a completing task's `Results:`
   block inside `latest-session.md`.
2. A test that runs a full analyse-work → git-commit-work cycle,
   then a triage cycle, and asserts the hand-off survives as either
   a new `not_started` backlog task or a new `memory.md` entry.

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
- The "Preserve hand-off rationale" task (now done) means
  Q6 can rely on the `[HANDOFF]` convention in Results blocks.
  The research question is narrower as a result: the Results block
  authorship path only needs to support the now-stable convention.

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

### Make `ravel-lite survey` incremental via per-plan input hashes

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none — the survey design already splits structured YAML (`src/survey/schema.rs`) from human rendering (`src/survey/render.rs`), so the two-pass shape is in place.

**Description:**

Today `ravel-lite survey` re-reads every plan and re-asks the LLM on
each invocation, even when nothing changed. On multi-plan config roots
the cost scales with plan count, not with what actually moved. Make
it incremental so subsequent runs only re-analyse changed plans and
merge their results into the prior structured response.

**Shape:**

1. **Hash per plan.** `discover_plans` (`src/survey/discover.rs`)
   computes a stable hash over the files that contribute to each
   `PlanSnapshot` — proposed: phase + backlog + memory + related-plans
   (explicitly NOT session-log.md, which is append-only and would
   defeat change detection). Include the hash in `PlanRow`
   (`src/survey/schema.rs`).

2. **Persist prior structured response.** `SurveyResponse` is already
   YAML-round-trippable; write it to a stable location on each run
   (proposed: `<config_root>/survey-state.yaml`) so next-run can load
   it, diff per-plan hashes, and classify plans as `unchanged` /
   `changed` / `removed`.

3. **Feed only the delta to the LLM.** `defaults/survey.md` gains two
   new tokens: the prior structured response (context) and the
   changed-plans input rendered by `render_survey_input`. Prompt
   instruction: re-analyse only the listed plans, revisit cross-plan
   blockers/streams if the changes affect them, and preserve all
   unchanged rows verbatim.

4. **Structured-first, human-render second.** The LLM always returns
   YAML (as today). `render::render_survey_output` runs over the
   merged structured response — not the raw LLM delta — so the
   presentation layer stays separate from the data layer.

**Open questions to settle during implementation:**

- Which files contribute to a plan's hash? Proposal above (phase +
  backlog + memory + related-plans); confirm or amend.
- Where does the state file live — per config root or per project?
  Multiple `--root` args suggest per-config-root.
- Schema-version marker so a `SurveyResponse` struct change
  invalidates old state rather than mis-parsing.
- Merge validation: refuse to accept a delta that mutates a plan
  outside the declared `changed` set.

**Deliverables:**

1. Per-plan hash in `discover.rs` and a new `input_hash` field on
   `PlanRow`.
2. State-file I/O in `schema.rs` (or a new `state.rs` sibling).
3. Diff-and-prompt logic in `invoke.rs`.
4. Extended `defaults/survey.md` prompt with new tokens and
   instructions.
5. Tests: unchanged-plan reuse, changed-plan re-analysis,
   removed-plan pruning, schema-bump invalidation.
6. `--force` flag on the CLI to bypass incremental mode when debugging.

**Results:**

Split into four finer-grained items in the new
`LLM_STATE/survey-restructure/` plan:

- **5a:** Structured YAML output for `survey` (canonical
  round-trip, positional plan-dir args, `survey-format` subcommand,
  forward-compat `input_hash` field injected in Rust post-parse).
- **5b:** Incremental survey via `--prior` — the original intent
  of this task, plus `--force` bypass and a `schema_version` marker.
- **5c:** Multi-plan `run` mode with survey-driven routing (new
  scope, replaces former task #1's coordinator-plan concept).
- **5d:** Remove `stack.yaml`, `push-plan` CLI, `pivot.rs`, and
  `run_stack` — cleanup of the now-obsolete coordinator
  infrastructure.

5a → 5b → 5c form a linear dependency chain; 5d is independent.
Architectural rationale and open decisions per item are captured
in `docs/survey-pivot-design.md`. The split was driven by two
realisations during the 2026-04-21 rescoping session: (1) the
original task's scope was plan-sized rather than cycle-sized;
(2) "incremental survey" is a precondition for a larger
architectural shift (multi-plan routing) that this task alone
did not capture.

---

### Make git operations subtree-scoped so ravel-lite can run inside a monorepo

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** composes naturally with the narrowed `warn_if_project_tree_dirty` (now done) — the subtree-scoping uses the same pathspec plumbing.

**Description:**

`src/git.rs` assumes the project is its own top-level git repo. In a
monorepo, the subtree ravel-lite orchestrates lives inside a larger
`.git/`. `find_project_root` walks up to that outer `.git/` and returns
the monorepo root, so every un-scoped git call (`git status --porcelain`,
`git diff --stat <baseline>`, `git add .`) reports on or writes across
sibling subtrees that aren't part of the plan.

**Simplification the user settled in the `2026-04-21` discussion:** the
project root is always `<plan_dir>/../..`. No config, no marker file —
it's a pure path derivation. `find_project_root`'s current `.git`-walkup
collapses two concepts that are only identical in the single-repo case:
"where is `.git`?" and "where is the subtree ravel-lite controls?"
Using the plan-dir derivation separates them cleanly.

**Fix direction:**

1. Replace (or narrow) `find_project_root` so it returns
   `<plan_dir>/../..`. That path IS the subtree root; in the
   single-repo case it also happens to be the git-repo root, so
   existing behaviour is unchanged.

2. Scope every git call in `src/git.rs` to the subtree root as a
   pathspec:
   - `git status --porcelain -- <project_root>`
   - `git diff --stat <baseline> -- <project_root>`
   - `git diff --name-only <baseline> -- <project_root>`
     (used by the narrowed `warn_if_project_tree_dirty`)
   - `git add -- <project_root>` in `git_commit_plan` (currently
     `git add .` under `plan_dir` — that's already scoped to the
     plan dir but doesn't help when the commit should also capture
     sibling source edits inside the subtree but outside the plan).

3. `work-baseline` stays a repo-wide `HEAD` SHA — the diff
   *query* is the scoping surface, not the baseline itself.

**Open design questions:**

- Commit-message prefix convention — may or may not need
  parameterisation if the monorepo has conventional-commit rules. The
  LLM-authored `commit-message.md` is already a per-commit override,
  so the most minimal approach is to do nothing here and let the
  user's prompt customisation carry whatever prefix the monorepo
  wants.
- `ravel-lite create` scaffolds into `<path>` — the derivation
  `<plan_dir>/../..` assumes the user passes a path two levels deep
  under the subtree root. Verify that's what `create` produces today
  and document the expectation if so.

**Deliverables:**

1. `src/git.rs` — `find_project_root` (or a replacement) derives
   subtree root from the plan dir; every git call takes a pathspec.
2. Integration test: synthesise an outer repo containing an inner
   subtree with a ravel-lite plan; assert per-phase commits, baseline
   diffs, and dirty-tree warnings are scoped to the subtree.
3. README section on running ravel-lite inside a monorepo subtree,
   noting the `<plan_dir>/../..` rule.

**Out of scope:**

- `git subtree` / `git submodule` mechanics — this task is about
  pathspec scoping of git queries, not about the embedding strategy.
- Initialising the outer monorepo — ravel-lite assumes a `.git`
  exists somewhere up the tree; if neither monorepo nor subtree has
  one, that's a separate error path.

**Results:** _pending_

---
