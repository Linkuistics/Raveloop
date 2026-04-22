//! Typed session-log.yaml + latest-session.yaml surface and CLI verbs.

pub mod parse_md;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use parse_md::{parse_latest_session_markdown, parse_session_log_markdown};
pub use schema::{SessionLogFile, SessionRecord};
pub use verbs::{
    append_latest_to_log, build_record_for_append, run_append, run_list, run_set_latest,
    run_show, run_show_latest, OutputFormat,
};
pub use yaml_io::{
    read_latest_session, read_session_log, write_latest_session, write_session_log,
};
