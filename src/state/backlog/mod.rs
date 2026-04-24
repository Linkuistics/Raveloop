//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod lint_dependencies;
pub mod parse_md;
pub mod repair_stale_statuses;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use lint_dependencies::{lint_dependencies, run_lint_dependencies, LintReport, TaskDrift};
pub use parse_md::parse_backlog_markdown;
pub use repair_stale_statuses::{
    analyse_repairs, run_repair_stale_statuses, Repair, RepairReason, RepairReport,
};
pub use schema::{BacklogFile, Status, TaskCounts};
pub use verbs::{
    run_add, run_clear_handoff, run_delete, run_init, run_list, run_reorder,
    run_set_dependencies, run_set_description, run_set_handoff, run_set_results,
    run_set_status, run_set_title, run_show, AddRequest, ListFilter, OutputFormat,
    ReorderPosition,
};
pub use yaml_io::{read_backlog, write_backlog};
