# Related Plans

## Siblings
Other plans in this project:
- {{DEV_ROOT}}/Ravel-Lite/LLM_STATE/survey-restructure — survey-driven multi-plan run mode and removal of stack.yaml/push-plan. Supersedes core's former task #1 (coordinator plans) and task #5 (original incremental-survey design). See {{DEV_ROOT}}/Ravel-Lite/docs/survey-pivot-design.md for rationale. Be aware that survey-restructure's items touch code in `src/survey/*`, `src/phase_loop.rs`, `src/pivot.rs`, `src/state.rs`, and `src/main.rs` — areas core also owns. Avoid concurrent edits to those paths while survey-restructure is active.
