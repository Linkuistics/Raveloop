# Memory

## All prompt loading routes through `substitute_tokens`
Ad-hoc `str::replace` bypasses the hard-error guard regex. Any prompt-loading path that does not delegate to `substitute_tokens` silently passes unresolved tokens through. Drift guards require one canonical substitution path.

## `shipped_pi_prompts_have_no_dangling_tokens` test guards pi prompts
The test iterates every on-disk pi prompt file and asserts no unresolved tokens remain. Enforcement mechanism for the canonical-substitution-path rule.

## Config overlays use deep-merge via `load_with_optional_overlay<T>()`
`src/config.rs` implements `*.local.yaml` overlays. Deep-merge: scalar collisions go to overlay, map collisions recurse. A `models.work: ""` overlay blanks only that key without losing sibling keys.

## Pi subagent definitions live at `agents/pi/subagents/`
`defaults/agents/pi/subagents/` holds pi subagent definitions (brainstorming, tdd, writing-plans). The former `defaults/skills/` location was a misnomer; `init.rs` embed paths and `pi.rs` reads are updated accordingly.

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
`pi_phase_cycle_substitutes_tokens_and_streams_events` runs a full `phase_loop` cycle with a real `PiAgent` and a fake `pi` shell script; asserts zero unresolved `{{…}}` tokens in the captured prompt, correct `UIMessage` variant fan-out (`Progress`, `Persist`, `AgentDone`), and audit commit via `commit-message.md`. Closes the gap that let the `{{MEMORY_DIR}}` regression escape.

## `pi_invoke_headless` test guards stderr-tail surfacing
`pi_invoke_headless_surfaces_stderr_tail_on_failure` asserts a non-zero `pi` exit (code 17) surfaces the stderr tail in the returned error. Guards the buffered-stderr fix. See: `Pi stderr captured in 4096-byte rolling buffer`.

## `pi_dispatch_subagent` test pins dispatch argv contract
`pi_dispatch_subagent_invokes_pi_with_target_plan_args` pins the exact argv for `dispatch_subagent`: `--no-session`, `--append-system-prompt`, `--provider anthropic`, `--mode json`, `-p`, prompt.

## `EnvOverride` serialises env mutation in integration tests
`EnvOverride` holds a process-wide `OnceLock<Mutex<()>>`; struct-field drop order keeps the lock held until `PATH`/`HOME` restoration completes, preventing fake-pi `PATH` from leaking into concurrent test runners.

## `write_phase` called before `git_commit_plan` in all `GitCommit*` handlers
All four `ScriptPhase::GitCommit*` handlers in `phase_loop.rs` call `write_phase(next)` before `git_commit_plan`. Phase.md is captured in the same commit as other plan-state writes; the plan tree is clean at every user-prompt point.

## Work-baseline seeded atomically in the triage commit
`GitCommitTriage` calls `git_save_work_baseline` before committing. `LlmPhase::Work` seeds work-baseline only when the file is absent (first-run fallback).

## `LlmPhase::Work` does not delete `latest-session.md`
Analyse-work overwrites `latest-session.md` unconditionally on entry; a deletion in the Work handler is decorative and was removed.

## Plan-tree cleanliness asserted via `git status --porcelain`
`git_commit_triage_leaves_plan_tree_clean_at_user_prompt` and `git_commit_work_leaves_plan_tree_clean_at_user_prompt` assert `git status --porcelain -- <plan_dir>` is empty after `phase_loop` returns from a user-declined exit.

## `run_stack` replaces `phase_loop` as top-level entry point
`main.rs` calls `run_stack` in `phase_loop.rs`. `run_stack` owns the `Frame`/`Stack` push/pop/continue logic across nested plan cycles; the original `phase_loop` is an internal helper called per frame.

## Pivot state machines are purely functional
`decide_after_work` and `decide_after_cycle` in `pivot.rs` take current frame state and return the next action — no I/O, no async, no side effects. The four-case matrix is fully testable in `tests/integration.rs` without a real agent.

## `spawn_blocking` does not cancel cleanly in `tokio::select!`
Use `tokio::time::sleep` for tty event polling. A `spawn_blocking` thread is not dropped when the select arm is cancelled; it races the spawned child for the tty. `tokio::time::sleep` is properly cancellable and eliminates the race.

## Dream-baseline seeded in `GitCommitReflect` handler
`seed_dream_baseline_if_missing` in `src/dream.rs` is called from the `GitCommitReflect` handler in `phase_loop.rs`. Written only when absent; no-ops on subsequent cycles.

## Claude Code TUI requires `--debug-file` workaround for ≤2.1.116
`invoke_interactive` in `src/agent/claude_code.rs` passes `--debug-file /tmp/claude-debug.log`; debug mode masks a TUI rendering failure via an unknown upstream mechanism. Root cause not found. Remove both `args.push` lines when claude is updated past 2.1.116.

## Phase prompts invoke `ravel-lite state set-phase`
All 5 `defaults/phases/*.md` prompts use `ravel-lite state set-phase <plan-dir> <phase>` to transition phase. Direct writes to `phase.md` bypass `Phase::parse` validation; LLM phases must use the CLI. `run_set_phase` also refuses to create a plan dir that does not already exist.

## `push_timestamp()` in `pivot.rs` is canonical
Single source-of-truth for the `pushed_at` timestamp format. Phase-loop and the state CLI both call `pivot::push_timestamp()`. Any third call site must use that function rather than inlining the format string.

## CLI integration tests use `CARGO_BIN_EXE_ravel-lite`
`tests/integration.rs` shells out to the binary via the `CARGO_BIN_EXE_ravel-lite` env var and asserts on-disk effects. This is the Cargo-idiomatic pattern for end-to-end CLI testing without `std::process::Command` hardcoding.

## `term_title.rs` sets phase title via OSC escape
`src/term_title.rs` exposes `set_title(project, plan, phase)` (side-effecting) and `format_title_escape` (testable helper). Writes to stdout; clean side-channel because Ratatui uses stderr backend. Includes tmux passthrough (doubled inner ESCs). Called at `LlmPhase` entry in `phase_loop` and `run_stack`, in `do_push` after `sync_stack_to_disk`, and at both pop sites in `run_stack`. Phase labels uppercased to match the phase-header banner convention.
