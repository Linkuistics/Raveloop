# Backlog

## Tasks

### Capture and surface pi subprocess stderr on non-zero exit

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`PiAgent::invoke_headless` (src/agent/pi.rs:~236) uses
`stderr(Stdio::inherit())`, which lets pi's error output bypass the TUI
log and bleed into the raw terminal during a headless phase — often
overwritten immediately by later TUI repaints. `ClaudeCodeAgent`
(src/agent/claude_code.rs:~199) already pipes stderr, accumulates a
tail buffer, and includes the tail in its `anyhow::bail!` on failure.

Port the same pattern to `PiAgent`. When touching the code, consider
extracting `drain_stderr_into_buffer(..)` into a shared helper (e.g.
`src/agent/common.rs`) so the two copies can't drift. (Task 7 covers
the full shared-helper extraction; this task can either inline the
port and defer extraction, or co-land a narrow helper — either is
fine.)

**Results:** _pending_

---

### Bump pi default models to match claude-code

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`defaults/agents/pi/config.yaml` sets `models.work: claude-opus-4-6`
while `defaults/agents/claude-code/config.yaml` uses
`claude-opus-4-7`. Bring pi's per-phase models in line with
claude-code's defaults so a user swapping `agent: claude-code` for
`agent: pi` gets equivalent capability, not a silent downgrade.

Mirror the claude-code config: `work: claude-opus-4-7`, and verify the
other phases (`analyse-work`, `reflect`, `dream`, `triage`) use the
same models the claude-code defaults do (currently `claude-sonnet-4-6`
in both — already aligned, but re-verify at implementation time in
case claude-code's defaults have moved). Also review the `thinking:`
map: `work: medium` is pi-specific (claude-code does not use this
field) but the other `thinking` phases are blank, which may or may
not be intended — sanity-check against pi-coding-agent's own defaults.

Landing this task before the `embedded_defaults_are_valid` extension
means that test can assert the specific canonical values; landing them
in the other order means it only asserts non-empty strings. Either
sequencing is acceptable.

**Results:** _pending_

---

### Extend `embedded_defaults_are_valid` to cover pi config

**Category:** `test`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Memory records that `embedded_defaults_are_valid` asserts every
(agent, phase) pair in `defaults/agents/claude-code/config.yaml` has a
non-empty model string — a cheap guard against silent model-omission
regressions. No equivalent guard exists for pi, which is how the
`claude-opus-4-6` staleness survived unnoticed until audit.

Extend the existing test (or add a sibling test) to load
`defaults/agents/pi/config.yaml` and assert the same invariant for
every (agent, phase) pair. Consider also asserting a non-empty
`provider` string, since `PiAgent::build_headless_args` defaults it to
`"anthropic"` if missing — making that an explicit config requirement
rather than an implicit fallback eliminates a drift source.

**Results:** _pending_

---

### Add integration test exercising the pi phase path

**Category:** `test`
**Status:** `not_started`
**Dependencies:** Capture and surface pi subprocess stderr on non-zero exit

**Description:**

The existing `phase_contract_round_trip_writes_expected_files` test
runs `phase_loop` with `ContractMockAgent` — it verifies the phase
state machine writes the right files but does not exercise either the
`ClaudeCodeAgent` or `PiAgent` concrete implementations. As a result,
changes like `substitute_tokens` hard-erroring on unresolved tokens
broke pi end-to-end without a single test failing.

Add a contract-level test that runs a phase cycle with `PiAgent` as
the agent, using a mock `pi` binary on PATH (or a test double) that
emits a scripted stream of JSON events. At minimum the test should
cover: prompt loading resolves every `{{…}}` token (catches
`{{MEMORY_DIR}}`-class regressions), stream parsing maps events to
`UIMessage` variants correctly, stderr-tail appears in error messages
on non-zero exit (so the stderr capture fix stays fixed), and dispatch
invokes the right args for the target plan.

Depends on "Capture and surface pi subprocess stderr on non-zero exit"
being done so the pi path boots end-to-end and `stderr` is piped —
without that landing first, the test has nothing to assert about
stderr.

**Results:** _pending_

---

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `not_started`
**Dependencies:** Capture and surface pi subprocess stderr on non-zero exit

**Description:**

`src/agent/claude_code.rs` (492 LOC) and `src/agent/pi.rs` (510 LOC)
duplicate significant boilerplate around process spawn, stdout line
reading, stderr-tail buffering, subagent dispatch scaffolding, and
UIMessage emission patterns. The duplication is a drift source — e.g.
one agent getting an `anyhow::bail!` improvement that the other
silently misses.

Identify genuinely shared logic (process spawn helper, stdout
line-read loop that forwards to the right formatter, stderr drain +
tail buffer with size cap, dispatch pattern scaffolding) and lift
into a new `src/agent/common.rs`. Leave agent-specific surface — CLI
flag construction, JSON event parsing (different schemas between the
two agents) — in the concrete `*.rs` files.

Must land AFTER "Capture and surface pi subprocess stderr on non-zero exit"
so the stderr-tail helper has two real callsites to justify its
existence — extracting a helper with a single caller is premature
abstraction per the universal coding-style rules. The `ClaudeCodeAgent`
test surface and the pi integration test together form the regression
net for this refactor.

**Results:** _pending_

---
