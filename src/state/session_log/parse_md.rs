//! Strict parser for the legacy `<plan>/session-log.md` and
//! `<plan>/latest-session.md` prose formats.
//!
//! Used exclusively by the `state migrate` verb. Accepts the canonical
//! shape analyse-work prompts have emitted:
//!
//! ```text
//! ### Session <N> (<timestamp>) — <title>
//!
//! <body paragraphs>
//! ```
//!
//! Any preamble before the first `### Session` heading (typically the
//! `# Session Log` title) is discarded.
//!
//! Id is derived as `<YYYY-MM-DD>-<title-slug>`, falling back to the
//! raw title slug when the timestamp can't be date-extracted. The
//! `phase` field is populated with `"work"` — the only phase that
//! writes session records in the current loop — when migrating from
//! the untyped prose format (the field is required in the new schema).
//! Post-migration, set-latest callers pass the phase explicitly.

use anyhow::{anyhow, bail, Result};

use crate::state::backlog::schema::{allocate_id, slug_from_title};

use super::schema::{SessionLogFile, SessionRecord};

/// Default phase for records parsed from the legacy prose format, which
/// does not record the phase. Work-cycle sessions are the only ones
/// produced by the current loop, so this is the only plausible value.
const DEFAULT_MIGRATED_PHASE: &str = "work";

pub fn parse_session_log_markdown(input: &str) -> Result<SessionLogFile> {
    let mut sessions = Vec::new();
    let mut existing_ids: Vec<String> = Vec::new();

    for (block_index, block) in split_into_session_blocks(input).into_iter().enumerate() {
        let record = parse_single_session_block(&block, &existing_ids).map_err(|err| {
            anyhow!("failed to parse session block #{}: {err:#}", block_index + 1)
        })?;
        existing_ids.push(record.id.clone());
        sessions.push(record);
    }

    Ok(SessionLogFile {
        sessions,
        extra: Default::default(),
    })
}

/// Parse a single-record latest-session.md file. Returns the first (and
/// only expected) session record. An input with multiple `### Session`
/// headings is an error — latest-session.md always holds exactly one.
pub fn parse_latest_session_markdown(input: &str) -> Result<SessionRecord> {
    let blocks = split_into_session_blocks(input);
    match blocks.len() {
        0 => bail!("latest-session.md has no `### Session` heading"),
        1 => parse_single_session_block(&blocks[0], &[]),
        n => bail!(
            "latest-session.md has {n} `### Session` headings; expected exactly one"
        ),
    }
}

fn split_into_session_blocks(input: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in input.lines() {
        if line.starts_with("### Session") {
            if let Some(buffer) = current.take() {
                if !buffer.trim().is_empty() {
                    blocks.push(buffer);
                }
            }
            current = Some(String::new());
        }
        if let Some(buf) = current.as_mut() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(buffer) = current {
        if !buffer.trim().is_empty() {
            blocks.push(buffer);
        }
    }
    blocks
}

fn parse_single_session_block(block: &str, existing_ids: &[String]) -> Result<SessionRecord> {
    let mut lines = block.lines();
    let heading_line = lines
        .next()
        .ok_or_else(|| anyhow!("empty session block"))?;
    let (timestamp, title) = parse_heading(heading_line)?;

    let body_lines: Vec<&str> = lines.collect();
    let mut start = 0;
    while start < body_lines.len() && body_lines[start].trim().is_empty() {
        start += 1;
    }
    let mut end = body_lines.len();
    while end > start && body_lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    if start == end {
        bail!("session {title:?} has no body");
    }
    let body = body_lines[start..end].join("\n") + "\n";

    let id_base = id_base_from(&timestamp, &title);
    let id = allocate_id(&id_base, existing_ids.iter().map(String::as_str));

    // The typed schema has no `title:` field — prepend the title as a
    // bold first line so migrated records remain self-describing when
    // read back from YAML. Prose inputs already containing that heading
    // at the top are left alone (idempotent).
    let heading = format!("**{title}**\n\n");
    let body_with_title = if body.starts_with(&heading) {
        body
    } else {
        format!("{heading}{body}")
    };

    Ok(SessionRecord {
        id,
        timestamp,
        phase: DEFAULT_MIGRATED_PHASE.to_string(),
        body: body_with_title,
    })
}

/// Extract `(timestamp, title)` from an `### Session <N> (<timestamp>) — <title>`
/// heading line. Accepts both em-dash `—` and ASCII `-` separators; the live
/// files consistently use em-dash but a migration shouldn't fail on a
/// hand-edit.
fn parse_heading(line: &str) -> Result<(String, String)> {
    let rest = line
        .strip_prefix("### Session")
        .ok_or_else(|| anyhow!("session heading does not start with `### Session`: {line:?}"))?
        .trim_start();
    // Skip the session number (digits up to the `(`).
    let paren_open = rest
        .find('(')
        .ok_or_else(|| anyhow!("session heading missing `(timestamp)`: {line:?}"))?;
    let paren_close = rest[paren_open..]
        .find(')')
        .ok_or_else(|| anyhow!("session heading missing closing `)`: {line:?}"))?
        + paren_open;
    let timestamp = rest[paren_open + 1..paren_close].trim().to_string();
    let after_paren = rest[paren_close + 1..].trim_start();

    // Separator is either em-dash or ASCII hyphen.
    let title = if let Some(stripped) = after_paren.strip_prefix("—") {
        stripped.trim().to_string()
    } else if let Some(stripped) = after_paren.strip_prefix('-') {
        stripped.trim().to_string()
    } else if after_paren.is_empty() {
        String::new()
    } else {
        after_paren.trim().to_string()
    };

    if timestamp.is_empty() {
        bail!("session heading has empty timestamp: {line:?}");
    }
    if title.is_empty() {
        bail!("session heading has empty title: {line:?}");
    }
    Ok((timestamp, title))
}

fn id_base_from(timestamp: &str, title: &str) -> String {
    // Use the date prefix (first 10 chars `YYYY-MM-DD`) when the
    // timestamp looks ISO-shaped; otherwise fall back to the title slug
    // alone. Matches the design-doc example id shape
    // `2026-04-22-run-plan-work-core`.
    let date_prefix: String = timestamp.chars().take(10).collect();
    let looks_like_date = date_prefix.len() == 10
        && date_prefix.chars().enumerate().all(|(idx, ch)| match idx {
            4 | 7 => ch == '-',
            _ => ch.is_ascii_digit(),
        });
    let title_slug = slug_from_title(title);
    if looks_like_date {
        format!("{date_prefix} {title_slug}")
    } else {
        title_slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_SESSION_MARKDOWN: &str = "\
# Session Log

### Session 1 (2026-04-21T08:03:01Z) — First session title

- Bullet one.
- Bullet two.

### Session 2 (2026-04-21T09:52:55Z) — Second session title

Paragraph body.

Another paragraph.
";

    #[test]
    fn parses_two_sessions_skipping_top_level_header() {
        let log = parse_session_log_markdown(TWO_SESSION_MARKDOWN).unwrap();
        assert_eq!(log.sessions.len(), 2);

        let first = &log.sessions[0];
        assert_eq!(first.id, "2026-04-21-first-session-title");
        assert_eq!(first.timestamp, "2026-04-21T08:03:01Z");
        assert_eq!(first.phase, "work");
        assert!(first.body.contains("Bullet one."));
        assert!(first.body.contains("First session title"));

        let second = &log.sessions[1];
        assert_eq!(second.id, "2026-04-21-second-session-title");
        assert!(second.body.contains("Another paragraph."));
    }

    #[test]
    fn id_suffixes_on_collision() {
        let input = "\
### Session 1 (2026-04-21T00:00:00Z) — Same title

Body one.

### Session 2 (2026-04-21T00:01:00Z) — Same title

Body two.
";
        let log = parse_session_log_markdown(input).unwrap();
        assert_eq!(log.sessions[0].id, "2026-04-21-same-title");
        assert_eq!(log.sessions[1].id, "2026-04-21-same-title-2");
    }

    #[test]
    fn rejects_session_with_empty_body() {
        let input = "\
### Session 1 (2026-04-21T00:00:00Z) — Empty body

### Session 2 (2026-04-21T00:01:00Z) — Has body

real body
";
        let err = parse_session_log_markdown(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no body"), "error must mention empty body: {msg}");
    }

    #[test]
    fn empty_input_parses_to_zero_sessions() {
        let log = parse_session_log_markdown("").unwrap();
        assert!(log.sessions.is_empty());
    }

    #[test]
    fn preamble_only_parses_to_zero_sessions() {
        let log = parse_session_log_markdown("# Session Log\n\n(no sessions yet)\n").unwrap();
        assert!(log.sessions.is_empty());
    }

    #[test]
    fn parse_latest_session_returns_single_record() {
        let input = "\
### Session 10 (2026-04-22T06:14:36Z) — Implement R2

- Built the thing.
- Shipped tests.
";
        let record = parse_latest_session_markdown(input).unwrap();
        assert_eq!(record.id, "2026-04-22-implement-r2");
        assert_eq!(record.timestamp, "2026-04-22T06:14:36Z");
        assert_eq!(record.phase, "work");
        assert!(record.body.contains("Built the thing."));
    }

    #[test]
    fn parse_latest_session_rejects_multiple_headings() {
        let input = "\
### Session 1 (2026-04-21T00:00:00Z) — One

body one.

### Session 2 (2026-04-22T00:00:00Z) — Two

body two.
";
        let err = parse_latest_session_markdown(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("exactly one"), "error must state the count rule: {msg}");
    }

    #[test]
    fn parse_latest_session_rejects_empty_input() {
        let err = parse_latest_session_markdown("").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no `### Session` heading"), "error must explain: {msg}");
    }

    #[test]
    fn parses_live_core_session_log_without_error() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("LLM_STATE/core/session-log.md");
        if !path.exists() {
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let log =
            parse_session_log_markdown(&text).expect("core session-log must parse");
        assert!(
            log.sessions.len() >= 5,
            "core session-log should have many sessions, got {}",
            log.sessions.len()
        );
    }

    #[test]
    fn parses_live_core_latest_session_without_error() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("LLM_STATE/core/latest-session.md");
        if !path.exists() {
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let record = parse_latest_session_markdown(&text)
            .expect("core latest-session must parse");
        assert!(!record.id.is_empty(), "record id must be populated");
    }
}
