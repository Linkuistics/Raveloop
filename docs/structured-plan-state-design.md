# Structured Plan-State Design

**Status:** Proposed. Recommendation: **go**. Prototype pending (see _Prototype scope_).
**Date:** 2026-04-22
**Related backlog task:** _Research: expose plan-state markdown as structured data via `ravel-lite state <file> <verb>` CLI_.

## Problem

Ravel-lite's plan state lives in markdown files under `<project>/LLM_STATE/<plan>/`:
`backlog.md`, `memory.md`, `session-log.md`, `related-plans.md`, `phase.md`.
Every LLM phase reads and edits these directly via `Read` and `Edit` tool calls,
with prose conventions (headings, `---` separators, `**Status:**` fields)
serving as the only schema.

This costs context (work-phase reads all ~450 lines of `backlog.md` to pick
one task), costs tool calls (Edit requires exact-string anchors, often needing
a probe Read first), and permits silent drift (invalid status values, missing
Results on done tasks, stale Status when Results is non-empty). The
`ravel-lite state set-phase` precedent has already demonstrated that a typed
CLI over small state files (`phase.md`) pays off; the question is whether the
pattern scales up.

## Reframe

The original task framed the decision as "markdown as source of truth" vs
"structured sidecar" vs "canonical markdown with strict grammar." The reframe
adopted here is simpler:

**Plan-state files are structured data that may contain markdown as
element content. LLMs never read or write them directly — only through CLI
verbs.**

This removes the conflict between human-readable markdown and machine-
parseable structure: files are YAML (still readable), and multi-paragraph
markdown bodies (task descriptions, Results, memory bodies, session bodies)
live inside YAML block-scalar fields where they render cleanly.

## Decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Scope of conversion | All plan-state files (per-plan: backlog, memory, session-log, latest-session, phase; global: related-projects, projects catalog) |
| 2 | Storage format | YAML |
| 3 | File layout | One file per type |
| 4 | Record identity | Slug from title at creation, persisted, stable across title edits |
| 5 | CLI output format | YAML default, `--format json` on request |
| 6 | Body field authoring | Inline scalars + `--body-file <path>` OR `--body -` (stdin) |
| 7 | Hand-edit policy | Strict parse, canonical write (no comment preservation) |
| 8 | Migration path | Single-plan `state migrate <plan-dir>` + single-plan `state migrate-related-projects <plan-dir>` (no cross-plan orchestrator) |
| 9 | Global config location | `../ravel-lite-config/` (sibling dir, already in use for `fixed-memory/`) |
| 10 | Catalog population | Auto-add project on first `ravel-lite run` |

Additional decisions from audit:
- Relationships are between **projects** (stable), not plans (ephemeral).
- Global `related-projects.yaml` is an **edge list**, shareable between users,
  references projects **by name**; names resolve to paths via a per-user
  `projects.yaml` catalog.
- `handoff` promotes from a prose `\n---\n[HANDOFF]` block to a typed field on
  each task.
- Dream rewrites memory **per entry** (add/set-body/delete), not as a bulk
  atomic swap.
- `latest-session.yaml` is plan-state, covered by migrate. `commit-message.md`
  remains an orchestrator artifact (scalar string, no structured consumer).
- Session id is assigned by the writer (analyse-work), not the appender
  (GitCommitWork), so `latest-session.yaml` is a fully-formed session-log
  record at rest.

## Schemas

### `backlog.yaml`

```yaml
tasks:
  - id: add-clippy-d-warnings-ci-gate
    title: Add clippy `-D warnings` CI gate
    category: maintenance
    status: not_started            # not_started | in_progress | done | blocked
    blocked_reason: null           # required iff status == blocked
    dependencies: []               # list of task ids
    description: |
      Multi-paragraph markdown body describing the task.
    results: null                  # null until set; multi-line markdown when done
    handoff: null                  # optional hand-off block appended in analyse-work
```

### `memory.yaml`

```yaml
entries:
  - id: phase-prompts-invoke-ravel-lite-state-set-phase
    title: Phase prompts invoke `ravel-lite state set-phase`
    body: |
      Multi-paragraph markdown body.
```

Order preserved by list position. Dream uses per-entry mutation.

### `session-log.yaml`

```yaml
sessions:
  - id: 2026-04-22-run-plan-work-core
    timestamp: 2026-04-22T14:33:07Z
    phase: work
    body: |
      Full session body markdown.
```

Append-only from the LLM's perspective. If size ever becomes a problem,
future work can split into per-session files.

### `latest-session.yaml`

Transient file between analyse-work and git-commit-work. Same record shape
as a session-log entry (id, timestamp, phase, body). Analyse-work writes via
`state session-log set-latest`; git-commit-work parses and appends to
`session-log.yaml`'s `sessions:` list, using `id` for idempotency.

### `related-projects.yaml` (global, shareable)

Lives at `../ravel-lite-config/related-projects.yaml`. References projects
by name.

```yaml
schema_version: 1
edges:
  - kind: sibling                         # symmetric
    projects: [ravel-lite, other-proj]
    note: shared telemetry schema
  - kind: parent-of                       # directed
    from: monorepo-pkg-a
    to:   monorepo-pkg-a-sub
    note: sub is a sub-package of pkg-a
```

### `projects.yaml` (global, per-user)

Lives at `../ravel-lite-config/projects.yaml`. Contains absolute paths —
not shared between users.

```yaml
schema_version: 1
projects:
  - name: ravel-lite
    path: /Users/antony/Development/Ravel-Lite
  - name: other-proj
    path: /Users/antony/Development/other-proj
```

Auto-populated on `ravel-lite run <plan-path>` when the plan's project
(derived as `<plan>/../..`) has no catalog entry, using the directory
basename as the default name (collision triggers explicit-rename prompt).

### `phase.yaml`

```yaml
phase: work   # work | analyse-work | git-commit-work | reflect | git-commit-reflect
              # | dream | git-commit-dream | triage | git-commit-triage
```

Already managed by `state set-phase`; schema formalised by this design,
no new verb needed.

## CLI surface

All verbs sit under `ravel-lite state`. Existing `set-phase` is preserved
unchanged.

### `state backlog`

```
state backlog list       [--status <s>] [--category <c>] [--ready]
                         [--has-handoff] [--missing-results]
                         [--format yaml|json]                       # yaml default
state backlog show       <id> [--format yaml|json]
state backlog add        --title <t> --category <c>
                         [--dependencies <id,id,...>]
                         [--description-file <path> | --description -]
state backlog init       --body-file <path>                         # create-plan only; fails if non-empty
state backlog set-status <id> <status> [--reason <text>]            # reason required for `blocked`
state backlog set-results <id> [--body-file <path> | --body -]
state backlog set-handoff <id> [--body-file <path> | --body -]
state backlog clear-handoff <id>
state backlog set-title  <id> <new-title>                           # id unchanged
state backlog reorder    <id> <before|after> <target-id>
state backlog delete     <id>
```

Filter semantics:
- `--ready` = `status == not_started AND every dep.status == done`
- `--missing-results` = `status == done AND results == null`
- `--has-handoff` = `handoff != null`

### `state memory`

```
state memory list       [--format yaml|json]
state memory show       <id>
state memory add        --title <t> [--body-file <path> | --body -]
state memory init       --body-file <path>                          # create-plan only; fails if non-empty
state memory set-body   <id> [--body-file <path> | --body -]
state memory set-title  <id> <new-title>
state memory delete     <id>
```

### `state session-log`

```
state session-log list         [--format yaml|json] [--limit <n>]
state session-log show         <id>
state session-log append       --body-file <path> [--phase <p>] [--timestamp <iso8601>]
state session-log set-latest   [--body-file <path> | --body -]
                               [--phase <p>] [--timestamp <iso8601>]
state session-log show-latest  [--format yaml|json]
```

`set-latest` writes `latest-session.yaml`. `show-latest` reads it (consumed
by reflect). The `latest -> sessions` append happens in Rust inside
`phase_loop::GitCommitWork`, idempotent on session id.

### `state related-projects`

```
state related-projects list        [--plan <path>] [--format yaml|json]
state related-projects add-edge    --kind sibling   --projects <n1,n2> [--note <text>]
state related-projects add-edge    --kind parent-of --from <n> --to <n> [--note <text>]
state related-projects remove-edge --kind <k> ...
```

`list --plan <path>` derives the plan's project via `<plan>/../..`, reverse-
looks the path up in `projects.yaml` to get the project name, walks edges
involving that name, forward-looks related names back to paths, and emits
a projection with active plans per related project. Missing catalog entries
degrade gracefully (warn + empty projection).

### `state projects` (catalog)

```
state projects list    [--format yaml|json]
state projects add     --name <n> --path <p>
state projects remove  <name>
state projects rename  <old> <new>                                  # cascades into related-projects.yaml
```

### `state phase`

```
state set-phase <plan-dir> <phase>     # existing
state get-phase <plan-dir>             # new, symmetric
```

### `state migrate` (single-plan, both verbs)

```
state migrate <plan-dir>                    [--dry-run]
                                            [--keep-originals | --delete-originals]
                                            [--force]
    # Per-plan: backlog.md, memory.md, session-log.md, latest-session.md
    # (if present), phase.md → .yaml siblings. Does NOT touch related-plans.md
    # or any global files.

state migrate-related-projects <plan-dir>   [--dry-run]
                                            [--keep-originals | --delete-originals]
                                            [--force]
    # Per-plan: reads the plan's related-plans.md, merges edges into the
    # global ../ravel-lite-config/related-projects.yaml (dedupe by
    # kind+participants). Adds the plan's project to projects.yaml catalog
    # if absent (basename as name; errors on collision).
```

Shared flag semantics:
- `--dry-run` — prints planned changes; writes nothing.
- `--keep-originals` (default) — `.md` survives migration.
- `--delete-originals` — `.md` removed only after write + validation both succeed.
- `--force` — overwrite when target `.yaml` exists and differs from the re-migration output.

## Migration

Migration is **single-plan-scoped** — each verb acts on one `<plan-dir>`
passed explicitly. No discovery, no cross-plan walking, no orchestrator.
Users invoke the verbs once per plan.

### `state migrate <plan-dir>`

Converts per-plan files in one plan directory:

- `backlog.md` → `backlog.yaml`
- `memory.md` → `memory.yaml`
- `session-log.md` → `session-log.yaml`
- `latest-session.md` → `latest-session.yaml` (only if present)
- `phase.md` → `phase.yaml`

Does **not** touch `related-plans.md` (separate verb handles it) or any
global files.

### `state migrate-related-projects <plan-dir>`

Per-plan. Reads the plan's `related-plans.md`, merges resulting edges into
`../ravel-lite-config/related-projects.yaml` (deduped by `kind +
participants`). Creates the global file on first invocation; merges on
subsequent ones. Adds the plan's project (derived as `<plan-dir>/../..`)
to `../ravel-lite-config/projects.yaml` if absent, using the directory
basename as the name; errors on collision (user resolves with
`state projects rename`).

### Behaviour contract (both verbs)

| Aspect | Behaviour |
|--------|-----------|
| Atomicity | Parse all source files first; write all targets second. Any parse failure → write nothing, exit non-zero with file + line. |
| Idempotency | Target `.yaml` already exists and matches re-migration output → no-op (exit 0). Exists and differs → abort unless `--force`. |
| Validation | Every emitted `.yaml` is re-parsed against its schema before the command returns success. Mismatch aborts. |
| Original cleanup | `.md` files deleted only with `--delete-originals`, only after write + validation both succeed. Default `--keep-originals` leaves them for safe re-run. |
| No cross-plan walking | Neither verb reads or writes state for plans other than `<plan-dir>`. |

### Parser design notes

- Markdown structure is strict: each task starts with `### <title>`,
  followed by `**Category:** <c>`, `**Status:** <s>`, etc. The parser
  accepts the canonical form emitted by today's prompts and rejects
  anything outside.
- Hand-written prose in description bodies is preserved verbatim into
  the YAML `description:` block scalar.
- The `\n---\n[HANDOFF]` separator in current Results blocks is detected
  and lifted into the typed `handoff:` field.

### Migration tests

- **Round-trip:** `migrate LLM_STATE/core`; assert `state backlog list`,
  `state memory list`, `state session-log list` record counts match source
  hand-counts; assert all body fields preserved byte-for-byte in block scalars.
- **Idempotency:** run twice; second invocation is a no-op (exit 0, no
  filesystem changes).
- **Parse failure:** feed a malformed `.md`; assert no `.yaml` is written
  and exit code is non-zero.
- **Dry-run:** run with `--dry-run`; assert no filesystem changes.
- **Related-projects append:** run `migrate-related-projects` on plan A,
  then on plan B; assert plan B's edges are appended to (not clobbering)
  plan A's edges in the global file.
- **Catalog collision:** two plans with the same directory basename; assert
  the second `migrate-related-projects` invocation errors cleanly.

## Prompt migration scope

Eight prompt files need edits to swap Read+Edit patterns for CLI calls:

- `defaults/phases/work.md`
- `defaults/phases/analyse-work.md`
- `defaults/phases/reflect.md`
- `defaults/phases/dream.md`
- `defaults/phases/triage.md`
- `defaults/phases/create-plan.md`
- `defaults/survey.md`
- `defaults/survey-incremental.md`

Per-prompt edit density is roughly 5–15 instruction rewrites. The scope is
bounded; the `{{RELATED_PLANS}}` substitute-tokens injection point keeps the
same name (projection output shape preserves plan paths).

## Deliverable #3 — subsumed by the R1 implementation plan

The original research-task brief called for a narrow PoC (just
`migrate-backlog` + `list --status not_started`) to validate parser
feasibility. During review the scope was upgraded to full production
R1 (complete `state backlog` verb surface + `state migrate <plan-dir>`
backlog-scoped + tests).

See `docs/structured-backlog-r1-plan.md` for the TDD-by-task
implementation plan. The parser-feasibility validation the original
PoC would have delivered lives in the plan as Task 4's
`parses_live_core_backlog_without_error` test — it's a superset of what
the PoC promised.

## Rollout plan

| # | Task | Size | Dependencies |
|---|------|------|--------------|
| R1 | Full `state backlog` verb surface + migration + tests | medium | — |
| R2 | Full `state memory` verb surface + migration + tests | small | — |
| R3 | `state session-log` + `latest-session.yaml` + `GitCommitWork` rewire + migration + tests | medium | — |
| R4 | `state projects` catalog + auto-add on `ravel-lite run` + tests | small | — |
| R5 | `state related-projects` global edge list + `migrate-related-projects` + tests | medium | R4 |
| R6 | Update all 8 phase/survey prompts to use CLI verbs | medium | R1–R5 |
| R7 | LLM-driven discovery for related-projects (subagent parallelism + SHA cache) — **separate design-ish task** | large | R5 |
| R8 | Move per-plan task-count extraction into Rust (previously-blocked backlog task) | small | R1 |

R1–R5 ship the verbs; R6 flips the prompts; R7 is an independent discovery
feature; R8 is unblocked as a side effect.

## Evaluation

| Criterion | Estimate |
|-----------|----------|
| **Context savings — work phase** | Large. `list --status not_started --ready` returns ~10 lines vs ~450 today. |
| **Context savings — analyse-work** | Moderate. `list --missing-results` returns a short list; most Edit-anchor reads eliminated. |
| **Context savings — reflect** | None. Holistic memory read. |
| **Context savings — dream** | None. Holistic memory read. |
| **Context savings — triage** | Small. Holistic backlog read, but `list --has-handoff` shortcuts the hand-off scan. |
| **Tool-call delta — work** | Similar call count, but each call is more reliable (no Edit-anchor string matching). |
| **Tool-call delta — analyse-work safety-net** | Large win: N Read+Edit pairs → 1 `list --missing-results` + N `set-status`. |
| **Invariant coverage** | Status vocabulary enforced at parse. `--reason` required for blocked. Missing Results queryable. Dangling `dependencies:` detectable. Edge endpoints must resolve via catalog. Handoff shape pinned. |
| **Implementation cost** | ~1500–2500 LOC Rust (parser is the bulk; CLI dispatch + serde is plumbing). |
| **Prompt-update cost** | 8 files, ~5–15 edits each. |
| **Principle cost** | None. README principle "All config, prompts, phase state, and memory are readable files on disk" still satisfied. |

## Recommendation — go

Context savings are real where they matter (work-phase task selection,
analyse-work safety-net). Reliability gains from structured mutation are
arguably bigger than the raw token savings — no more exact-string Edit
failures, no more status-value typos, no more stale Status drift when
Results is non-empty. Scope is bounded and incrementally deliverable
(R1–R6 are each one-session tasks). The reframe (structured native, not
markdown-as-source) keeps the parser as a one-shot migration concern
rather than a permanent reader/writer consistency burden, materially
reducing lifetime cost vs the original framing.

## Deferred / out of scope

- **LLM-driven discovery** for relationships (R7). Substantial subagent
  orchestration + cache-invalidation design; captured as its own backlog
  task.
- **Comment-preservation round-trip** on YAML writes. Rejected in favour
  of canonical write; revisit only if a real debugging workflow needs it.
- **Subagent-dispatch.yaml** is already YAML and out of this redesign's
  scope — no verbs proposed for it.
- **Fixed-memory** (`../ravel-lite-config/fixed-memory/`) is static
  documentation and not plan-state; not in scope.
