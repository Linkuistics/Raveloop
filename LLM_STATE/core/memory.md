# Memory

## All prompt loading routes through `substitute_tokens`
Ad-hoc `str::replace` bypasses the hard-error guard regex. Any prompt-loading path that does not delegate to `substitute_tokens` silently passes unresolved tokens through. Drift guards require one canonical substitution path.

## `shipped_pi_prompts_have_no_dangling_tokens` test guards pi prompts
The test iterates every on-disk pi prompt file and asserts no unresolved tokens remain. Enforcement mechanism for the canonical-substitution-path rule.

## Config overlays use deep-merge via `load_with_optional_overlay<T>()`
`src/config.rs` implements `*.local.yaml` overlays. Deep-merge: scalar collisions go to overlay, map collisions recurse. A `models.work: ""` overlay blanks only that key without losing sibling keys.

## Pi subagent definitions live at `agents/pi/subagents/`
`defaults/agents/pi/subagents/` holds pi subagent definitions (brainstorming, tdd, writing-plans); `init.rs` embed paths and `pi.rs` reads reference this location.

## `init.rs` drift-detection test guards coding-style registration
The test reads `defaults/fixed-memory/coding-style-*.md` at test time and asserts every file on disk is registered as an `EmbeddedFile`. Adding a new coding-style file without registering it fails the test.

## `embedded_defaults_are_valid` asserts non-empty model and provider
Every (agent, phase) pair in `defaults/agents/claude-code/config.yaml` must have a non-empty model string and non-empty provider. The test catches omissions that would silently delegate model/provider selection to the spawn context.

## Work phase must not commit source files
`work.md` step 8 explicitly tells the work phase to leave non-plan paths dirty; source-commit authority belongs to analyse-work. A session that commits source in work is a contract violation.

## Analyse-work commits all non-plan paths
`analyse-work.md` step 6 requires staging and committing every path outside the plan dir, or justifying each skipped path in the session log.

## Analyse-work receives dirty-tree snapshot as `{{WORK_TREE_STATUS}}`
`phase_loop.rs` calls `git::work_tree_snapshot(project_dir, baseline_sha)` when entering `LlmPhase::AnalyseWork` and inserts the result under `{{WORK_TREE_STATUS}}`. Captured at prompt-compose time so hand-edits made after work exits are included.

## `StreamLineOutcome` enum distinguishes ignored vs malformed stream lines
Replacing `Option<FormattedOutput>` with an enum makes `valid but no display` and `parse failure` distinguishable. Apply this pattern wherever an `Option` return collapses two semantically distinct outcomes into one.

## Survey stdout read has 300s timeout
`src/survey/invoke.rs` wraps the stdout read in `tokio::time::timeout` (`DEFAULT_SURVEY_TIMEOUT_SECS = 300`); on expiry the child is killed and the error includes elapsed time, captured bytes, partial stdout, and remediations. Override via `--timeout-secs`.

## Phase contract test validates per-phase file writes
`phase_contract_round_trip_writes_expected_files` runs `phase_loop` from `analyse-work` via `ContractMockAgent`; 6 assertions cover latest-session.md, commit-message.md consumed, memory.md updated, backlog.md updated, phase.md ends at `work`, and git log subjects.

## `substitute_tokens` expands content macros before path tokens
RELATED_PLANS and custom tokens expand first; atomic path tokens ({{DEV_ROOT}} etc.) expand second. Reversing the order causes fatal errors when RELATED_PLANS content itself contains path tokens.

## Pi stderr captured in 4096-byte rolling buffer
`PiAgent::invoke_headless` pipes stderr into a fixed-size rolling buffer (`STDERR_BUFFER_CAP = 4096`). Tail surfaces in error messages on failure; eliminates TUI bleed-through during headless invocation.

## `pi_phase_cycle` test guards runtime token substitution
`pi_phase_cycle_substitutes_tokens_and_streams_events` runs a full `phase_loop` cycle with a real `PiAgent` and a fake `pi` shell script; asserts zero unresolved `{{…}}` tokens in the captured prompt, correct `UIMessage` variant fan-out (`Progress`, `Persist`, `AgentDone`), and audit commit via `commit-message.md`.

## `pi_invoke_headless` test guards stderr-tail surfacing
`pi_invoke_headless_surfaces_stderr_tail_on_failure` asserts a non-zero `pi` exit (code 17) surfaces the stderr tail in the returned error. Guards the buffered-stderr fix. See: `Pi stderr captured in 4096-byte rolling buffer`.

## `pi_dispatch_subagent` test pins dispatch argv contract
`pi_dispatch_subagent_invokes_pi_with_target_plan_args` pins the exact argv for `dispatch_subagent`: `--no-session`, `--append-system-prompt`, `--provider anthropic`, `--mode json`, `-p`, prompt.

## `EnvOverride` serialises env mutation in integration tests
`EnvOverride` holds a process-wide `OnceLock<Mutex<()>>`; struct-field drop order keeps the lock held until `PATH`/`HOME` restoration completes, preventing fake-pi `PATH` from leaking into concurrent test runners.

## `GitCommitWork` appends `latest-session.md` to `session-log.md`
`append_session_log` in `phase_loop.rs` runs at the top of `GitCommitWork`, before `write_phase`. Reads `latest-session.md` and appends to `session-log.md`. Tail-check makes it idempotent; crash-retry is safe.

## `write_phase` precedes `git_commit_plan` in all commit handlers
All four `ScriptPhase::GitCommit*` handlers in `phase_loop.rs` call `write_phase(next)` before `git_commit_plan`. Phase.md is captured in the same commit as other plan-state writes; the plan tree is clean at every user-prompt point.

## Work-baseline seeded atomically in the triage commit
`GitCommitTriage` calls `git_save_work_baseline` before committing. `LlmPhase::Work` seeds work-baseline only when the file is absent (first-run fallback).

## `LlmPhase::Work` does not delete `latest-session.md`
Analyse-work overwrites `latest-session.md` unconditionally on entry; a deletion in the Work handler is decorative and was removed.

## Plan-tree cleanliness asserted via `git status --porcelain`
`git_commit_triage_leaves_plan_tree_clean_at_user_prompt` and `git_commit_work_leaves_plan_tree_clean_at_user_prompt` assert `git status --porcelain -- <plan_dir>` is empty after `phase_loop` returns from a user-declined exit.

## `spawn_blocking` does not cancel cleanly in `tokio::select!`
Use `tokio::time::sleep` for tty event polling. A `spawn_blocking` thread is not dropped when the select arm is cancelled; it races the spawned child for the tty. `tokio::time::sleep` is properly cancellable and eliminates the race.

## `dream-baseline` seeded from three call sites
`seed_dream_baseline_if_missing` in `src/dream.rs` is called from `run_create` (post-session scaffolding), `run_set_phase` (every LLM phase transition including coordinators), and `GitCommitReflect`. Seeds to 0 ("never dreamed"), not current word count; seeding to current count silently delays the first dream by `headroom` words on populated plans.

## Claude Code TUI requires `--debug-file` workaround for ≤2.1.116
`invoke_interactive` in `src/agent/claude_code.rs` passes `--debug-file /tmp/claude-debug.log`; debug mode masks a TUI rendering failure via an unknown upstream mechanism. Root cause not found. Remove both `args.push` lines when claude is updated past 2.1.116.

## Phase prompts invoke `ravel-lite state set-phase`
All 5 `defaults/phases/*.md` prompts use `ravel-lite state set-phase <plan-dir> <phase>` to transition phase. Direct writes to `phase.md` bypass `Phase::parse` validation; LLM phases must use the CLI. `run_set_phase` also refuses to create a plan dir that does not already exist.

## CLI integration tests use `CARGO_BIN_EXE_ravel-lite`
`tests/integration.rs` shells out to the binary via the `CARGO_BIN_EXE_ravel-lite` env var and asserts on-disk effects. This is the Cargo-idiomatic pattern for end-to-end CLI testing without `std::process::Command` hardcoding.

## `build.rs` emits timestamp, git-describe, and SHA
`build.rs` emits `BUILD_TIMESTAMP`, `GIT_DESCRIBE`, and `GIT_SHA` as compile-time env vars; `main.rs` concatenates them into a `VERSION` constant used by both `--version` and the `version` subcommand.

## `release.toml` disables publish and push
`release.toml` sets `publish=false` and `push=false`; `cargo release patch --execute` bumps the version and tags locally without touching crates.io or the remote.

## `term_title.rs` sets phase title via OSC escape
`src/term_title.rs` exposes `set_title(project, plan, phase)` (side-effecting) and `format_title_escape` (testable helper). Writes to stdout; clean side-channel because Ratatui uses stderr backend. Includes tmux passthrough (doubled inner ESCs). Called at `LlmPhase` entry in `phase_loop` and `run_single_plan`. Phase labels uppercased to match the phase-header banner convention.

## Reflect phase runs automatically after git-commit-work
The runner proceeds from `GitCommitWork` to reflect without user confirmation. No pre-reflect gate exists; reflect is unconditional after work commit.

## Fake-pi script must be phase-aware
The fake-pi script in `pi_phase_cycle` uses a case statement on the current phase to emit distinct next-phase values. Writing `git-commit-work` unconditionally causes an infinite loop; each phase must map to a different output.

## `ContractMockAgent` for Triage appends, not overwrites
`ContractMockAgent::invoke_headless` uses append mode for `Triage` so the safety-net test can observe analyse-work's status flips written earlier in the same cycle.

## Hand-off fields are live in analyse-work and triage prompts
`defaults/phases/analyse-work.md` and `defaults/phases/triage.md` include the hand-off convention. Analyse-work writes a hand-off block; triage reads it on entry. See: `[HANDOFF] integration tests guard promote-vs-archive cycle`.

## `[HANDOFF]` integration tests guard promote-vs-archive cycle
`handoff_marker_in_analyse_work_is_promoted_by_triage` and `handoff_marker_in_analyse_work_is_archived_by_triage` in `tests/integration.rs` run the full analyse-work → git-commit-work → reflect → git-commit-reflect → triage → git-commit-triage cycle. CI-protects the hand-off convention. See: `ContractMockAgent has opt-in handoff injection`.

## `ContractMockAgent` has opt-in handoff injection
`handoff_injection: Option<HandoffInjection>` field with `with_handoff_injection()` builder. `HandoffDisposition` inside `HandoffInjection` pins promote-vs-archive without hardcoding LLM reasoning, keeping hand-off tests deterministic.

## Task blocks delimited by `\n---` separator
`inject_handoff_into_task_block` and `extract_handoff_from_block` both split on `\n---`. Protocol for separating the main task block from an appended hand-off block.

## `warn_if_project_tree_dirty` is advisory-only
`warn_if_project_tree_dirty` at `GitCommitWork` entry has no user-facing gate; the warning fires but the phase proceeds unconditionally. If this becomes unacceptable, promote to a hard error rather than re-adding a gate.

## `phase_loop` is single-cycle; `run_single_plan` holds the inter-cycle prompt
`phase_loop` is a single-cycle function. The inter-cycle user prompt ("continue / switch plan / exit") lives in `run_single_plan`, which wraps `phase_loop` in a loop. Multi-plan dispatch calls `phase_loop` directly via `dispatch_one_cycle` in `src/multi_plan.rs`.

## `src/multi_plan.rs` owns survey-driven multi-plan routing
`src/multi_plan.rs` implements `build_plan_dir_map`, `options_from_response`, `select_plan_interactive`, and `run_multi_plan`. `ravel-lite run` accepts 1..N plan dirs; `--survey-state` is required when N > 1. Routes to the next plan via `dispatch_one_cycle`; the dispatch loop replaced the former LLM-authored coordinator-plan concept.

## `ravel-lite survey` emits structured YAML
`src/survey/schema.rs` defines the output schema with `Serialize` derives and a `schema_version` marker. `input_hash` is seeded in Rust post-parse. `survey-format` subcommand renders YAML output to human-readable form.

## Incremental survey splits `invoke.rs` into two functions
`compute_survey_response` is the in-memory core; `run_survey` is the CLI wrapper. `src/survey/delta.rs` owns hash-comparison and delta-merge. `--prior` names the baseline state; `--force` skips the hash guard. `defaults/survey-incremental.md` is the prompt template for the delta path.

## `push-plan`, `stack.yaml`, and `src/pivot.rs` removed
`push-plan` CLI verb, `run_stack`, `stack.yaml`, and `src/pivot.rs` are deleted. `src/state.rs` reduced from ~230 to ~80 lines; `src/phase_loop.rs` de-pivoted. Multi-plan routing is exclusively in `src/multi_plan.rs`.

## `project_root_for_plan` derives project root as `<plan>/../..`
Pure path math; no disk walk, no `.git` requirement. Contract: `<plan>` must be at least three path components deep.

## Git query functions scope to project dir via pathspec
`working_tree_status`, `paths_changed_since_baseline`, and `work_tree_snapshot` append `-- <project_dir>` pathspec. `git_commit_plan` is intentionally unscoped; it runs from `plan_dir` CWD, already limiting `git add .` to plan-state files.

## Integration tests use three-level `<project>/LLM_STATE/<plan>` layout
`tests/integration.rs` and `src/multi_plan.rs` tests construct `<project>/LLM_STATE/<plan>` directory trees matching ravel-lite convention. New integration tests must follow this layout.

## `format_result_text` renders `→` as continuation lines
Lines matching `^\s*→\s*(.*)` immediately after an action marker are re-indented to the detail column and styled with the preceding action's intent. Blank lines, insight blocks, and non-continuation lines break the chain. `last_action_intent: Option<Option<Intent>>` encodes "no prior action" (outer None) vs "prior action with no intent" (Some(None)).

## `PROMOTED` and `ARCHIVED` are valid action tags
`ACTION_INTENTS` in `src/format.rs` includes `PROMOTED` and `ARCHIVED` as triage hand-off markers; they emit new backlog tasks or memory entries.

## Work phase allows multiple tasks when requested
`work.md` step 10 allows multiple tasks per session when the user explicitly requests them; the default remains single-task-per-phase.

## Dream output uses label + `→` continuation format
`defaults/phases/dream.md` specifies a two-line entry layout (label line + `→ detail` continuation) matching the continuation-line renderer in `format_result_text`.

## `related-plans` stored as global name-indexed edge list
Project-level (not plan-level) edge list, keyed by plan name. Plans reference each other by name; the global list is shareable across all plans in a project.

## `latest-session.md` redesigned as structured YAML
The structured plan-state design replaces the prose `latest-session.md` format with structured YAML. R1 implements this change.

## Plan-state migration requires atomicity, idempotency, dry-run, validation
Any plan-state migration tool must apply changes atomically, be safe to re-run (idempotent), support `--dry-run` preview, and validate round-trip fidelity.

## Structured plan-state design at `docs/structured-plan-state-design.md`
Q1–Q8 design decisions for `ravel-lite state <file> <verb>` CLI. See also: R1 implementation plan at `docs/structured-backlog-r1-plan.md` (13-task TDD-by-task, covers `state backlog` verb surface and backlog-scoped migrate).

## `src/projects.rs` holds `ProjectsCatalog`
`ProjectsCatalog` (schema_version 1) maps project names to absolute paths. `auto_add` is pure and returns `AlreadyCatalogued`/`Added`/`NameCollision`. `ensure_in_catalog_interactive` is generic over `Read + Write`. Atomic save.

## `state projects add` rejects relative paths
`add` enforces absolute paths; relative paths resolve differently from different CWDs, so the catalog is path-anchored. Rejection is a hard error at CLI entry.

## `state projects rename` is catalog-only
`rename` updates the catalog name only. Cascade into `related-projects.yaml` is deferred to R5.

## `register_projects_from_plan_dirs` runs before TUI startup
Called in `Commands::Run` before Ratatui alternate-screen takeover so any `NameCollision` prompt reaches a real tty.
