# LLM-work audit: extractable deterministic helpers

**Scope.** Enumerate every sub-task the LLM performs inside the
shipped phase prompts and shipped subagent prompts, classify each
sub-task as *judgement* or *mechanical*, and propose deterministic CLI
(or Datalog) replacements for the mechanical ones. Out of scope:
implementation.

**Frame.** Each sub-task the LLM performs that a CLI could do
identically costs (a) input-tokens pulled into the prompt to describe
the sub-task, (b) output-tokens the LLM produces executing it, (c)
non-determinism and drift risk every cycle. The codebase has already
made this tradeoff many times — `ravel-lite state backlog`/`memory`/
`session-log`/`set-phase`/`related-components`, the post-parse
`task_counts` injection into survey responses, `should_dream` as a
pure function of `memory.yaml`, path-scoped git queries,
`append_latest_to_log` as a script phase. This audit pushes the same
principle across the remaining prompt surface.

**Standing memory.** "Never do in an LLM what you can do in code" —
`feedback_mechanical_ops_in_runner.md` in the Claude-Code auto-memory.
This brief operationalises that rule prompt-by-prompt.

## 1. Enumeration — what each prompt asks the LLM to do

Each row is one distinct sub-task the LLM performs on behalf of the
phase. `M` = mechanical, `J` = judgement. Sub-tasks already extracted
(CLI calls inside the prompt) are listed once to name what's done, not
re-proposed.

### 1.1 `defaults/phases/work.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| W1 | List backlog via `state backlog list` | — | already CLI |
| W2 | List memory via `state memory list` | — | already CLI |
| W3 | List related-components via `state related-components list` | — | already CLI |
| W4 | Render backlog summary for user (title / status / priority) | M | structured rendering; CLI has `--format yaml` only — no human-readable `table`/`summary` format yet |
| W5 | Ask user for task choice, interpret response | J | conversational |
| W6 | "Pick the best next task" considering deps, priority, momentum, memory | J | genuine ranking |
| W7 | Implement the task | J | the whole point |
| W8 | Verify: tests, outputs, inspect state | J | |
| W9 | Review `.gitignore` for new generated/build/secret paths | M | path-pattern match against a set of known-generated suffixes is deterministic; judgement part is "is this a secret?" |
| W10 | Flip status via `state backlog set-status` | — | already CLI |
| W11 | Write Results block via `state backlog set-results` | — | already CLI (prose is J) |
| W12 | Transition phase via `state set-phase` | — | already CLI |

### 1.2 `defaults/phases/analyse-work.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| A1 | Read `work-baseline` SHA | M | literally `cat`; trivial to bundle into the injected context |
| A2 | `git diff <baseline> --stat` + full diff | M | LLM is shelling; content of the diff is what it reasons over — shelling itself is mechanical |
| A3 | Inspect backlog results recorded by work phase | M | already CLI-backed (`state backlog list`) |
| A4 | **Safety-net stale-status scan**: for each task whose Results is non-empty and status is `not_started`/`in_progress`, flip to `done` | **M** | strong extraction candidate — pure boolean scan + conditional write |
| A5 | Review work tree from `{{WORK_TREE_STATUS}}`; revert accidental edits (`git checkout --` / `rm`) | J (identify accidentals) + M (reversion) | LLM no longer stages or commits — the `git-commit-work` script phase does both per the `commits.yaml` spec authored in A9 |
| A6 | Justify any snapshot path intentionally left uncovered by `commits.yaml` in the session record | J | operator intent; uncovered paths surface as uncommitted residue and trigger a TUI warning |
| A7 | **Session-number determination**: count `session-log list` records, add one | **M** | `wc`+arithmetic |
| A8 | Write session record (id, ts, phase, body) | M (metadata) + J (body prose) | id/ts/phase are all determinable by Rust |
| A9 | Author `{{PLAN}}/commits.yaml` — ordered list of `{paths, message}` entries | M (YAML format) + J (partition + message prose) | `src/git.rs::apply_commits_spec` consumes the spec in the subsequent `git-commit-work` script phase and issues the actual `git add`/`git commit` calls. `commits.yaml` replaced the two-commit flow (LLM-issued source commit + plan-state commit driven by `commit-message.md`); one spec now covers source, docs, config, and plan-state |

### 1.3 `defaults/phases/reflect.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| R1 | Read latest session via `session-log show-latest` | — | already CLI |
| R2 | Read current memory via `memory list` | — | already CLI |
| R3 | Read memory-style rules | M (fetch) + J (apply) | fetch is `Read`; application is judgement |
| R4 | For each learning: new? sharpens? contradicts? obsoletes? → `memory add`/`set-body`/`delete` | J | squarely judgement |
| R5 | Style conformance (assertion register, one fact per entry, etc.) | J | |
| R6 | "Prune aggressively" | J | |
| R7 | Transition phase | — | already CLI |

Reflect is well-factored. Nothing extractable beyond what's already
done.

### 1.4 `defaults/phases/dream.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| D1 | Rewrite memory prose, consolidate overlaps | J | |
| D2 | Decide what is a pure duplicate vs distinct fact | J | |
| D3 | **Report `[STATS] <before> → <after>` word counts** | **M** | Rust already computes these in `dream.rs` — no reason to ask the LLM to recount |
| D4 | Transition phase | — | already CLI |

### 1.5 `defaults/phases/triage.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| T1 | Relevance / priority / splitting review per task | J | |
| T2 | Add new tasks implied by memory learnings | J | |
| T3 | **Dependency-field drift reconciliation**: scan every task description for "depends on X" / "blocked by X" / "requires X", cross-check with structured `dependencies` | **M (detection)** | candidates for the LLM to adjudicate; right now the LLM prose-scans every task every cycle |
| T4 | Mine completed tasks for `[HANDOFF]` markers in Results bodies | M | literal marker scan; extend `--has-handoff` filter or add `--has-handoff-marker` |
| T5 | For each hand-off: promote to task vs archive to memory | J | |
| T6 | Clear hand-off + delete completed task | — | already CLI |
| T7 | Remove no-longer-relevant tasks | J | |
| T8 | Reprioritise | J | |
| T9 | **Scan descriptions for embedded blockers** (spike / validation / shared dep buried inside a task) | M (pre-filter) + J (decide) | keyword/regex heuristic narrows the set; LLM still adjudicates |
| T10 | Write `subagent-dispatch.yaml` for cross-plan propagation | J (content) + M (format) | the mechanics of the YAML are deterministic; runner parses |
| T11 | Emit `[NO DISPATCH] <reason>` label when no dispatches | **M (noise)** | the runner already knows whether `subagent-dispatch.yaml` exists; the label adds zero signal and is already filed for removal |
| T12 | Transition phase | — | already CLI |

### 1.6 `defaults/create-plan.md`

Scaffolding dialogue with the user — mostly judgement. The file
writes (backlog, memory, phase.md, related-plans.md) all already route
through `state * init` / `state set-phase` / direct write. Nothing new
to extract.

### 1.7 `defaults/discover-stage1.md` / `defaults/discover-stage2.md`

Stage 1 extracts a surface record by reading the codebase — the
entire task is judgement. Stage 2 already emits one
`discover-proposals add-proposal` call per edge (the per-edge
extraction is the whole point of that verb's design). The prompts
deliberately ban file-writes in favour of rejection-reporting CLI
calls; this is the **reference pattern** the rest of the audit aims
for.

### 1.8 `defaults/survey.md` / `defaults/survey-incremental.md`

| # | Sub-task | Class | Notes |
|---|---|---|---|
| S1 | `task_counts` (total/not_started/in_progress/done/blocked) | — | already Rust-injected post-parse |
| S2 | **`unblocked` count**: not_started AND every dep done | **M** | pure query over parsed backlog.yaml |
| S3 | **`blocked` count**: status=blocked OR not_started with any unmet dep | **M** | pure query |
| S4 | **`received` count**: dispatches under `## Received` not yet promoted | **M** | scan on backlog.yaml |
| S5 | `cross_plan_blockers` pairs | M (candidates) + J (rationale) | graph walk; prose is judgement |
| S6 | `parallel_streams` grouping | J (partly) | uses dep graph + thematic grouping |
| S7 | `recommended_invocation_order` ranking | J | uses hints + priors |

### 1.9 `defaults/agents/pi/subagents/*.md`, `defaults/agents/pi/prompts/*.md`

Identity/role prompts. Nothing to extract.

### 1.10 Input marshalling in `src/` (Rust side)

Already carries: `WORK_TREE_STATUS` snapshot (analyse-work),
`{{ONTOLOGY_KINDS}}`, `{{CATALOG_PROJECTS}}`, `{{SURFACE_OUTPUT_PATH}}`,
`{{CONFIG_ROOT}}`, `{{SURFACE_RECORDS_YAML}}`, `{{RELATED_PLANS}}`,
`{{TOOL_READ}}`. Missing candidates (see §2): a `{{SESSION_NEXT_ID}}`
/ `{{SESSION_TS_UTC}}` pair for analyse-work, a
`{{STALE_STATUS_CANDIDATES}}` block ditto, a `{{DEPENDENCY_DRIFT}}`
block for triage, a `{{HANDOFF_CANDIDATES}}` block ditto, and a
`{{DREAM_BEFORE_WORDS}}` token for dream.

## 2. Proposed CLI extractions

### 2.1 `state backlog repair-stale-statuses` &nbsp; *(replaces A4)*

**Shape.**

```
ravel-lite state backlog repair-stale-statuses <PLAN_DIR>
    [--dry-run]                      # emit candidates, no writes
    [--format yaml|lines]            # default yaml
```

**Behaviour.** For each task in `backlog.yaml` where status is
`not_started` or `in_progress` AND Results body is non-empty (not
absent, not `_pending_`, not empty string after trim), flip status to
`done`. Output: list of flipped ids + old→new status + a short
excerpt of each Results body (for audit).

**Input/output types.** Input: `PLAN_DIR`. Output: list of `{id,
old_status, new_status, results_excerpt}`.

**Prompt section replaced.** `analyse-work.md` §5 ("Safety-net: mark
completed tasks as `done`"). The entire step collapses into one
invocation.

**Context savings.** ~15 lines of prompt prose + the LLM's per-cycle
full-backlog scan (O(N) tokens for description/Results of every
task). On a 20-task backlog this is easily 2–5 KB of avoided
re-reading per cycle.

**Risk.** None — the current prompt text emphasises "this is a
post-condition check, not a judgement call". Extracting it removes an
opportunity for the LLM to miss the check.

### 2.2 `state session-log next-id` &nbsp; *(supports A7/A8)*

**Shape.**

```
ravel-lite state session-log next-id <PLAN_DIR>
    [--timestamp]                    # print ISO-8601 UTC alongside id
```

**Behaviour.** Count records in `session-log.yaml`, add one, emit
`session-<N>`. With `--timestamp`, emit `{id: session-N, ts:
2026-04-24T12:34:56Z}` as YAML.

**Prompt section replaced.** `analyse-work.md` §7 and the ts/id
parameters of §8's `state session-log set-latest` call.

**Context savings.** Small (5 lines), but the real win is
correctness: the LLM currently shell-executes `date -u
'+%Y-%m-%dT%H:%M:%SZ'` via an inline bash command, which is one more
thing to get right every cycle.

**Alternative.** Fold the metadata into `state session-log
set-latest` with an `--auto-id --auto-timestamp` flag so the LLM
never names either value.

**Risk.** None.

### 2.3 `state session-log auto-latest` &nbsp; *(replaces the structural part of A8)*

**Shape.**

```
ravel-lite state session-log set-latest <PLAN_DIR>
    --phase <phase>
    --body-file <path>
    [--auto-id --auto-timestamp]     # new flags
```

**Behaviour.** With `--auto-id`, derive id as `session-<N+1>` where N
is the current session-log length. With `--auto-timestamp`, stamp
`now_utc_iso8601()`.

**Prompt section replaced.** The id/ts bookkeeping inside
`analyse-work.md` §8.

**Context savings.** Trivial alone, but together with 2.2 removes the
"determine session number" sub-task entirely.

### 2.4 `state backlog lint-dependencies` &nbsp; *(replaces T3)*

**Shape.**

```
ravel-lite state backlog lint-dependencies <PLAN_DIR>
    [--format yaml|lines]            # default yaml
```

**Behaviour.** For each task, extract candidate dependency ids from
the description via regex (`depends on <id>`, `blocked by <id>`,
`requires <id>`, `after <id>`, `following <id>` — canonical set
documented inside the command). Emit drift rows:

```yaml
drift:
  - task: write-tutorial
    prose_mentions:
      - setup-brew-formula          # named in description, not in `dependencies`
    structured:
      - setup-brew-formula
      - pipeline-foo                # in `dependencies`, not named in description
    kind: mismatch
```

Categories: `prose_only` (description names an id but `dependencies`
doesn't include it), `structured_only` (reverse), `mismatch` (both
sides name things, disagree), `clean` (no drift — not emitted).

**Prompt section replaced.** `triage.md` §2 closing paragraph on
"Keep the `dependencies` field in sync with the prose". The LLM goes
from scanning every task's prose to adjudicating the drift rows the
linter already flagged. Fix still uses
`state backlog set-dependencies` (unchanged) or `set-description`
(also unchanged).

**Context savings.** Moderate (O(backlog_size) prose tokens
eliminated from the triage cycle). More importantly: the LLM no
longer *has* to remember to run the check — the rows either arrive
or they don't.

**Risk.** The regex will miss non-canonical phrasings ("cannot run
until X", "X must come first"). Mitigation: the linter is a
detection pass, not an autofix; it's strictly an aid. Output should
say so in the schema docstring so operators don't treat it as
complete.

**Design note.** The canonical phrase set is authored in one place
(docstring on the lint command) and surfaced to both the LLM (via the
help text the agent can see) and the operator. Phrasings the linter
misses become a bug to fix in Rust, not a prompt-engineering
problem.

### 2.5 Extend `state backlog list --has-handoff-marker` &nbsp; *(replaces T4)*

**Shape.** A new filter on the existing `list` verb:

```
ravel-lite state backlog list <PLAN_DIR> --has-handoff-marker
```

**Behaviour.** Match tasks whose Results body contains a
`[HANDOFF] <title>` line, regardless of whether the structured
hand-off block is set. (`--has-handoff` matches the block; this new
filter matches the marker.)

**Prompt section replaced.** `triage.md` §3 step "inspect each done
task's `Results:` body for `[HANDOFF]` markers or labelled `Hand-offs:`
/ `Followups:` sections".

**Context savings.** Moderate on cycles with hand-offs, zero on
cycles without (which are the common case). But the LLM currently
*has to scan* even on cycles without, to confirm there's nothing —
the filter removes that scan unconditionally.

**Risk.** The prompt also mentions labelled `Hand-offs:` /
`Followups:` sections. Either (a) add them to the marker set the
filter recognises, or (b) deprecate them in favour of the `[HANDOFF]`
convention and mark anything found by the old shape as a lint
warning.

### 2.6 Survey: Rust-inject `unblocked`/`blocked`/`received` alongside `task_counts` &nbsp; *(replaces S2/S3/S4)*

**Shape.** Extend `src/survey/discover.rs`'s per-plan snapshot to
carry `unblocked: usize`, `blocked: usize`, `received: usize`
computed from `backlog.yaml` directly (the schema already exists in
`src/state/backlog/schema.rs`; `Task::dependencies` is structured).
Mirror the existing `inject_task_counts` pattern with
`inject_survey_counts` that overwrites whatever the LLM emitted with
the tool-derived truth.

**Algorithms.**
- `unblocked`: count of tasks where `status == not_started` AND every
  dep id in `task.dependencies` refers to a task with `status ==
  done`.
- `blocked`: count of tasks where `status == blocked` OR (`status ==
  not_started` AND any dep id is not-done).
- `received`: the survey prompt today says this is "dispatches under
  `## Received` NOT yet promoted to numbered tasks". The backlog is
  pure YAML now; if `## Received` sections persist in task
  descriptions, count them structurally. If that prose convention is
  obsolete, set `received = 0` and remove the field from the survey
  schema.

**Prompt section replaced.** Roughly 8 lines of schema docstring in
both survey prompts; the LLM stops attempting these counts.

**Context savings.** The survey prompt is the most-commonly-run LLM
invocation in multi-plan mode. Each row's tally costs a few tokens
to emit — across ten plans that's 30–50 tokens per invocation plus
the reasoning overhead of "was that dep done?". Cumulative over a
working day this is material.

**Risk.** The survey's `recommended_invocation_order` priority
rubric refers to "P1 tasks with no deps" — but priority labels
aren't currently structured on `Task`. That is a pre-existing
issue; injecting counts doesn't touch it.

### 2.7 Dream: Rust-inject `{{DREAM_BEFORE_WORDS}}` &nbsp; *(replaces D3)*

**Shape.** Add token substitution in the phase-loop for the `Dream`
phase. `memory_content_word_count` already exists in `src/dream.rs`;
the post-dream count is already computed by
`update_dream_baseline`. Inject `{{DREAM_BEFORE_WORDS}}` into the
`dream.md` prompt (pre-rewrite measurement) and change the output
format to require only the after count — or even drop the `[STATS]`
line entirely from the LLM's output and have the runner emit it.

**Prompt section replaced.** `dream.md` output format lines 60–62.

**Context savings.** Tiny. Included for completeness / consistency
with the principle.

### 2.8 Triage: remove `[NO DISPATCH]` label &nbsp; *(replaces T11)*

**Shape.** Delete the label from `triage.md`'s output format. The
runner already parses `subagent-dispatch.yaml` in
`src/subagent.rs::parse_dispatch_file`; emptiness is visible to the
runner without LLM assistance.

**Prompt section replaced.** One line of `triage.md`'s output-format
block.

**Context savings.** A handful of tokens per triage cycle.

**Note.** Already filed as user-added backlog task
`triage-should-not-emit-the-no-dispatch-label-in-its-output`. This
audit confirms the extraction is trivial (prompt-only change, no
CLI verb needed).

### 2.9 analyse-work: commit partitioning helper &nbsp; *(supports A5)*

**Shape.** A `ravel-lite commit-work partition` verb that takes the
work-baseline SHA and the current snapshot and proposes a partition
of the diff into commit groups based on path prefixes, with a
canonical grouping rubric:

```
ravel-lite commit-work partition <PLAN_DIR>
    --baseline <sha>
    [--format yaml|lines]
```

**Output shape.**

```yaml
groups:
  - name: source
    paths:
      - src/foo.rs
      - src/bar.rs
    rationale: "files under src/"
  - name: tests
    paths:
      - tests/foo_test.rs
    rationale: "files under tests/"
  - name: docs
    paths:
      - docs/analysis/llm-work-audit.md
    rationale: "files under docs/"
  - name: config
    paths:
      - .gitignore
      - Cargo.toml
    rationale: "config-family files"
  - name: plan-state
    paths:
      - LLM_STATE/core/backlog.yaml
    rationale: "plan state — reserved for git-commit-work"
    commit: false
```

Plan-state paths are reported but flagged `commit: false` because
they are committed by the script phase, not by analyse-work.

**Relation to `commits.yaml`.** The LLM still authors `commits.yaml`
end-to-end under the current flow — paths *and* messages. Partition
structure is genuine judgement when one logical change straddles path
groups, so this verb does not aim to remove partitioning from the
LLM's job. What it adds is a Rust-generated *preview* of the canonical
path-prefix bucketing that the LLM can accept verbatim, refine, or
discard before writing `commits.yaml`. The framing is "extract the
mechanical partition rubric into Rust as a starting point", not
"introduce partitioning".

**Dependency.** This is the second user-added task
(`analyse-work-better-analysis-split-commits-by-hunk-better-messages`)
and is in scope for that task, not this one. The audit ranks it here
so the two tasks can be scheduled together: the audit's candidate
here should inform the design for that task.

**Risk.** Path-based partitioning fails when one logical change
straddles multiple groups (e.g. a schema change hitting `src/` and
its doc in `docs/`). Mitigation: emit partition as a *proposal* with
an escape hatch ("merge groups A+B if the LLM determines they are
one logical change"); keep hunk-level granularity as a future
refinement triggered only when path-level is clearly wrong.

### 2.10 Datalog candidate: cross-artifact queries

The clearly-relational queries that combine backlog / memory /
session-log / related-components are:

- **"tasks unblocked by task X's completion"** — forward dep-graph
  walk. Needed by survey's ranking heuristics and by work's "best
  next task" tie-breakers.
- **"memory entries never referenced by any task's description or
  by any Results block"** — join over memory × (tasks ∪ sessions).
  Potentially a useful signal for reflect/dream pruning.
- **"related-components edges where the participants span different
  lifecycles that appear in the backlog as paired tasks"** — cross-
  plan propagation hints for triage.

Of these, only the first is clearly worth building today; the second
and third are speculative. A single-purpose
`state backlog unblocks <id>` verb satisfies #1 imperatively — no
Datalog required.

**Recommendation.** Defer Datalog. The audit's top candidates are
all flat or single-join queries that fit naturally into imperative
`state *` verbs with typed YAML output. Revisit the Datalog
question if a future backlog item surfaces a query whose shape
genuinely spans three or more artifact kinds.

## 3. Ranking

Score = **expected context-tokens saved per cycle** × **expected
quality gain from determinism**. Ranked highest-first.

| Rank | Candidate | Replaces prompt section | Input → Output | Savings | Quality risk |
|---|---|---|---|---|---|
| 1 | §2.4 `state backlog lint-dependencies` | `triage.md` §2 drift reconciliation | `PLAN_DIR` → `[{task, kind, prose_mentions, structured}]` | High — eliminates full-backlog prose scan every triage | Regex phrase set; mitigated by docstring + operator review |
| 2 | §2.6 Survey `unblocked`/`blocked`/`received` injection | `survey.md` + `survey-incremental.md` schema rows | parsed `backlog.yaml` → counts | High — runs on every multi-plan invocation (most frequent) | None for u/b; `received` depends on whether the convention is still live |
| 3 | §2.1 `state backlog repair-stale-statuses` | `analyse-work.md` §5 | `PLAN_DIR` → flipped ids | Moderate-high — eliminates full-backlog scan every cycle | None — marked "not a judgement call" in the current prompt |
| 4 | §2.9 commit partitioning (path-based) | `analyse-work.md` §6 partition reasoning | `PLAN_DIR`, baseline SHA → groups | Moderate per cycle; large when paired with multi-commit split | Path-only partition fails on cross-cutting changes; needs merge escape hatch |
| 5 | §2.5 `list --has-handoff-marker` | `triage.md` §3 marker scan | `PLAN_DIR` → matching ids | Low-moderate; cycle-sensitive | Need to decide fate of `Hand-offs:` / `Followups:` labels |
| 6 | §2.8 Remove `[NO DISPATCH]` label | `triage.md` output-format | prompt-only delete | Low (handful of tokens) | None |
| 7 | §2.3 `--auto-id --auto-timestamp` on `session-log set-latest` | `analyse-work.md` §7–8 | built-in | Low (5 lines) | None |
| 8 | §2.2 `state session-log next-id` | superseded by §2.3 | — | Redundant if §2.3 ships | — |
| 9 | §2.7 Dream `{{DREAM_BEFORE_WORDS}}` | `dream.md` output-format `[STATS]` | `memory_content_word_count` | Tiny | None |

## 4. Concrete shapes — top 3 for promotion

The three candidates below are drafted at the specificity needed for
triage to promote them into standalone backlog tasks. Each lists the
proposed verb signature, the expected internal algorithm, and the
prompt-side change that lands alongside the Rust change.

### 4.1 Rank 1 — `state backlog lint-dependencies`

**Rust.** Add `src/state/backlog/lint.rs` with
`pub fn lint_dependencies(plan_dir: &Path) -> Result<LintReport>`.
Canonical phrase regex:

```
(?ix)
    \b(?:
        depends \s on
      | blocked \s by
      | requires
      | after (?!\s+(?:all|this))     # avoid "after all" / "after this"
      | following
    )
    \s+
    ` ([a-z0-9\-]+) `                 # require backtick-quoted id
```

Backtick-quoting is the existing convention in every shipped task
description — enforcing it narrows false positives and makes the
linter's findings actionable ("add backticks around what you mean").
`LintReport::drift` is `Vec<Drift>` where `Drift` carries
`{task_id, kind, prose_mentions: Vec<String>, structured:
Vec<String>}`. Expose via CLI; YAML output is the default for
LLM consumption.

**Prompt change.** In `triage.md` §2, replace the "Keep the
`dependencies` field in sync" paragraph with a one-liner: "Run
`ravel-lite state backlog lint-dependencies {{PLAN}}` — adjudicate
each drift row by editing the description or the structured
dependencies to match whichever is correct, using
`set-description` or `set-dependencies`."

**Tests.** Unit tests for each phrase in the canonical set;
regression test for a task description with no matches;
case-insensitivity; backtick requirement; multi-line descriptions.

### 4.2 Rank 2 — Survey counts injection

**Rust.** Extend `BacklogFile::task_counts` (in
`src/state/backlog/schema.rs`) or add a sibling
`BacklogFile::survey_counts() -> SurveyCounts { unblocked, blocked,
received }`. Where the LLM would have emitted these, the
post-parse injection in `src/survey/invoke.rs` overwrites the row
with the authoritative count. Mirror `inject_task_counts`.

**Algorithm.**
- Build `done_ids: HashSet<&str>` over tasks with `status == done`.
- For each task:
  - `blocked`: if `status == blocked` → `++blocked`; else if
    `status == not_started && deps.any(|d| !done_ids.contains(d))`
    → `++blocked`.
  - `unblocked`: if `status == not_started &&
    deps.all(|d| done_ids.contains(d))` → `++unblocked`.
- `received`: audit first whether `## Received` sections still
  appear in any current `backlog.yaml`. If they do, count by
  structural scan; if not, drop the field from the survey schema.

**Prompt change.** In both `survey.md` and `survey-incremental.md`,
change the schema docstrings for `unblocked` / `blocked` / `received`
from definitions-the-LLM-computes to
"(tool-injected, do not emit)". Add a hard rule: "do not emit
`unblocked`, `blocked`, or `received`; the tool injects them
post-parse" — mirrors the existing rule for `task_counts`.

**Tests.** Snapshot test over a sample backlog where the LLM-emitted
counts are wrong; assert the injected values are authoritative.
Regression test for the "`blocked` is OR, not AND" edge case
(status=blocked without deps).

### 4.3 Rank 3 — `state backlog repair-stale-statuses`

**Rust.** New verb in `src/state/backlog/verbs.rs`:
`pub fn run_repair_stale_statuses(plan_dir: &Path, dry_run: bool)
-> Result<RepairReport>`. Algorithm:

```
for task in backlog.tasks:
    if task.status in [not_started, in_progress]:
        if task.results_non_empty():
            if !dry_run { set status = done }
            report.flipped.push({ id, old_status, results_excerpt })
```

`results_non_empty()` returns true iff Results body after
`trim_end()` is non-empty and not `_pending_` (the seed sentinel
used in `create-plan.md` §2). Expose via
`ravel-lite state backlog repair-stale-statuses <PLAN_DIR> [--dry-run]`.

**Prompt change.** In `analyse-work.md` §5, replace the full
paragraph with: "Run `ravel-lite state backlog repair-stale-statuses
{{PLAN}}` — it flips every task whose Results block was written but
whose status was left at `not_started` or `in_progress`. This is a
post-condition check, not a judgement call."

**Tests.** Unit: not_started + non-empty Results → done.
in_progress + non-empty Results → done. not_started + `_pending_`
→ no-op. done + non-empty Results → no-op. blocked + non-empty
Results → no-op (blocked is operator intent, never overridden).
`--dry-run` never writes. Idempotent on a second call.

## 5. What this audit deliberately leaves in the LLM

- **Reflect** as a phase is unchanged. Its sub-tasks (new vs
  sharpen vs contradict vs obsolete) are irreducibly judgement.
- **Commit message prose** for every `commits.yaml` entry, and
  the partition decisions about which paths belong in which
  entry, remain LLM work — that is analyse-work's reason to
  exist.
- **The ranking inside survey's `recommended_invocation_order`**
  stays LLM work. Ordering is a judgement call that uses priors
  the tool can't fully reconstruct.
- **Hand-off promote-vs-archive (triage §3)** remains judgement.
  The filter narrows the candidates; the decision still requires
  reading the hand-off content.

## 6. Open questions for triage

- Is the `## Received` convention still used in any live plan, or is
  it legacy? The survey counts it but the shipped backlog schema
  doesn't carry it structurally. Either promote it to a typed
  `received_dispatches: Vec<...>` field on `Task`, or retire the
  field. §2.6 depends on this.
- `[HANDOFF]` vs `Hand-offs:` / `Followups:` — which is canonical?
  §2.5 depends on this.
- Does analyse-work benefit from receiving the `latest-session.yaml`
  Results block as an injected token, the same way it receives
  `WORK_TREE_STATUS`? Today it reads the backlog to find it. A token
  would save a round trip but duplicate state — the existing read is
  probably fine.

## 7. Promotion checklist

Triage should promote §2.4, §2.6, and §2.1 as top-priority, each as
its own backlog task. §2.9 belongs to the user-added task
`analyse-work-better-analysis-split-commits-by-hunk-better-messages`
and should be linked from that task's description, not duplicated.
§2.8 belongs to the user-added task
`triage-should-not-emit-the-no-dispatch-label-in-its-output` and is a
trivial prompt-only change. §2.5 / §2.3 / §2.7 are small wins to
bundle opportunistically.
