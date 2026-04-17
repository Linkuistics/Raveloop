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
