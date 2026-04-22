# Backlog

## Tasks

### R3 — Implement `state session-log` + `latest-session.yaml` + GitCommitWork rewire

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** R1 (done)

**Description:**

Adds `state session-log` verbs (list, show, append, set-latest, show-latest),
makes `latest-session.yaml` a typed file (same record shape as session-log
entries), rewires `phase_loop::GitCommitWork` to parse the new YAML + append
to `session-log.yaml`'s `sessions:` list with session-id idempotency. Extends
`state migrate` to cover session-log + latest-session.

Slots into `migrate.rs` as a third `PendingMigration` variant with no
structural change to the parse-all-then-write-all contract.

**Results:**

Shipped. New `src/state/session_log/` module mirrors R1/R2 layout (schema,
yaml_io, parse_md, verbs, mod). `SessionRecord { id, timestamp, phase, body }`
is shared by both `session-log.yaml` (wrapped in `SessionLogFile { sessions }`)
and `latest-session.yaml` (single record at rest). `append_record` idempotency
is session-id based (stronger than the old tail-string check) and treats a
missing file as empty — fresh plans work without explicit init.

`state migrate` grew two new `PendingMigration` variants (`SessionLog`,
`LatestSession`) via peer functions `plan_session_log_migration` /
`plan_latest_session_migration`. Parse-all-then-write-all invariant preserved.
A dry-run against live `LLM_STATE/core/` parsed 7 backlog + 65 memory + 10
sessions + 1 latest cleanly — the production format is fully covered, so the
manual migration is safe to run whenever convenient.

`phase_loop::append_session_log` now delegates to
`session_log::append_latest_to_log`; graceful no-op when `latest-session.yaml`
is absent; idempotent on crash-retry.

Tests: 347 lib (+32 for `state::session_log::*`, +7 for migrate), 27
integration (including updated `phase_contract_round_trip_writes_expected_files`),
4 new CLI tests in `tests/state_session_log.rs`. `clippy --all-targets -- -D
warnings` clean.

**Suggests next:** R6 (migrate phase prompts to CLI verbs) is now only blocked
on R5. Manual migration of `LLM_STATE/core/{session-log,latest-session}.{md→yaml}`
is a 1-command step (`ravel-lite state migrate`) that can happen any time —
worth scheduling alongside the eventual R1/R2 on-disk migrations.

---

### R5 — Implement global `state related-projects` edge list + `migrate-related-projects`

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R4 (done — catalog exists; names ↔ paths resolution is now available)

**Description:**

Global `../ravel-lite-config/related-projects.yaml` edge list (sibling /
parent-of), name-indexed, shareable between users. CLI: `state related-projects
list [--plan <path>]`, `add-edge`, `remove-edge`. `state migrate-related-projects
<plan-dir>` one-shot merges a plan's legacy `related-plans.md` into the global
file, creating it on first call and deduping by (kind, participants).

**Results:** _pending_

---

### Move per-plan task-count extraction from LLM survey prompt into Rust

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none (R1 is done — the structured backlog parser now exists)

**Description:**

The survey LLM currently infers per-plan task counts from the raw markdown in
`backlog.md`. Now that the structured backlog parser from R1 exists, task counts
(total, not_started, in_progress, done) can be computed directly in Rust and
injected as pre-populated tokens into the survey prompt — removing an
unnecessary inference burden from the LLM.

**Deliverables:**

1. Extend the structured backlog parser to expose a `task_counts() -> TaskCounts`
   method.
2. In `src/survey/discover.rs`, compute task counts from the parsed backlog
   and inject them into `PlanRow` (replacing the LLM-inferred field).
3. Update `defaults/survey.md` to remove the instruction asking the LLM
   to count tasks; add a note that counts are pre-populated.
4. Test: assert counts are correct for a plan with tasks in each status.

**Results:**

Shipped. `TaskCounts { total, not_started, in_progress, done, blocked }` lives
in `src/state/backlog/schema.rs`, exposed by `BacklogFile::task_counts()`.
`PlanRow` grew `task_counts: Option<TaskCounts>`; `load_plan` populates it by
parsing `backlog.md` via `parse_backlog_markdown`, falling through to `None` on
absent / malformed files. `inject_task_counts` wires cold and incremental
survey paths (`compute_survey_response` + `merge_delta` carries counts
verbatim through unchanged rows). Both `defaults/survey.md` and
`defaults/survey-incremental.md` now forbid the LLM from emitting
`task_counts` — the field is Rust-injected.

**Design note:** the original spec implied `TaskCounts` would *replace* an
LLM-inferred field, but `PlanRow`'s existing `unblocked`/`blocked`/`done`/
`received` fields require dep-traversal reasoning that is genuinely LLM work.
Kept those intact; added `task_counts` additively. Downstream renderers can
migrate at their leisure rather than in a big-bang.

Tests: 5 new (2 on `task_counts()`, 2 on `inject_task_counts`, 3 on
`load_plan`'s absent/malformed/present branches). 347 lib + 27 integration +
13 CLI all green; clippy clean.

**Suggests next:** the additive `task_counts` field creates a small cleanup
opportunity — the LLM-inferred `unblocked`/`blocked`/`done` on `PlanRow` could
be removed once every downstream consumer has migrated, collapsing duplicate
count data. Not urgent.

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

**Results:** _pending_

---

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5

**Description:**

Feature design + implementation. Given a set of projects, dispatch LLM
subagents in parallel to analyse each project's README / backlog / memory and
propose sibling / parent-of edges. SHA-based cache (keyed on per-project
content hash) avoids re-analysing unchanged projects. Output merges into the
global `related-projects.yaml`.

Large — probably needs its own design-ish pass (brainstorm → spec → plan)
before implementation.

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

### Fix `iter_cloned_collect` clippy lint in `backlog/parse_md.rs:227`

**Category:** `maintenance`
**Status:** `done`
**Dependencies:** none

**Description:**

R2 identified a pre-existing clippy lint (`iter_cloned_collect`) at
`src/state/backlog/parse_md.rs:227` that was left untouched as out of scope
for that work. Resolve the lint — replace the redundant `.cloned().collect()`
with a direct `.collect()` or equivalent idiomatic form.

**Results:**

Fixed. Replaced `body_lines.iter().copied().collect::<Vec<_>>().join("\n")`
with `body_lines.join("\n")` — `[&str]::join` works directly on the slice, so
the intermediate `Vec` was pure ceremony. Clippy clean under `-D warnings`.

---
