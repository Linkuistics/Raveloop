# Backlog

## Tasks

### R6 — Migrate all phase prompts to use CLI verbs

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1, R2, R3, R4, R5 (all verbs now exist — all done)

**Description:**

Replace direct `Read` / `Edit` of plan-state files with `ravel-lite state <verb>`
calls across `defaults/phases/work.md`, `analyse-work.md`, `reflect.md`,
`dream.md`, `triage.md`, `create-plan.md`, `defaults/survey.md`,
`defaults/survey-incremental.md`. ~5–15 instruction rewrites per file. Prompts
keep the `{{RELATED_PLANS}}` token (projection shape preserves plan paths).

Also wire cascade from `projects::run_rename` into `related-projects.yaml`
(currently catalog-only; fits naturally alongside related-projects rewiring).

**Atomicity caveat:** `.yaml` plan-state files diverge from `.md` files between
migration time and the prompt cutover — `.md` remains the operational data source
until R6 lands. Before rewriting phase prompts, run
`ravel-lite state migrate <plan-dir> --force --delete-originals` so the `.yaml`
files reflect the latest `.md` state at the moment of cutover. The re-migration
and the prompt rewrite must land in the same commit.

**Results:** _pending_

---

### R7-design — Design spike for LLM-driven related-projects discovery

**Category:** `research`
**Status:** `not_started`
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

**Results:** _pending_

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
