# Backlog

## Tasks

### Run `state migrate --delete-originals` to remove legacy `.md` plan-state files

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** R8 (done)

**Description:**

R8 is complete: all four Rust readers that formerly accessed `.md` plan-state
files directly have been migrated. `src/dream.rs` and `src/survey/discover.rs`
use the typed YAML API; `src/multi_plan.rs` and `src/main.rs` access only
`related-plans.md` (not a `--delete-originals` target) and `phase.md`.

Run:

```
ravel-lite state migrate --delete-originals LLM_STATE/core
```

Verify no `.md` plan-state originals remain and that `ravel-lite run` still
operates correctly against `LLM_STATE/core` after deletion.

**Results:** _pending_

---

### R7-design — Design spike for LLM-driven related-projects discovery

**Category:** `research`
**Status:** `done`
**Dependencies:** R5 (done)

**Description:**

R7 is explicitly flagged as requiring a design pass before implementation.
Conduct a brainstorm → spec → plan cycle covering:

- How subagents are dispatched in parallel per-project (dispatch contract,
  result aggregation)
- SHA-based cache key design (what content is hashed, where the cache lives,
  invalidation strategy)
- Edge-proposal schema (how subagents return proposed edges for merge into
  `related-projects.yaml`)
- Conflict / duplication handling when multiple subagents propose the same edge

Output: a written spec and implementation plan for R7.

**Results:**

Spec written to `docs/r7-related-projects-discovery-design.md`; implementation
plan written to `docs/r7-related-projects-discovery-plan.md`. Both committed as
part of this work phase (hand-off to analyse-work). R7 remains `not_started`
and is unblocked — the plan is ready to execute in a future work phase.

Key decisions settled during the brainstorm (captured in the spec):

- **Entry point:** new verb pair under the existing `state related-projects`
  command group (`discover` + `discover-apply`), not a bolt-on to `survey`.
  Keeps discovery philosophically next to the rest of the related-projects
  surface and leaves room to auto-invoke from a phase later without replumbing.
- **Fanout shape:** one subagent per project, no peer context supplied.
  Per-project input keeps the cache key trivially clean — per-user-provided
  peer summaries would create transitive cache invalidation (any peer change
  busts every cache).
- **Two-stage pipeline:** Stage 1 per-project extracts a structured
  interaction-surface record (`purpose`, `consumes_files`, `produces_files`,
  `network_endpoints`, `data_formats`, `external_tools_spawned`,
  `explicit_cross_project_mentions`, `notes`). Stage 2 is one global LLM call
  over all N surface records that proposes edges. Two stages because
  relationships via shared file formats or network protocols — where neither
  project names the other — are invisible from a single-project vantage and
  require global reasoning.
- **Input surface:** the whole project, not a curated file subset. User flagged
  that most catalogued projects will have no plan (so `memory.md` isn't
  available) and that first-principles reading is needed. Subagents may
  dispatch nested sub-subagents at their own discretion for large projects.
- **Cache key:** subtree-scoped git tree SHA via `git rev-parse HEAD:<rel>`
  where `<rel>` is the project path relative to the repo toplevel. Works
  identically for top-level repos and monorepo subtrees; a sibling subtree's
  commit does not bust this project's cache. Bail on non-git and on dirty
  subtree (subtree-scoped dirty check via the existing
  `git::working_tree_status` pathspec mechanics).
- **Merge policy:** review-gate. Discover writes proposals to
  `<config-dir>/discover-proposals.yaml` with rationale; a separate
  `discover-apply` merges via `RelatedProjectsFile::add_edge`. Kind-conflicts
  (proposed `sibling(A,B)` when `parent-of(A,B)` already exists) are reported
  on stdout, rejected, and do not abort the apply. `discover --apply` fuses
  the two steps for scripted use.
- **Operational:** Stage 1 concurrency bounded by `tokio::sync::Semaphore`
  (default 4; `--concurrency N` override). `--project <name>` restricts
  Stage 1 to one project while Stage 2 still operates over the full catalog's
  cached surfaces. Failure handling is best-effort — per-project Stage 1
  failures surface in `discover-proposals.yaml`'s `failures:` section;
  overall exit is non-zero when any failure occurred but the successful
  surfaces still flow into Stage 2.
- **Deferred to R7 execution time:** whether to widen `Agent::dispatch_subagent`'s
  target parameter or add a sibling `dispatch_project_subagent`; whether to
  split `src/discover.rs` on the first pass or wait for it to outgrow one
  file; whether `chrono` joins `Cargo.toml` (plan assumes yes) or timestamps
  are formatted via `std::time`.

The plan decomposes R7 into 12 bite-sized TDD tasks (module scaffold → schema
→ tree-SHA → cache → rename cascade → Stage 1 → Stage 2 → top-level
orchestrator → apply → CLI wiring → integration test → closeout). Each task
has explicit file paths, complete code, exact test commands, and a commit
step. Ready to execute with either `executing-plans` or `subagent-driven-development`.

User elected to defer R7 execution to a separate work phase rather than
execute inline, preserving reviewable plan-first artefacts and keeping
single-task-per-work-phase discipline.

---

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5 (done), R7-design

**Description:**

Given a set of projects, dispatch LLM subagents in parallel to analyse each
project's README / backlog / memory and propose sibling / parent-of edges.
SHA-based cache (keyed on per-project content hash) avoids re-analysing
unchanged projects. Output merges into the global `related-projects.yaml`.

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

**Caveat — a version bump alone is insufficient.** The fix is empirical: debug
mode masks the TUI failure via an unknown upstream mechanism, not a documented
patch. A later claude version may have bumped past 2.1.116 without actually
touching the offending code path. Before removing the workaround:

1. Reproduce the original TUI failure on the current binary *without* the flag
   (run `ravel-lite run` against a real plan, watch the Work phase render).
2. If the bug no longer reproduces without the flag, adding the flag should
   also make no observable difference — confirm that.
3. Only then remove the two `args.push` lines.

An attempt on claude 2.1.117 was rolled back unverified — the code change is
trivial (27-line deletion, produced by a subagent, reverted via `git checkout`)
but the TUI verification step cannot be done by a subagent (no tty) and was
not done by a human.

**Results:** _pending_

---
