# Backlog

## Tasks

### Set an explicit default model for the work phase (claude-code)

**Category:** `bug`
**Status:** `not_started`
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

**Results:** _pending_

---

### Propagate filesystem errors from `write_phase`

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`src/phase_loop.rs:~25` currently swallows the result of the `phase.md`
write with `let _ = fs::write(...)`. If the write fails (permissions,
full disk, stale handle) the loop proceeds with stale state and the
agent is re-invoked on the same phase, wasting compute and hiding the
real error.

Return a `Result` from `write_phase`, propagate up, and render the
error in the TUI log before exiting. Small change, but the loop's
invariants depend on it.

**Results:**

Changed `write_phase` to return `Result<()>` and attach
`Context("Failed to write phase marker: <path>")`. All five
callers in `handle_script_phase` (GitCommitWork, GitCommitReflect's
two arms, GitCommitDream, GitCommitTriage) now propagate via `?`;
since each already returns `Result<bool>` or `Result<()>`, no call
site needed structural change. Added two unit tests:
`write_phase_writes_marker_file` (happy path) and
`write_phase_errors_when_directory_is_missing` (error surface names
the target file). Full test suite: 128 passed.

---

### Fail loudly on unresolved `{{tokens}}` in prompt substitution

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`src/prompt.rs` substitutes a fixed set of tokens via string replace
and returns the rendered template with no validation that every
`{{...}}` was consumed. A typo in a phase prompt (e.g. `{{PLANN}}`
instead of `{{PLAN}}`) therefore reaches the LLM verbatim and is
usually not noticed in review.

After all substitutions, scan the final string for leftover `{{...}}`
patterns; either log a warning to the TUI or hard-error depending on
desired strictness. Add a unit test covering the detection. This
check would have caught the `{{MEMORY_DIR}}` case in its sibling task.

**Results:**

Chose hard-error over warning: the `{{MEMORY_DIR}}` precedent was
precisely a silent-drift failure, and `compose_prompt` already returns
`Result`, so propagation is free. `substitute_tokens` now returns
`Result<String>` and scans the post-substitution string with a
cached `regex::Regex` matching `{{[A-Za-z0-9_]+}}` (the punctuation
restriction prevents false positives on Rust format specifiers like
`{x}` and hyphenated-non-tokens like `{{not-a-token}}`). Unresolved
names are collected into a `BTreeSet` so the error message is sorted
and de-duplicated. Added 4 unit tests covering: happy path, single
unresolved token, multiple tokens (sorted + deduped), and the
single-brace / hyphen false-positive avoidance. Updated the two
existing tests to `.unwrap()` the new Result. Verified all existing
`{{…}}` in `defaults/` are legitimate tokens — no escaping needed.
Full test suite: 128 passed.

---

### Decide pi agent scope: complete the port or mark it aspirational

**Category:** `meta`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Multiple audit findings point to pi being a visibly less-polished
sibling to claude-code:

- Unresolved `{{MEMORY_DIR}}` in `memory-prompt.md`.
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
and `{{PLAN}}`. The literal `{{MEMORY_DIR}}` passes through to the LLM
unchanged, silently corrupting the memory-handling instructions pi
sees.

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

### Timeout the `claude` invocation in `raveloop survey`

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`src/survey.rs:~545` awaits `stdout.read_to_string(&mut output)` with
no timeout. If `claude` hangs or the model stalls, `raveloop survey`
hangs forever with no feedback. The subcommand is advertised as single-
shot and read-only, so a hang is doubly surprising.

Wrap the read (or the whole invocation) in `tokio::time::timeout`.
Surface an error that names the elapsed time, preserves whatever
stdout was captured so far, and suggests the usual remediations
(re-run, swap `--model`, check network). A reasonable default might
be 5 minutes; make it overridable via flag or env if it turns out to
be too tight.

**Results:** _pending_

---

### Surface claude stream-JSON parse errors instead of silently skipping

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`ClaudeCodeAgent`'s stream reader does `serde_json::from_str(line).ok()?`
(src/agent/claude_code.rs:~72), silently dropping any line that doesn't
parse. Combined with the 4 KB rolling stderr buffer (line ~214) that
also discards earlier content without a marker, failures in claude's
output format become invisible.

Emit a one-line `Persist` (or `Log`) entry per dropped stream line,
carrying the first N bytes of the offending text, and annotate the
stderr-buffer overflow with a single warning the first time it wraps.
Both should be unobtrusive — the user just needs a reliable signal
that something was discarded.

**Results:**

Replaced `parse_stream_line` → `Option<FormattedOutput>` with an
explicit `StreamLineOutcome { Output | Ignored | Malformed { snippet } }`.
The old `Option` collapsed "parsed JSON, nothing to display" and
"couldn't parse JSON" into the same `None`, which is exactly how the
bug was silent. Malformed lines now carry a UTF-8-boundary-safe
snippet capped at `STREAM_SNIPPET_BYTES = 200` (ellipsis appended on
truncation). The invoke_headless loop matches the enum and emits a
`Persist` warning via `tx` on `Malformed`. Stderr overflow: lifted
the magic 4096 into `STDERR_BUFFER_CAP`, added a `warned` flag inside
the draining tokio task so the first overflow emits a one-shot
`Persist` warning via a cloned `tx` before further drops silently
accumulate. Both warnings use a shared `warning_line(…)` helper
that builds a `⚠  …` styled line with `Intent::Changed` (yellow,
matching the existing `warn_if_project_tree_dirty` pattern in
phase_loop). Added 4 new tests: `parse_unhandled_event_type_is_ignored`
(guards against false-positives on valid non-tool events),
`parse_malformed_json_surfaces_snippet`,
`malformed_snippet_is_bounded_and_utf8_safe` (multibyte `café`
boundary), `truncate_snippet_passes_short_inputs_unchanged`. Updated
4 existing tests for the new enum via an `expect_output` helper.
Full test suite: 128 passed.

---

### Split `src/survey.rs` (1287 LOC) along natural seams

**Category:** `refactor`
**Status:** `done`
**Dependencies:** none

**Description:**

`survey.rs` is by far the largest source file and mixes several
concerns: plan discovery + project derivation, prompt composition,
claude subprocess invocation, YAML schema + deserialization, and
deterministic final rendering. Future changes (timeout wrapping,
parser strictness, output tweaks) will be easier with clean module
boundaries.

Candidate modules: `discover.rs` (walk + classify), `compose.rs`
(plan → bundle → prompt), `invoke.rs` (spawn/read claude), `schema.rs`
(YAML types + parse), `render.rs` (deterministic output). Tests should
split naturally along the same seams.

Do not change behavior or externally observable output as part of the
split; any improvements should land in separate, focused tasks.

**Results:**

Split into the five suggested modules under `src/survey/`:

- `survey.rs` (35 lines): module root, re-exports the library's
  public API (`run_survey`, `discover_plans`, `PlanSnapshot`,
  `load_survey_prompt`, `render_survey_input`).
- `discover.rs` (216 lines, ~130 of which are tests): `PlanSnapshot`,
  `project_name_for_plan`, `discover_plans`.
- `compose.rs` (117 lines): `SURVEY_PROMPT_PATH`, `render_survey_input`,
  `load_survey_prompt`.
- `schema.rs` (201 lines): `SurveyResponse`, `PlanRow`, `Blocker`,
  `ParallelStream`, `Recommendation`, `parse_survey_response`,
  `strip_code_fence`.
- `render.rs` (633 lines, ~400 of which are tests): all rendering
  helpers + `render_survey_output`.
- `invoke.rs` (157 lines): `DEFAULT_SURVEY_MODEL`, `resolve_model`,
  `run_survey`.

Tests migrated with their implementations — each module has its
own `#[cfg(test)] mod tests` with the per-concern helpers it needs
(`write_plan`/`mark_as_git_project` in discover, `sample_yaml` in
schema, `row` in render, `empty_agent_config` in invoke). No
production code changed; all 128 unit tests + 5 integration tests
pass, same counts as pre-split.

Binary-crate wrinkle: `main.rs` uses `mod survey;` rather than the
library crate, so the re-exports for `{load_survey_prompt,
render_survey_input, PlanSnapshot, discover_plans}` appear unused
when building the bin. Annotated those two re-export lines with
`#[allow(unused_imports)]` and a comment explaining why. The
library's `deny(warnings)` gate is satisfied; `run_survey` is used
directly and needs no annotation.

What this enables: follow-on tasks (timeout wrapping in
`raveloop survey`, parser strictness, output tweaks) can now touch
one module instead of navigating a 1287-line monolith. The
schema/render split in particular makes `parse_survey_response`
tests runnable without pulling in rendering code paths.

Pre-existing clippy warnings in format.rs and the `row` helper
were NOT addressed — they predate the split. If desired, a
follow-up task can batch them together.

---

### Wire up or remove orphaned `defaults/skills/*` files

**Category:** `meta`
**Status:** `not_started`
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

**Results:** _pending_

---

### Add integration coverage for the phase → file-write round-trip

**Category:** `feature`
**Status:** `not_started`
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

**Results:** _pending_

---
