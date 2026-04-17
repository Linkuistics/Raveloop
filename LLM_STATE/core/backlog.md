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

### Set an explicit default model for the work phase (claude-code)

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`defaults/agents/claude-code/config.yaml:2` has `work: ""` while every
other phase has a real model string. The interactive work phase is the
highest-leverage phase and deserves an explicit, auditable default; an
empty string delegates to whatever `claude` happens to pick at spawn
time.

Pick an explicit default matching the work phase's reasoning budget
(probably opus-class), update the embedded default, and add an
integration assertion (tests/integration.rs) that rejects any empty
model string in the embedded defaults so this doesn't silently recur.

**Results:** Set `work: claude-opus-4-7` in
`defaults/agents/claude-code/config.yaml`. Extended
`embedded_defaults_are_valid` in `tests/integration.rs` to iterate
every (agent, phase) pair in the embedded defaults and assert the
model string is present and non-empty; the test panics with the
specific agent/phase name if anyone reintroduces an empty string.
Verified by running `cargo test --test integration
embedded_defaults_are_valid`.

---

### Timeout the `claude` invocation in `raveloop survey`

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`src/survey/invoke.rs` awaits `stdout.read_to_string(&mut output)` with
no timeout. If `claude` hangs or the model stalls, `raveloop survey`
hangs forever with no feedback. The subcommand is advertised as single-
shot and read-only, so a hang is doubly surprising.

Wrap the read (or the whole invocation) in `tokio::time::timeout`.
Surface an error that names the elapsed time, preserves whatever
stdout was captured so far, and suggests the usual remediations
(re-run, swap `--model`, check network). A reasonable default might
be 5 minutes; make it overridable via flag or env if it turns out to
be too tight.

**Results:** Wrapped the stdout read in `tokio::time::timeout` with a
new `DEFAULT_SURVEY_TIMEOUT_SECS = 300` constant and a pure
`resolve_timeout(Option<u64>) -> Duration` helper (covered by two
unit tests alongside the existing `resolve_model_*` pattern). On
timeout the child is killed, and the error message includes the
elapsed time, the number of captured stdout bytes, the partial
stdout itself, and the usual remediation suggestions (re-run,
`--model`, network, `--timeout-secs`). Added a `--timeout-secs
<SECS>` CLI flag on `raveloop survey` that threads through to
`run_survey`. I/O errors mid-read now also kill the child instead
of leaking it.

---

### Wire up or remove orphaned `defaults/skills/*` files

**Category:** `meta`
**Status:** `done`
**Dependencies:** none

**Description:**

`defaults/skills/brainstorming.md`, `tdd.md`, and `writing-plans.md`
are embedded and written by `init`, but no phase prompt or other
default references them — a grep for "skill" across `defaults/` turns
up no matches. They become dead weight in every user's config
directory and mislead maintainers about whether skills are part of
the orchestrator's contract.

Pick one:
1. Delete the files from the defaults tree.
2. Reference them from the appropriate phase prompts (e.g. `work.md`
   directing the agent to consult `writing-plans.md` when drafting a
   plan).
3. Move them under `reference/` with a README clarifying they are
   optional, user-invocable material.

**Results:** The task's premise was partially wrong: the files aren't
truly orphaned — `src/agent/pi.rs` already reads every file under
`<config_root>/skills/` and deploys it to `<project_dir>/.pi/agents/`
as a pi-subagent definition (frontmatter parses `name`/`description`/
`tools`/`model`/`thinking`, matching pi's subagent format, not the
claude-code skill format). They are pi subagent definitions in a
generically-named directory.

Chose option 2 reinterpreted: moved the three files to
`defaults/agents/pi/subagents/`, updated `src/init.rs`
`EmbeddedFile` entries and `include_str!` paths to match, and
updated `src/agent/pi.rs` to read from the new location (renamed
the local `skills_dir` binding to `subagents_src`, updated error
messages and the nearby deploy comment). Also updated
`docs/architecture.md` (two places) to drop the `skills/` node from
both the repo tree and the installed-config tree, and to show the
new `agents/pi/subagents/` node with a note about its runtime
destination. Claude-code users no longer receive these pi-specific
files in their config dir at all; pi users continue to receive them
at `agents/pi/subagents/`, from which pi's setup step still deploys
them at session start. Full test suite and `cargo check
--all-targets` still pass.

---

### Add integration coverage for the phase → file-write round-trip

**Category:** `feature`
**Status:** `done`
**Dependencies:** none

**Description:**

`tests/integration.rs` has ~5 tests today; none of them validate that
the embedded phase prompts actually direct the agent to produce the
files the Rust code expects (`phase.md` transitions, session-log
append, `commit-message.md`, `latest-session.md`,
`subagent-dispatch.yaml`). If a prompt drifts to a wrong filename,
nothing catches it until a real run fails in the field.

Add a test that installs the defaults into a tempdir config, runs a
tiny mock `Agent` trait impl that writes files matching what a
well-behaved model *should* do per each phase prompt, and asserts the
expected files exist with expected contents. The test doubles as a
living executable description of the phase contract.

**Results:** Added `phase_contract_round_trip_writes_expected_files`
and a `ContractMockAgent` to `tests/integration.rs`. The test
installs the real embedded defaults via `raveloop::init::run_init`
into a tempdir config, seeds a plan at `phase=analyse-work` (work
is skipped because it is interactive), and runs `phase_loop` with
a mock that writes the files each phase prompt instructs a
well-behaved model to write: `latest-session.md` and
`commit-message.md` for analyse-work, a `memory.md` update for
reflect, a `backlog.md` update for triage, plus the
`phase.md` transitions. The drain thread approves "Proceed to
reflect phase?" and declines "Proceed to next work phase?" so the
full cycle runs once without entering interactive work. Six
assertions cover: latest-session.md exists with a Session heading,
commit-message.md was consumed by git-commit-work, memory.md was
updated by reflect, backlog.md was updated by triage, phase.md
ends at `work`, and git log contains the custom analyse-work
commit subject plus reflect and triage default subjects. Dream is
not exercised — memory is kept tiny and headroom set to 10 000 so
`should_dream` stays false. A future task could extend the mock
to also write `subagent-dispatch.yaml` and assert dispatch
behaviour, but that pulls in the real subagent runner and is a
separate contract.

**Suggests next:** If the pi-scope task resolves toward completing
the port, add a similar round-trip test covering pi's headless
path (different spawn/stream code). If the dream phase ever gets
its own prompt-driven file-write expectations beyond `memory.md`,
extend this test rather than duplicating the plumbing.

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
