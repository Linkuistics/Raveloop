### Session 1 (2026-04-17T13:36:43Z) — resolve 6 backlog tasks: UI fixes, init wiring, commit guardrail

- Worked through 6 tasks from the backlog in a single session, all completed successfully.

- **create: auto-create missing parent dirs** (`src/create.rs`): `validate_target` now calls
  `fs::create_dir_all(parent)` instead of hard-erroring when the parent doesn't exist. Preserved the
  "parent is a file" error case. Inverted old test, added new coverage; all 7 create-module tests pass.

- **ui: transcript truncation at phase boundary** (`src/ui.rs`): Root cause was ratatui's `Terminal::clear()`
  using a stale `viewport_area` after the child's output had scrolled the screen. Fix: on `Resume`, write a
  newline to stderr to fence the child's output, flush, re-enable raw mode, and reconstruct the `Terminal`
  so the viewport starts on a clean row. Eliminates the stale `clear()` path entirely.

- **phase_loop: project name in phase header banner** (`src/phase_loop.rs`, `docs/architecture.md`):
  Added `project_name` (basename extractor) and `header_scope` (formats `project / plan`) helpers.
  Threaded `project_dir` through `phase_loop` and `handle_script_phase`; updated all banner call sites.
  Added 5 unit tests; banner now renders as `◆  REFLECT  ·  raveloop / core`. Total: 119 unit tests pass.

- **docs: sync architecture.md Message Model** (`docs/architecture.md`): Corrected `UIMessage` variant
  field shapes (`StyledLine`-based), removed phantom `RegisterAgent.header` field, added `Quit` variant,
  added Ordering Invariants subsection. Also fixed TUI Layout section (1-row viewport, real `AppState`
  struct, `Terminal::insert_before` scroll mechanism). Doc-only; no code changes.

- **init: embed 5 new coding-style files** (`src/init.rs`): Added `EmbeddedFile` entries for swift,
  typescript, python, bash, and elixir coding-style files. Added drift-detection unit test that reads
  `defaults/fixed-memory/` at test time and asserts every `coding-style-*.md` on disk is registered.

- **phase_loop/git: guard against meta-only work commits** (`src/phase_loop.rs`, `src/git.rs`,
  `defaults/phases/work.md`): Added `git::working_tree_status` helper and `warn_if_project_tree_dirty`
  postcondition that fires after `GitCommitWork` and logs a `⚠  WARNING` block to the TUI if the
  project tree is dirty. Removed the false "auto-commits all project changes" claim from work.md and
  added an explicit step 8 requiring the agent to stage + commit its own source changes and verify with
  `git status` before writing `analyse-work` to phase.md.

- All work was verified: 119 unit tests + 5 integration tests pass after each task.
- What this suggests next: tackle one of the `not_started` bugs — good candidates are
  `Propagate filesystem errors from write_phase` (small, safe) or the pi agent scope decision.
