# Backlog

## Tasks

### Auto-create missing parent directories in `create` subcommand

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`raveloop create <plan-dir>` currently hard-errors in `validate_target`
(src/create.rs:58-63) when the parent of the target plan directory does
not exist, telling the user to create it first and re-run. This is
unnecessarily strict — the user's intent is clear and refusing to
create `LLM_STATE/some-plan` just because `LLM_STATE/` is missing adds
friction with no benefit.

Change the behavior to auto-create missing parent directories (any
depth) before spawning `claude`. The parent must actually exist on disk
at spawn time because `--add-dir <parent>` resolves the path eagerly.

The existing test `validate_target_rejects_when_parent_missing`
(src/create.rs:170-175) pins the current behavior and will need to be
inverted or replaced.

**Results:** _pending_

---

### Review current state and expand backlog

**Category:** `meta`
**Status:** `not_started`
**Dependencies:** none

**Description:**

This backlog was seeded with a single concrete task. Use a work session
to audit the codebase (`src/`, `defaults/`, `docs/`, `tests/`) with
fresh eyes — identify gaps, rough edges, half-finished areas, and
aspirational features — then append concrete tasks in the standard
shape (title, category, status, dependencies, description, results) so
subsequent cycles have real work to pick from. Triage will prune and
reorder afterwards.

**Results:** _pending_
