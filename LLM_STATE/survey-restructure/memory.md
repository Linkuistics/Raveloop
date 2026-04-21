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
`classification.is_noop()` carries the prior forward unchanged with no subprocess spawn. This makes every-cycle survey invocation affordable in 5c's plan loop.

### `--prior` doubles as survey state file for 5c
`run_survey` accepts `--prior <file>` for input and should write the same path as output. This single-file round-trip is the intended integration point for `run_single_plan` in `phase_loop.rs`.

### `SurveyResponse` hierarchy requires `Clone`
All nested structs (`PlanRow`, `SurveyResponse`, and their fields) require `#[derive(Clone)]` for the merge and noop-carry paths. Extending the hierarchy requires propagating `Clone`.

### `run_single_plan` is the seam for 5c multi-plan dispatch
`run_single_plan` in `src/phase_loop.rs` is a 9-line delegate retained intentionally. Task 5c branches on plan-count in `main::run_phase_loop`: single-plan path calls `run_single_plan` unchanged; multi-plan path adds a survey-routed dispatch loop around it.

### `merge_delta` validation must be surfaced in 5c's run loop
`merge_delta` refuses deltas that mutate plans outside the declared changed set ("expected keys == returned keys"). Task 5c's run loop should surface this error directly to the user on first occurrence rather than silently retrying — model drift is user-visible information, not a transparent retry case.

### Six pre-existing clippy doc-formatting errors in `src/survey/schema.rs`
`cargo clippy` reports 6 doc-formatting warnings in `src/survey/schema.rs`. These predate the survey restructure and are out of scope for this plan.
