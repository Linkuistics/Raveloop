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

### Work-phase transcript truncated by incoming phase banner

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

When a work session ends and the next phase (e.g. `analyse-work`)
begins, the final lines of the work transcript are cut off by the
incoming phase header banner. Reported from a `racket-oo` plan session;
the bug is in raveloop itself, not the plan.

Observed output:

```
  │ 4    │ PDF loads + renders         │ make-pdfdocument-init-with-url, pdfview-set-document!                │

────────────────────────────────────────────────────
  ◆  ANALYSE  ·  racket-oo
  Examine git diff, write session log and commit message
────────────────────────────────────────────────────
```

The bottom border of the table (and any trailing lines after it) are
missing — the ANALYSE banner has displaced them. The visible scrollback
is misleading because the reader cannot see where the work phase
actually ended, which also means `analyse-work` consumers and human
reviewers see a truncated record.

Likely suspects:
- Ratatui progress renderer not flushing/scroll-preserving the tail of
  the child process's output before tearing down the work view.
- Phase banner printed before a final newline/flush of the preceding
  agent's stdout stream.
- Terminal-mode transition (alternate screen ↔ main screen, or
  raw-mode toggle) dropping the last lines on restore.

Reproduce by running any plan through a work → analyse-work transition
whose final stdout lines include a wide table or other output flush
right at the phase boundary, then inspect the rendered scrollback.

Fix should guarantee every byte emitted by the work-phase subprocess
is visible in the final scrollback before the next banner is drawn.

**Results:** _pending_

---

### Show project name alongside plan name in phase header banner

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Phase header banners currently render as `◆ REFLECT · core` (see
`docs/architecture.md` TUI Layout section, e.g. `◆ REFLECT ·
mnemosyne-orchestrator`). Many plans share the name `core` — `raveloop/
LLM_STATE/core`, sibling projects' `LLM_STATE/core`, etc. — so the
banner alone does not tell the user which project is running, which
matters when multiple sessions are up or when reviewing scrollback.

Add the project name (the git repo root basename) to the banner, e.g.
`◆ REFLECT · raveloop / core` or `◆ REFLECT · core · raveloop`. The
`PlanContext` already carries both `project` and `plan` (see
`src/types.rs`), so the banner formatter just needs both.

Touch points: wherever the phase header string is composed for the log
(likely `src/phase_loop.rs` or `src/ui.rs`), and any test that asserts
on the banner text. Related to the transcript-truncation task above —
both live in the same render seam, so a dev picking up either should
check the other.

**Results:** _pending_

---

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
**Status:** `not_started`
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

**Results:** _pending_

---

### Fail loudly on unresolved `{{tokens}}` in prompt substitution

**Category:** `bug`
**Status:** `not_started`
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

**Results:** _pending_

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
**Status:** `not_started`
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

**Results:** _pending_

---

### Sync `docs/architecture.md` Message Model with the real `UIMessage` enum

**Category:** `docs`
**Status:** `not_started`
**Dependencies:** none

**Description:**

The Message Model section of `docs/architecture.md` (~lines 58–86)
documents variants that no longer match the code:

- `RegisterAgent { agent_id, header }` in docs — the real enum
  (`src/ui.rs`) has only `agent_id`; headers now flow through `Log`.
- `Progress` / `Persist` / `RegisterAgent` field shapes differ from
  the `StyledLine`-based variants in the code.

Update the doc to reflect the actual enum, callsites, and any implicit
ordering invariants (e.g. whether `AgentDone` must fence a later
`Log`). If the current enum is what we want, tighten the doc. If the
doc's richer shape is what we want, adjust the code — but first record
the decision here.

**Results:** _pending_

---

### Split `src/survey.rs` (1287 LOC) along natural seams

**Category:** `refactor`
**Status:** `not_started`
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

**Results:** _pending_

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

### Embed new language coding-style files in `init`

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`defaults/fixed-memory/` now contains coding-style files for every
language the user works in: `coding-style-swift.md`,
`coding-style-typescript.md`, `coding-style-python.md`,
`coding-style-bash.md`, `coding-style-elixir.md` (in addition to the
existing `coding-style-rust.md`). They are not yet wired into the
product — `src/init.rs` only embeds the rust file, so `raveloop init`
does not write the new files into a user's config dir, and the
work-phase agent therefore cannot find them at runtime.

The work-phase prompt (`defaults/phases/work.md` lines 36–42) already
does language-based lookup of `coding-style-<lang>.md`, so the only
change required is registering each new file as an `EmbeddedFile` in
the `EMBEDDED_FILES` constant in `src/init.rs` (alongside the existing
`coding-style-rust.md` entry on line 27). After this, `raveloop init`
(and `raveloop init --force`) will install the new files automatically.

Touch points:
- `src/init.rs` — five new `EmbeddedFile` entries, one per language
- Consider whether the embedded-file roster deserves a unit test that
  asserts every `defaults/fixed-memory/coding-style-*.md` on disk is
  registered, so future additions can't drift out of sync silently.

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

### Work-phase commit can land meta-only commits that claim source work

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

A work-phase session can produce a commit whose message narrates
substantial source changes (specific files, line counts, fixes) while
the commit itself only touches `LLM_STATE/<plan>/` bookkeeping files
(`backlog.md`, `phase.md`, `work-baseline`). The described source
changes remain unstaged in the working tree. Subsequent `reflect` /
`triage` / `git-commit-triage` phases do not catch this — none of
them diff the actual commit payload against the claims in the commit
message, the backlog `Results` block, or the session-log entry.

Observed in a Modaliser-Racket plan run. Commit `06941ab` (message:
"Fix pipeline teardown: group-kill all subprocess spawn sites;
eliminate ffi/unsafe") described edits to ~17 source + test files,
but its staged diff was three files:

```
LLM_STATE/modaliser/backlog.md    | 58 +++++++++++++++++++++++----
LLM_STATE/modaliser/phase.md      |  2 +-
LLM_STATE/modaliser/work-baseline |  2 +-
```

The real changes (~276 insertions / 623 deletions across `ffi/`,
`services/`, `tests/`, `ui/`, `main.rkt`) sat in the working tree
across subsequent reflect/triage/git-commit-triage cycles without
being noticed, until a fresh-context work session read `git status`
and recovered them as `ad84de7 Recover missing payload from 06941ab`.
Between the two commits the plan's backlog was "empty", the phase
transitions all succeeded, and tests were never re-run against HEAD.

Proximate cause is most likely a narrow `git add <pathspec>` in the
work phase's commit step that missed the source files. The deeper
issue is the lack of any postcondition that compares claimed work to
committed diff.

Candidate guards (not mutually exclusive):

1. **Rust-side postcondition at commit boundaries.** After the
   work-phase commit, assert that `git diff --name-only
   <baseline>..HEAD` contains at least one non-`LLM_STATE/` path if
   the just-completed task's category is anything other than
   `meta`/`docs` limited to plan state. Likely home:
   `src/phase_loop.rs` right after the commit phase closes.

2. **Triage-level verification.** In `git-commit-triage` (or
   whichever phase drives the commit), fail loudly when
   `git status --porcelain` after the commit still shows unstaged,
   pre-existing source changes whose paths also appear in the commit
   message. This would have caught 06941ab on the spot.

3. **Prompt-level guardrail.** Update `defaults/phases/work.md` (and
   the commit phase prompt) to require the agent, before writing
   `phase.md`, to run `git diff --stat --staged` and confirm that
   every file path mentioned in the commit message or the task
   `Results` block appears in the staged diff.

4. **Work-baseline sanity tick.** When advancing `work-baseline` to
   HEAD at phase boundaries, diff the new baseline against the
   previous one: if the delta is `LLM_STATE/**` only but the most
   recent task claimed non-meta work, surface a warning.

Touch points:

- `src/phase_loop.rs` — phase-boundary postcondition (#1, #4).
- `defaults/phases/work.md`, `defaults/phases/git-commit-triage.md`
  — prompt-level guardrails (#2, #3).
- `tests/integration.rs` — regression test: simulate a work session
  that edits source files but stages only `LLM_STATE/` before
  committing, assert the guard triggers.

The failure mode is silent, crosses multiple phase boundaries, and
masks lost work as "backlog empty" — worth catching at more than one
seam.

**Results:** _pending_

---
