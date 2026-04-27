//! Optional process-wide debug log for ravel ↔ claude interactions.
//!
//! Activated by `ravel-lite run --debug`. While active:
//!   * every claude spawn is invoked with
//!     `--debug-file /tmp/ravel-claude-debug.log` (claude's own internal
//!     trace), and
//!   * every spawn site (argv, prompt, raw stdout/stderr lines, exit
//!     status) is recorded to `/tmp/ravel-embedding-debug.log` so the
//!     user can correlate ravel-side decisions with claude-side trace.
//!
//! Implemented as a `OnceLock<DebugLog>` singleton because debug logging
//! is one process-wide on/off state — threading an `Option<Arc<...>>`
//! through `run_phase_loop`, `run_multi_plan`, both agent constructors,
//! and `run_streaming_child` would add five-plus signatures for what is
//! morally a CLI flag.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Path the `--debug-file` argument points claude at. Hard-coded — the
/// flag is a debug knob, not a configuration surface.
pub const CLAUDE_DEBUG_FILE: &str = "/tmp/ravel-claude-debug.log";

/// Path the ravel-side embedding-debug log is written to.
pub const EMBEDDING_DEBUG_FILE: &str = "/tmp/ravel-embedding-debug.log";

static DEBUG_LOG: OnceLock<DebugLog> = OnceLock::new();

pub struct DebugLog {
    file: Mutex<std::fs::File>,
}

/// Initialise the global debug log at `path`, truncating any previous
/// run's content. Idempotent: a second call is a no-op so re-initialising
/// inside test harnesses or dispatched cycles cannot panic.
pub fn enable(path: impl AsRef<Path>) -> Result<()> {
    if DEBUG_LOG.get().is_some() {
        return Ok(());
    }
    let path: PathBuf = path.as_ref().to_path_buf();
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("failed to open debug log {}", path.display()))?;
    let _ = DEBUG_LOG.set(DebugLog { file: Mutex::new(file) });
    log("debug log opened", &format!("path: {}", path.display()));
    Ok(())
}

pub fn is_enabled() -> bool {
    DEBUG_LOG.get().is_some()
}

/// Append a labelled, timestamped entry. `body` is split on `\n` and
/// each line is indented for readability. No-op when the log is not
/// enabled — every spawn site can call this unconditionally.
pub fn log(label: &str, body: &str) {
    let Some(dl) = DEBUG_LOG.get() else { return };
    let ts = unix_iso_now();
    let mut guard = match dl.file.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let _ = writeln!(*guard, "[{ts}] {label}");
    if !body.is_empty() {
        for line in body.lines() {
            let _ = writeln!(*guard, "    {line}");
        }
    }
    let _ = guard.flush();
}

/// Convenience: log a single raw stream line under a `<agent>:<channel>`
/// label, without the multi-line splitting of `log` (so embedded
/// newlines inside one stream message survive).
pub fn log_stream_line(agent_name: &str, channel: &str, line: &str) {
    let Some(dl) = DEBUG_LOG.get() else { return };
    let ts = unix_iso_now();
    let mut guard = match dl.file.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let _ = writeln!(*guard, "[{ts}] {agent_name}:{channel} {line}");
    let _ = guard.flush();
}

fn unix_iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_utc(secs)
}

/// Convert a Unix-epoch second count into `YYYY-MM-DDTHH:MM:SSZ`.
/// Duplicated from `discover::stage1` rather than extracted: the two
/// modules are otherwise unrelated and lifting a 30-line helper into a
/// shared util crate would obscure rather than simplify.
fn format_unix_utc(mut secs: u64) -> String {
    let seconds = (secs % 60) as u32;
    secs /= 60;
    let minutes = (secs % 60) as u32;
    secs /= 60;
    let hours = (secs % 24) as u32;
    let mut days = secs / 24;

    let mut year: u32 = 1970;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days as u64 {
            break;
        }
        days -= year_days as u64;
        year += 1;
    }

    let month_lens: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u32 = 0;
    while month < 12 && days >= month_lens[month as usize] as u64 {
        days -= month_lens[month as usize] as u64;
        month += 1;
    }
    let day = (days as u32) + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month + 1, day, hours, minutes, seconds
    )
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Format an argv vector as a multi-line block suitable for the body of
/// a `log("... spawn ...", ...)` call. One argv element per line.
pub fn format_argv(program: &str, args: &[String]) -> String {
    let mut s = String::from("argv:\n");
    s.push_str(&format!("    {program}\n"));
    for a in args {
        s.push_str(&format!("    {a}\n"));
    }
    s.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_unix_utc_known_timestamp() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_utc(1_704_067_200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn format_argv_renders_one_arg_per_line() {
        let out = format_argv("claude", &["-p".to_string(), "hello".to_string()]);
        assert_eq!(out, "argv:\n    claude\n    -p\n    hello");
    }

    #[test]
    fn log_when_disabled_is_noop() {
        // The DEBUG_LOG static is process-wide; in the test process we
        // never call enable(), so log() must be a silent no-op rather
        // than panicking or unwrapping.
        log("test", "no debug log enabled");
        log_stream_line("claude", "stdout", "line");
        assert!(!is_enabled());
    }
}
