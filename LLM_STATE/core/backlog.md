# Backlog

## Tasks

### Add integration test exercising the pi phase path

**Category:** `test`
**Status:** `not_started`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

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

**Results:** _pending_

---

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `not_started`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

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

`STDERR_BUFFER_CAP` and `warning_line` are currently duplicated across
`pi.rs` and `claude_code.rs` with comments pointing here. The pi
integration test (task above) forms the regression net for this
refactor alongside the existing `ClaudeCodeAgent` test surface.

**Results:** _pending_

---
