# Backlog

## Tasks

### 5c — Multi-plan `run` mode with survey-driven routing

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5b ✓ (incremental survey complete); 5d ✓ (clean runner architecture)

**Description:**

Turn `ravel-lite run` into a multi-plan orchestrator when given N
positional plan-dir args. At the top of every cycle, run an
incremental survey over all N plans, present the top-ranked plans to
the user via a minimal stdout prompt, and dispatch one phase cycle of
the user's choice before looping back. Replaces the LLM-driven
coordinator concept with a code-driven routing loop. See
`docs/survey-pivot-design.md` §5c.

**Deliverables:**

1. `run` accepts `N > 1` positional plan dirs. `N == 1` remains
   exactly as today (no survey, no state file, unchanged behaviour).
2. New required flag for `N > 1`: `--survey-state <path>`. Rejected
   when `N == 1`. The file is both output (written at cycle end) and
   input (read as `--prior` next cycle via 5b's incremental path).
3. Run-loop shape: **survey → select → dispatch one cycle → repeat**.
   Survey is the first operation of every iteration; no separate
   cold-start branch (cold vs incremental is internal to the survey
   call based on whether `--survey-state` already exists).
4. Minimal selection UI: plain stdout listing of top-ranked plans
   with ordinals, plan identifiers, and rationales; single stdin
   read for the user's numeric choice. No ratatui widget — a richer
   TUI selection experience is a separate future enhancement.
5. Dispatch: a single invocation of the existing `phase_loop` for
   the selected plan directory; return to the top of the run loop
   on completion.
6. Tests: integration test that exercises the full
   survey→select→dispatch→re-survey loop with fake plans;
   validation that `--survey-state` is required for `N > 1` and
   rejected for `N == 1`; state-file round-trip across invocations.

**Results:** _pending_
