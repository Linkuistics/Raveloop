//! Strict parser for the legacy `<plan>/backlog.md` prose format.
//!
//! Used exclusively by the `state migrate` verb. Accepts the canonical
//! shape phase prompts emit today (`### Title` headings, `**Field:**
//! value` lines, `---` separators between tasks) and refuses anything
//! else rather than risk silent data loss.

use anyhow::{anyhow, bail, Context, Result};

use super::schema::{slug_from_title, BacklogFile, Status, Task};

pub fn parse_backlog_markdown(input: &str) -> Result<BacklogFile> {
    let mut tasks = Vec::new();
    let mut existing_ids: Vec<String> = Vec::new();

    let blocks = split_into_task_blocks(input);
    for (block_index, block) in blocks.into_iter().enumerate() {
        let task = parse_single_task_block(&block, &existing_ids)
            .with_context(|| format!("failed to parse task block #{}", block_index + 1))?;
        existing_ids.push(task.id.clone());
        tasks.push(task);
    }

    Ok(BacklogFile {
        tasks,
        extra: Default::default(),
    })
}

fn split_into_task_blocks(input: &str) -> Vec<String> {
    // Split on `### ` heading boundaries rather than on `---` separators.
    // A `---` inside a task block is ambiguous: it may be the trailing
    // task separator, or the start of a `\n---\n[HANDOFF]` marker.
    // Heading-boundary splitting sidesteps the ambiguity — the handoff
    // body stays inside its owning task block. `normalise_block` then
    // strips the optional trailing separator line.
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in input.lines() {
        if line.starts_with("### ") {
            if let Some(buffer) = current.take() {
                let normalised = normalise_block(&buffer);
                if !normalised.trim().is_empty() {
                    blocks.push(normalised);
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
        let normalised = normalise_block(&buffer);
        if !normalised.trim().is_empty() {
            blocks.push(normalised);
        }
    }
    blocks
}

/// Strip the optional trailing task-separator `---` line from a block.
/// The separator is a structural marker between tasks, not part of the
/// last task's content. A `---\n[HANDOFF]` marker in the middle of the
/// block is left untouched — only a truly-trailing `---` is removed.
fn normalise_block(block: &str) -> String {
    let mut lines: Vec<&str> = block.lines().collect();
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if lines.last().map(|l| l.trim() == "---").unwrap_or(false) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn parse_single_task_block(block: &str, existing_ids: &[String]) -> Result<Task> {
    let mut lines = block.lines();
    let title_line = lines
        .next()
        .ok_or_else(|| anyhow!("empty task block"))?;
    let title = title_line
        .strip_prefix("### ")
        .ok_or_else(|| anyhow!("task block does not start with `### <title>`: {title_line:?}"))?
        .trim()
        .to_string();
    if title.is_empty() {
        bail!("task title is empty");
    }

    let id = allocate_id_from(&title, existing_ids);

    let mut category: Option<String> = None;
    let mut status: Option<Status> = None;
    let mut dependencies: Vec<String> = Vec::new();
    let mut results: Option<String> = None;
    let mut handoff: Option<String> = None;
    let mut blocked_reason: Option<String> = None;

    let rest: Vec<&str> = lines.collect();
    let mut cursor = 0;

    while cursor < rest.len() {
        let line = rest[cursor];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            cursor += 1;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("**Category:**") {
            category = Some(strip_backticks(value.trim()).to_string());
            cursor += 1;
        } else if let Some(value) = trimmed.strip_prefix("**Status:**") {
            let raw = strip_backticks(value.trim());
            let (status_value, reason) = split_blocked_reason(raw);
            status = Some(
                Status::parse(status_value)
                    .ok_or_else(|| anyhow!("invalid status value: {status_value:?}"))?,
            );
            if status == Some(Status::Blocked) {
                blocked_reason = Some(reason.unwrap_or_default().to_string());
            }
            cursor += 1;
        } else if let Some(value) = trimmed.strip_prefix("**Dependencies:**") {
            let raw = value.trim();
            if raw != "none" && !raw.is_empty() {
                dependencies = raw
                    .split(',')
                    .map(|s| slug_from_title(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            cursor += 1;
        } else {
            break;
        }
    }

    let body: Vec<&str> = rest[cursor..].to_vec();
    let (desc_block, rest_after_desc) =
        take_section(&body, "**Description:**", &["**Results:**", "[HANDOFF]"]);
    let description = desc_block.trim_end_matches('\n').to_string() + "\n";

    let (results_block, rest_after_results) =
        take_section(&rest_after_desc, "**Results:**", &["[HANDOFF]"]);
    if !results_block.trim().is_empty() {
        let body = results_block.trim_end_matches('\n');
        if body == "_pending_" {
            // Convention: `_pending_` means "no results yet"; leave as None.
        } else {
            results = Some(body.to_string() + "\n");
        }
    }

    if !rest_after_results.is_empty() {
        let joined = rest_after_results.join("\n");
        if let Some(idx) = joined.find("[HANDOFF]") {
            let tail = joined[idx..].trim_start_matches("[HANDOFF]").trim_start();
            handoff = Some(tail.trim_end().to_string() + "\n");
        }
    }

    Ok(Task {
        id,
        title,
        category: category.ok_or_else(|| anyhow!("missing **Category:** field"))?,
        status: status.ok_or_else(|| anyhow!("missing **Status:** field"))?,
        blocked_reason,
        dependencies,
        description,
        results,
        handoff,
    })
}

fn allocate_id_from(title: &str, existing: &[String]) -> String {
    super::schema::allocate_id(title, existing.iter().map(String::as_str))
}

fn strip_backticks(value: &str) -> &str {
    value.trim().trim_matches('`')
}

fn split_blocked_reason(raw: &str) -> (&str, Option<&str>) {
    // `**Status:** blocked (reason: upstream release)`
    if let Some((status, rest)) = raw.split_once('(') {
        let reason = rest.trim_end_matches(')').trim();
        let reason = reason.strip_prefix("reason:").unwrap_or(reason).trim();
        (status.trim(), Some(reason))
    } else {
        (raw.trim(), None)
    }
}

/// Return `(section_body, remaining_lines)` where `section_body` is
/// everything under a `starts_with` heading up to (but not including)
/// any of `terminators`, joined by newlines. If no heading is found,
/// `section_body` is empty and `remaining_lines` is the original input.
fn take_section<'a>(
    lines: &[&'a str],
    starts_with: &str,
    terminators: &[&str],
) -> (String, Vec<&'a str>) {
    let mut iter = lines.iter().enumerate();
    let start_index = loop {
        match iter.next() {
            Some((idx, line)) if line.trim_start().starts_with(starts_with) => break Some(idx),
            Some(_) => continue,
            None => break None,
        }
    };
    let Some(start_index) = start_index else {
        return (String::new(), lines.to_vec());
    };
    let first_content = start_index + 1;
    let mut end_index = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(first_content) {
        if terminators.iter().any(|t| line.trim_start().starts_with(t)) {
            end_index = idx;
            break;
        }
    }
    let body_lines = &lines[first_content..end_index];
    let joined = body_lines.join("\n");
    let remaining = lines[end_index..].to_vec();
    (joined.trim_start_matches('\n').to_string(), remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TASK: &str = "\
### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Cargo clippy is clean today. Add a CI gate.

**Results:** _pending_

---
";

    #[test]
    fn parses_minimal_task() {
        let backlog = parse_backlog_markdown(MINIMAL_TASK).unwrap();
        assert_eq!(backlog.tasks.len(), 1);
        let task = &backlog.tasks[0];
        assert_eq!(task.id, "add-clippy-d-warnings-ci-gate");
        assert_eq!(task.title, "Add clippy `-D warnings` CI gate");
        assert_eq!(task.category, "maintenance");
        assert_eq!(task.status, Status::NotStarted);
        assert!(task.dependencies.is_empty());
        assert_eq!(
            task.description.trim(),
            "Cargo clippy is clean today. Add a CI gate."
        );
        assert_eq!(task.results, None);
        assert_eq!(task.handoff, None);
    }

    #[test]
    fn parses_task_with_results_and_handoff() {
        let input = "\
### Finished task

**Category:** `research`
**Status:** `done`
**Dependencies:** none

**Description:**

Did the thing.

**Results:**

Did the thing successfully.

---
[HANDOFF]

Promote `follow-up-task` to backlog.
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(backlog.tasks.len(), 1);
        let task = &backlog.tasks[0];
        assert_eq!(task.status, Status::Done);
        assert!(task.results.as_deref().unwrap().contains("successfully"));
        assert!(task
            .handoff
            .as_deref()
            .unwrap()
            .contains("Promote `follow-up-task` to backlog."));
    }

    #[test]
    fn parses_dependencies_as_slugged_ids() {
        let input = "\
### Dependent task

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** Research: expose plan-state markdown, Another Upstream

**Description:**

Waits for the two above.

**Results:** _pending_

---
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(
            backlog.tasks[0].dependencies,
            vec![
                "research-expose-plan-state-markdown".to_string(),
                "another-upstream".to_string()
            ]
        );
    }

    #[test]
    fn parses_blocked_status_with_reason() {
        let input = "\
### Blocked task

**Category:** `maintenance`
**Status:** `blocked (reason: upstream pending 2.1.117)`
**Dependencies:** none

**Description:**

Waiting for Claude Code upstream.

**Results:** _pending_

---
";
        let backlog = parse_backlog_markdown(input).unwrap();
        assert_eq!(backlog.tasks[0].status, Status::Blocked);
        assert_eq!(
            backlog.tasks[0].blocked_reason.as_deref(),
            Some("upstream pending 2.1.117")
        );
    }

    #[test]
    fn multiple_tasks_split_on_separator() {
        let input = format!("{MINIMAL_TASK}\n{MINIMAL_TASK}");
        let backlog = parse_backlog_markdown(&input).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
        // The second task's id collides with the first's title; the
        // allocator must suffix it.
        assert_eq!(backlog.tasks[0].id, "add-clippy-d-warnings-ci-gate");
        assert_eq!(backlog.tasks[1].id, "add-clippy-d-warnings-ci-gate-2");
    }

    #[test]
    fn rejects_task_missing_category() {
        let input = "\
### No category task

**Status:** `not_started`
**Dependencies:** none

**Description:**

Body.

**Results:** _pending_

---
";
        let err = parse_backlog_markdown(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Category"), "error must name the missing field: {msg}");
    }

    #[test]
    fn parses_live_core_backlog_without_error() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("LLM_STATE/core/backlog.md");
        if !path.exists() {
            // Allow the test to skip on a fresh checkout without a core plan.
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let backlog = parse_backlog_markdown(&text).expect("core backlog must parse");
        assert!(
            backlog.tasks.len() >= 2,
            "core backlog must have multiple tasks, got {}",
            backlog.tasks.len()
        );
    }
}
