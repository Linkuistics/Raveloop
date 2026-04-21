# Memory

### Input hash uses length-prefixed concatenation
`PlanRow.input_hash` is SHA-256 over length-prefixed `phase.md + backlog.md + memory.md + related-plans.md`. Absent file hashes distinctly from empty file; this is intentional to detect missing vs. empty inputs.

### `PlanRow.input_hash` carries `#[serde(default)]`
The field is absent in LLM-emitted YAML and injected by the Rust harness post-parse. `#[serde(default)]` lets `parse_survey_response` accept both LLM output (no field) and harness-injected round-trips (field present).

### `schema_version` uses `#[serde(default)]` for backward compatibility
`SurveyResponse.schema_version: u32` carries `#[serde(default)]`, allowing `parse_survey_response` to read pre-version YAML (field absent) and current YAML (field present) without a migration step.

### Hash injection hard-errors on unknown or missing rows
`inject_input_hashes` treats undiscovered rows and missing rows as hard errors. No silent pass-through. Delta classification also refuses mutation outside the declared changed set.

### YAML is persistence; markdown is presentation
`run_survey` emits YAML only (`serde_yaml::to_string`). Markdown rendering lives exclusively in `survey-format`. Coupling them again would re-entangle two separate concerns.

### Survey CLI names plan dirs individually
`discover_plans` tree walk replaced by `load_plan(plan_dir)`. CLI positional args are `plan_dirs`. Callers enumerate plans explicitly; no implicit directory walk.

### `plan_key` and `input_hash` key delta classification
These are the keying primitives for delta logic. `parse_survey_response` is the single entry point for both LLM stdout and `--prior` file reads.

### Noop fast path skips LLM when no plans changed
`classification.is_noop()` carries the prior forward unchanged with no subprocess spawn. This makes every-cycle survey invocation affordable in the multi-plan run loop.

### `compute_survey_response` is the in-memory survey entry point
`run_survey` is a thin CLI wrapper over `compute_survey_response(...)`, which returns `Result<SurveyResponse>`. Multi-plan needs the response in-memory (for `recommended_invocation_order`) and on disk (for next cycle's `--prior`).

### `--survey-state` is the multi-plan state file path
`run_multi_plan` reads and writes `--survey-state <path>` as the survey YAML across cycles. `--prior` is `run_survey`'s per-invocation input flag; `--survey-state` is the `run` subcommand's flag that threads the file path through the plan loop.

### `SurveyResponse` hierarchy requires `Clone`
All nested structs (`PlanRow`, `SurveyResponse`, and their fields) require `#[derive(Clone)]` for the merge and noop-carry paths. Extending the hierarchy requires propagating `Clone`.

### Multi-plan dispatch lives in `src/multi_plan.rs`
`run_multi_plan` is the survey→select→dispatch→re-survey loop. `dispatch_one_cycle` handles per-cycle TUI setup, `phase_loop` invocation, and teardown. `select_plan_from_response` is IO-parameterised via `BufRead`+`Write` generics for in-memory test driving.

### `phase_loop` exits after one full cycle
`phase_loop` returns `Ok(false)` after one full cycle. `run_single_plan` wraps it in a loop and owns the inter-cycle `ui.confirm` prompt. Multi-plan uses `phase_loop` directly via `dispatch_one_cycle`, bypassing the prompt.

### `Run` CLI validates plan-count against `--survey-state`
`N==1` plan dir rejects `--survey-state`; `N>1` requires it. Validation fires before any state file is written.

### `merge_delta` errors surface immediately in the run loop
`merge_delta` refusal (delta mutates plans outside the declared changed set) propagates directly to the user rather than being silently retried. Model drift is user-visible information, not a transparent retry case.

### Six pre-existing clippy doc-formatting errors in `src/survey/schema.rs`
`cargo clippy` reports 6 doc-formatting warnings in `src/survey/schema.rs`. These predate the survey restructure and are out of scope for this plan.

### Per-plan task counts: deferred until core task #3 (structured backlog parser) settles
The design doc flagged moving per-plan task-count extraction from LLM prompt to Rust as a follow-on. This is intentionally deferred: the right implementation depends on how `core/backlog.md` task #3 (structured backlog parser) resolves. Once that parser exists, task counts can be derived from it rather than inferred by the survey LLM. Do not implement until core task #3 is complete.

### Plan directory pending manual archive
After the survey-restructure plan's final cycle completes, `LLM_STATE/survey-restructure/` should be moved to `LLM_STATE/archive/survey-restructure/`. A one-step `mv` after the triage phase exits is sufficient. This keeps session-logs discoverable without polluting active-plan routing.
