# Memory

## `survey.rs` is the primary refactor candidate
At 1287 LOC it is the largest file in the codebase.

## `prompt.rs` has no post-substitution token validation
Leftover `{{...}}` tokens after substitution are silently accepted. This is how the pi `{{MEMORY_DIR}}` bug was introduced without error.

## `phase_loop.rs` silently drops `write_phase` errors
Uses `let _ = ...` pattern; phase-file write failures go unobserved.

## Pi agent has unresolved `{{MEMORY_DIR}}` token
Template substitution for pi's memory path is broken; the literal token appears in output.

## Pi scope meta-task blocks all pi-specific bug work
A meta decision task must be resolved before investing in pi bugs (stderr capture, integration tests, model update).

## `init.rs` drift-detection test guards coding-style registration
The test reads `defaults/fixed-memory/coding-style-*.md` at test time and asserts every file on disk is registered as an `EmbeddedFile`. Adding a new coding-style file without registering it fails the test.

## `warn_if_project_tree_dirty` fires after `GitCommitWork`
`git::working_tree_status` checks the project tree post-commit; a dirty tree logs a `⚠  WARNING` to the TUI. Guards against sessions that commit only meta files and leave source changes unstaged.
