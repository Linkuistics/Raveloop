# Backlog

## Tasks

### 5b — Incremental survey via `--prior`

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5a ✓ (canonical YAML round-trip + `input_hash` field complete)

**Description:**

Add a `--prior <file>` flag to `ravel-lite survey` so a prior YAML
survey can seed an incremental run: compare per-plan `input_hash`
values, send only changed+added plans to the LLM, and merge the delta
with unchanged rows from the prior. Per-cycle survey cost becomes
proportional to what actually changed — a precondition for 5c's
every-cycle survey to be affordable. See `docs/survey-pivot-design.md`
§5b.

**Deliverables:**

1. `--prior <file>` flag on `survey`. Parse the prior YAML; classify
   each plan as `unchanged` / `changed` / `removed` / `added` by
   comparing freshly-computed `input_hash` values against the prior.
2. Delta-aware `render_survey_input` in `src/survey/compose.rs`: only
   changed+added plans appear in the LLM payload. The prior survey
   is carried in full as context so the LLM can revisit cross-plan
   blockers and parallel streams when deltas affect them.
3. Merge logic in `run_survey`: LLM delta + prior-unchanged rows →
   final `SurveyResponse`. Validation refuses a delta that mutates a
   plan outside the declared changed set (mirrors `inject_input_hashes`
   hard-error pattern).
4. `--force` bypass flag: re-analyses everything regardless of hash
   match. For debugging and schema-bump paths.
5. Prompt strategy — settle during implementation; lean: two prompts
   (`defaults/survey.md` cold, `defaults/survey-incremental.md` warm)
   beats one with conditional branches. Embed via
   `src/init.rs::EMBEDDED_FILES`; preserve drift-guard coverage.
6. Add `schema_version: u32` to `SurveyResponse` with
   `#[serde(default = "default_schema_version")]` so 5a-emitted YAML
   without the marker still parses once 5b lands. Mismatched-version
   `--prior` either fails fast with a remediation hint or
   auto-falls-back to `--force`-equivalent behaviour.
7. Tests: unchanged-plan reuse, changed-plan re-analysis,
   removed-plan pruning, added-plan detection, schema-bump
   invalidation, `--force` path, validation-rejects-delta-outside-
   changed-set.

**Results:** _pending_

---

### 5c — Multi-plan `run` mode with survey-driven routing

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5b (incremental survey for affordable per-cycle
invocation); 5d ✓ (clean runner architecture)

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
