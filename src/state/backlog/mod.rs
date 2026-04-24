//! Typed backlog.yaml surface and CRUD CLI verbs.

pub mod parse_md;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use parse_md::parse_backlog_markdown;
pub use schema::{BacklogFile, Status, TaskCounts};
pub use verbs::{
    run_add, run_clear_handoff, run_delete, run_init, run_list, run_reorder,
    run_set_dependencies, run_set_description, run_set_handoff, run_set_results,
    run_set_status, run_set_title, run_show, AddRequest, ListFilter, OutputFormat,
    ReorderPosition,
};
pub use yaml_io::{read_backlog, write_backlog};
