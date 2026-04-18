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

## `STDERR_BUFFER_CAP` and `warning_line` duplicated across pi.rs and claude_code.rs
Both constants/helpers carry comments pointing to the unification extraction task. Extraction has not yet occurred.
