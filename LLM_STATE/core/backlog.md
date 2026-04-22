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

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5 (done), R7-design (done)

**Description:**

Design complete. Spec at `docs/r7-related-projects-discovery-design.md`;
12-task TDD implementation plan at `docs/r7-related-projects-discovery-plan.md`.

Given a set of projects, dispatch LLM subagents in parallel to analyse each
project's interaction surface and propose sibling / parent-of edges. Two-stage
pipeline: Stage 1 extracts a structured surface record per project (subtree-scoped
git tree SHA cache key); Stage 2 is one global LLM call over all N surface
records that proposes edges. Output written to `discover-proposals.yaml`; a
separate `discover-apply` verb merges after human review (`discover --apply`
fuses both steps).

Ready to execute with `executing-plans` or `subagent-driven-development`.

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
