# Backlog

## Tasks

### R5 — Implement global `state related-projects` edge list + `migrate-related-projects`

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** R4 (done — catalog exists; names ↔ paths resolution is now available)

**Description:**

Global `../ravel-lite-config/related-projects.yaml` edge list (sibling /
parent-of), name-indexed, shareable between users. CLI: `state related-projects
list [--plan <path>]`, `add-edge`, `remove-edge`. `state migrate-related-projects
<plan-dir>` one-shot merges a plan's legacy `related-plans.md` into the global
file, creating it on first call and deduping by (kind, participants).

**Results:**

New module `src/related_projects.rs` + integration test
`tests/state_related_projects.rs`; `main.rs` gains two subcommand trees.

- File lives at `<config_root>/related-projects.yaml`
  (`schema_version: 1`). Schema: a flat `edges:` list with
  `kind: sibling | parent-of` and a uniform `participants: [A, B]` pair.
  Dedup key is kind-aware — sibling uses sorted participants
  (order-insensitive), parent-of keeps participant order (direction is
  part of the identity). `Edge::validate` rejects self-loops and
  non-pair participant counts.
- CLI surface:
  - `state related-projects list [--plan <path>] [--config <path>]` —
    emits YAML; `--plan` filters to edges that involve the project
    derived as `<plan>/../..`.
  - `state related-projects add-edge <kind> <a> <b>` — refuses unknown
    project names and points at `state projects add` in the error.
  - `state related-projects remove-edge <kind> <a> <b>` — errors if the
    edge is absent; sibling removal honours order-insensitivity.
- `state migrate-related-projects <plan-dir> [--config] [--dry-run]
  [--delete-original]` parses the legacy `related-plans.md` (sections
  `## Siblings`, `## Parents`, `## Children`, bullet lines with
  ` — description` tail), substitutes `{{DEV_ROOT}}` / `{{PROJECT}}` /
  `{{PLAN}}` tokens, resolves each bullet path to a project (auto-add
  via `projects::auto_add`, bails on `NameCollision` with actionable
  `state projects add` guidance), and merges the derived edges into the
  global file with the same dedup semantics. Directionality: "Parents"
  → `parent-of [peer, me]`; "Children" → `parent-of [me, peer]`;
  "Siblings" → `sibling [me, peer]`. Parse-all-then-write-all: the
  catalog and the edge file are touched only after every peer has
  resolved. Idempotent on re-run (every edge comes back as
  `already present`).
- 27 unit tests in `src/related_projects.rs` + 4 end-to-end tests in
  `tests/state_related_projects.rs` (using `CARGO_BIN_EXE_ravel-lite`).
  `cargo test` (all 418 tests) and `cargo clippy --all-targets` both
  green.

Not done (deliberate, per memory and R6's description):
- Phase prompts still read `related-plans.md` — `multi_plan.rs`,
  `survey/discover.rs`, and the `{{RELATED_PLANS}}` token expansion in
  `prompt.rs` remain unchanged. The operational data source for the
  prompts stays `.md` until R6 migrates them over.
- `projects::run_rename` still does not cascade into
  `related-projects.yaml` (documented caveat already in main.rs;
  follow-up is minor and fits naturally alongside R6 rewiring).

Next suggested task: **R6** (migrate all phase prompts to use CLI
verbs). R7-design and R7 also unblock, but R6 is the higher-leverage
follow-up because it closes the `.md`/`.yaml` divergence gap the memory
flags.

---

### R7-design — Design spike for LLM-driven related-projects discovery

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5

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

**Results:** _pending_

---

### R6 — Migrate all phase prompts to use CLI verbs

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1, R2, R3, R4, R5 (all verbs must exist before prompts can invoke them)

**Description:**

Replace direct `Read` / `Edit` of plan-state files with `ravel-lite state <verb>`
calls across `defaults/phases/work.md`, `analyse-work.md`, `reflect.md`,
`dream.md`, `triage.md`, `create-plan.md`, `defaults/survey.md`,
`defaults/survey-incremental.md`. ~5–15 instruction rewrites per file. Prompts
keep the `{{RELATED_PLANS}}` token (projection shape preserves plan paths).

**Atomicity caveat:** `.yaml` plan-state files diverge from `.md` files between
migration time and the prompt cutover — `.md` remains the operational data source
until R6 lands. Before rewriting phase prompts, run
`ravel-lite state migrate <plan-dir> --force --delete-originals` so the `.yaml`
files reflect the latest `.md` state at the moment of cutover. The re-migration
and the prompt rewrite must land in the same commit.

**Results:** _pending_

---

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5, R7-design

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
