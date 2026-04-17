# Backlog

## Tasks

### Decide pi agent scope: complete the port or mark it aspirational

**Category:** `meta`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Multiple audit findings point to pi being a visibly less-polished
sibling to claude-code:

- Unresolved `{{MEMORY_DIR}}` in `memory-prompt.md`. Now that
  `substitute_tokens` hard-errors on unresolved tokens, pi invocation
  **fails immediately** rather than silently corrupting instructions —
  resolving this is no longer deferred cleanup, it is a hard blocker
  on using pi at all.
- stderr not captured on failure (no tail in error messages).
- Older default model (`claude-opus-4-6`) in
  `defaults/agents/pi/config.yaml` vs claude-code's more current
  `claude-sonnet-4-6` / haiku variants.
- No integration test exercises the pi agent path.

Pick a direction: either invest in parity (and cover it in tests +
docs) or mark pi explicitly aspirational in `README.md` /
`docs/architecture.md` so future readers don't assume drop-in
equivalence. If we commit to parity, extract the genuinely shared
spawn/stream/dispatch boilerplate from `claude_code.rs` and `pi.rs`
into `src/agent/common.rs` as part of that effort.

**Results:** _pending_

---

### Resolve or remove `{{MEMORY_DIR}}` token in pi memory prompt

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** Decide pi agent scope

**Description:**

`defaults/agents/pi/prompts/memory-prompt.md` references `{{MEMORY_DIR}}`
at three sites (lines ~3, 61, 74) but `PiAgent::load_prompt_file`
(src/agent/pi.rs:~142) only substitutes `{{PROJECT}}`, `{{DEV_ROOT}}`,
and `{{PLAN}}`. Previously the literal `{{MEMORY_DIR}}` passed through
to the LLM unchanged; now that `substitute_tokens` hard-errors on
unresolved tokens, pi invocation fails immediately on any phase that
loads this prompt.

Decide whether memory lives in a distinct directory from the plan (if
so, thread `MEMORY_DIR` through `PlanContext` and the pi token map) or
rewrite the prompt to use `{{PLAN}}` and drop the placeholder. Also
grep the prompt for any other dangling `{{...}}` while you're there.

**Results:** _pending_

---

### Capture and surface pi subprocess stderr on non-zero exit

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** Decide pi agent scope

**Description:**

`PiAgent::invoke_headless` (src/agent/pi.rs:~236) uses
`stderr(Stdio::inherit())`, which lets pi's error output bypass the TUI
log and bleed into the raw terminal during a headless phase — often
overwritten immediately by later TUI repaints. `ClaudeCodeAgent`
(src/agent/claude_code.rs:~199) already pipes stderr, accumulates a
tail buffer, and includes the tail in its `anyhow::bail!` on failure.

Port the same pattern to `PiAgent`. When touching the code, consider
extracting `drain_stderr_into_buffer(..)` into a shared helper (e.g.
`src/agent/common.rs`) so the two copies can't drift.

**Results:** _pending_

---

### Reliably mark completed backlog items as `done`

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Completed tasks are unreliably transitioning from `Status:
not_started` (or `in_progress`) to `Status: done` in `backlog.md`.
The work-phase prompt currently says only "Record results on the
task in `{{PLAN}}/backlog.md`: what was done, what worked, what
didn't, what this suggests next." (`defaults/phases/work.md:76-78`).
It never explicitly instructs the agent to flip the `Status:` line.
When the model focuses on writing the `Results:` block, the status
field silently stays stale — which then misleads triage into
treating a finished task as still open, wasting a future cycle or
(worse) causing duplicate work.

Two complementary fixes to implement together:

1. **Tighten `defaults/phases/work.md`.** Change step 7's wording so
   it names the status transition as a required part of the
   recorded result: e.g. "Update the task's `Status:` line to
   `done` (or `blocked` with a reason) and write a `Results:`
   block beneath it covering what was done, what worked, what
   didn't, and what this suggests next." Make the status update
   the *first* sub-bullet so a hurried model sees it even if it
   skims.

2. **Add a safety net in `defaults/phases/analyse-work.md`.**
   Analyse-work already reads `backlog.md` (step 4) and the diff
   (steps 2-3). Extend it: after determining the session produced
   a non-empty `Results:` block on a task whose `Status:` is still
   `not_started` or `in_progress`, flip the status to `done`
   before writing `latest-session.md`. Describe this as a
   post-condition check, not a judgement call — the diff is
   authoritative, and analyse-work runs against the diff.

Also add integration coverage: extend (or parallel-copy)
`phase_contract_round_trip_writes_expected_files` so the
`ContractMockAgent`'s analyse-work branch writes a `Results:`
block on a task that was `not_started` and leaves its `Status:`
line unchanged; the test then asserts that after analyse-work
runs, the status is `done`. That pins the safety-net behaviour.

**Suggests next:** If the tightened work-phase prompt alone closes
the gap in practice, the analyse-work safety net can be left as
belt-and-braces. If the gap persists, the third lever is a
`Status:`-flip check in `warn_if_project_tree_dirty`-style code
inside `phase_loop`, surfaced as a TUI warning — but that's a
bigger change and should only follow if prompt-level fixes fail.

**Results:** _pending_

---
