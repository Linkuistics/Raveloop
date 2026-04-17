// src/format.rs
use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::LlmPhase;

/// Formatted output from tool call or result parsing.
pub struct FormattedOutput {
    pub text: String,
    pub persist: bool,
}

/// A tool call to format for display.
pub struct ToolCall {
    pub name: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub edit_context: Option<String>,
}

struct HighlightRule {
    pattern: Regex,
    label: &'static str,
}

struct ActionStyle {
    colour: &'static str,
}

const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

static PHASE_HIGHLIGHTS: Lazy<HashMap<LlmPhase, Vec<HighlightRule>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert(LlmPhase::AnalyseWork, vec![
        HighlightRule { pattern: Regex::new(r"latest-session\.md$").unwrap(), label: "Writing session log" },
        HighlightRule { pattern: Regex::new(r"commit-message\.md$").unwrap(), label: "Writing commit message" },
    ]);
    m.insert(LlmPhase::Reflect, vec![
        HighlightRule { pattern: Regex::new(r"memory\.md$").unwrap(), label: "Updating memory" },
    ]);
    m.insert(LlmPhase::Dream, vec![
        HighlightRule { pattern: Regex::new(r"memory\.md$").unwrap(), label: "Rewriting memory" },
    ]);
    m.insert(LlmPhase::Triage, vec![
        HighlightRule { pattern: Regex::new(r"backlog\.md$").unwrap(), label: "Updating backlog" },
        HighlightRule { pattern: Regex::new(r"subagent-dispatch\.yaml$").unwrap(), label: "Dispatching to related plans" },
    ]);
    m
});

static ACTION_STYLES: Lazy<HashMap<&'static str, ActionStyle>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("ADDED", ActionStyle { colour: GREEN });
    m.insert("SHARPENED", ActionStyle { colour: CYAN });
    m.insert("REPLACED", ActionStyle { colour: YELLOW });
    m.insert("REMOVED", ActionStyle { colour: RED });
    m.insert("MERGED", ActionStyle { colour: CYAN });
    m.insert("TIGHTENED", ActionStyle { colour: CYAN });
    m.insert("REWORDED", ActionStyle { colour: DIM });
    m.insert("STATS", ActionStyle { colour: DIM });
    m.insert("DELETED", ActionStyle { colour: RED });
    m.insert("PROMOTED", ActionStyle { colour: YELLOW });
    m.insert("REPRIORITISED", ActionStyle { colour: YELLOW });
    m.insert("DISPATCH", ActionStyle { colour: GREEN });
    m.insert("NO DISPATCH", ActionStyle { colour: DIM });
    m
});

static LABEL_WIDTH: Lazy<usize> = Lazy::new(|| {
    ACTION_STYLES.keys().map(|k| k.len()).max().unwrap_or(0)
});

pub struct PhaseInfo {
    pub label: &'static str,
    pub description: &'static str,
}

pub fn phase_info(phase: LlmPhase) -> PhaseInfo {
    match phase {
        LlmPhase::Work => PhaseInfo { label: "WORK", description: "Pick a task, implement it, record results" },
        LlmPhase::AnalyseWork => PhaseInfo { label: "ANALYSE", description: "Examine git diff, write session log and commit message" },
        LlmPhase::Reflect => PhaseInfo { label: "REFLECT", description: "Distil session learnings into durable memory" },
        LlmPhase::Dream => PhaseInfo { label: "DREAM", description: "Rewrite memory losslessly in tighter form" },
        LlmPhase::Triage => PhaseInfo { label: "TRIAGE", description: "Reprioritise backlog, propagate to related plans" },
    }
}

/// Format a tool call for display. Returns persist=true for highlight matches.
/// `shown_highlights` tracks which labels have already been emitted this phase.
pub fn format_tool_call(
    tool: &ToolCall,
    phase: Option<LlmPhase>,
    shown_highlights: &mut std::collections::HashSet<String>,
) -> FormattedOutput {
    let is_write = matches!(
        tool.name.to_lowercase().as_str(),
        "write" | "edit"
    );
    let is_bash_write = tool.name.to_lowercase() == "bash"
        && tool.detail.as_deref().map_or(false, |d| {
            d.contains("cat ") && d.contains("> ") || d.contains("echo ") && d.contains("> ")
        });

    if is_write || is_bash_write {
        if let Some(phase) = phase {
            let path_to_check = tool.path.as_deref()
                .or(tool.detail.as_deref())
                .unwrap_or("");

            if let Some(rules) = PHASE_HIGHLIGHTS.get(&phase) {
                for rule in rules {
                    if rule.pattern.is_match(path_to_check) {
                        if shown_highlights.contains(rule.label) {
                            return FormattedOutput { text: String::new(), persist: false };
                        }
                        shown_highlights.insert(rule.label.to_string());
                        return FormattedOutput {
                            text: format!("  {BOLD}{GREEN}★  {}{RESET}", rule.label),
                            persist: true,
                        };
                    }
                }
            }

            // Silently skip phase.md writes
            if path_to_check.contains("phase.md") {
                return FormattedOutput { text: String::new(), persist: false };
            }
        }
    }

    // Strip newlines from detail
    let desc = tool.detail.as_deref()
        .or(tool.path.as_deref())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");

    FormattedOutput {
        text: format!("{DIM}  ·  {} {desc}{RESET}", tool.name),
        persist: false,
    }
}

/// Format result text from a headless phase.
/// Recognises [ACTION] markers and Insight blocks.
pub fn format_result_text(text: &str) -> String {
    static ACTION_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[\s\-\*]*\[([A-Za-z ]+)\]\s*(.*)$").unwrap()
    });
    static PHASE_MD_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(?:^(?:`?phase\.md`?|Phase)\s+(?:set to|written|→))|(?:phase\.md.*`git-commit-)|(?:wrote.*phase\.md)").unwrap()
    });

    let mut formatted = vec![String::new()]; // blank line separates progress from result
    let mut in_insight = false;

    for line in text.lines() {
        // Filter phase.md status lines
        if PHASE_MD_RE.is_match(line) { continue; }
        // Filter code fence lines
        if line.trim() == "```" { continue; }

        // Structured action markers
        if let Some(caps) = ACTION_RE.captures(line) {
            let tag = caps[1].to_uppercase();
            let detail = &caps[2];
            if let Some(style) = ACTION_STYLES.get(tag.as_str()) {
                let padded = format!("{:<width$}", tag, width = *LABEL_WIDTH);
                formatted.push(format!(
                    "  {}{BOLD}{padded}{RESET}  {}{detail}{RESET}",
                    style.colour, style.colour
                ));
                continue;
            }
        }

        // Insight block opening
        if line.contains("★") && line.contains("Insight") && line.contains("─") {
            in_insight = true;
            formatted.push(format!("  {BOLD}{CYAN}★ Insight{RESET}"));
            continue;
        }
        // Insight block closing
        if in_insight && line.chars().filter(|c| *c == '─').count() >= 10 {
            in_insight = false;
            continue;
        }
        // Insight content or regular text — indent
        formatted.push(format!("  {DIM}{line}{RESET}"));
    }

    // Trim trailing blank lines
    while formatted.len() > 1 && formatted.last().map_or(false, |l| l.trim().is_empty()) {
        formatted.pop();
    }

    formatted.join("\n")
}

/// Extract a brief context string from Edit old/new strings.
pub fn extract_edit_context(old: Option<&str>, new: Option<&str>) -> Option<String> {
    let source = new.or(old)?;
    // Look for markdown headings
    static HEADING_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#{1,4}\s+(.{1,60})").unwrap());
    if let Some(caps) = HEADING_RE.captures(source) {
        return Some(caps[1].trim().to_string());
    }
    // Look for bold text
    static BOLD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*(.{1,60}?)\*\*").unwrap());
    if let Some(caps) = BOLD_RE.captures(source) {
        return Some(caps[1].trim().to_string());
    }
    None
}

/// Clean up a tool name — strip MCP prefixes.
pub fn clean_tool_name(name: &str) -> String {
    static MCP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^mcp__[^_]+(?:__)?(.+)$").unwrap());
    if let Some(caps) = MCP_RE.captures(name) {
        return caps[1].to_string();
    }
    name.to_string()
}

/// Extract a meaningful detail string from tool input parameters.
pub fn extract_tool_detail(input: &serde_json::Value) -> String {
    const DETAIL_KEYS: &[&str] = &["path", "file_path", "command", "pattern", "query", "url", "prompt", "mode"];
    if let Some(obj) = input.as_object() {
        for key in DETAIL_KEYS {
            if let Some(serde_json::Value::String(val)) = obj.get(*key) {
                if !val.is_empty() {
                    return if val.len() > 80 { format!("{}...", &val[..77]) } else { val.clone() };
                }
            }
        }
        // Fallback: first string value
        for val in obj.values() {
            if let serde_json::Value::String(s) = val {
                if !s.is_empty() {
                    return if s.len() > 60 { format!("{}...", &s[..57]) } else { s.clone() };
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn format_tool_call_highlight_memory() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.persist);
        assert!(result.text.contains("Updating memory"));
    }

    #[test]
    fn format_tool_call_deduplicates_highlights() {
        let mut shown = HashSet::new();
        let r1 = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(r1.persist);

        let r2 = format_tool_call(
            &ToolCall { name: "Edit".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(!r2.persist);
        assert!(r2.text.is_empty());
    }

    #[test]
    fn format_tool_call_skips_phase_md() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/phase.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.text.is_empty());
    }

    #[test]
    fn format_result_text_recognises_action_markers() {
        let text = "[ADDED] New entry — description\n[REMOVED] Old entry — stale";
        let result = format_result_text(text);
        assert!(result.contains("ADDED"));
        assert!(result.contains("REMOVED"));
        assert!(result.contains("description"));
    }

    #[test]
    fn format_result_text_filters_phase_md() {
        let text = "phase.md set to git-commit-reflect\n[ADDED] Real content";
        let result = format_result_text(text);
        assert!(!result.contains("phase.md set to"));
        assert!(result.contains("Real content"));
    }

    #[test]
    fn clean_tool_name_strips_mcp_prefix() {
        assert_eq!(clean_tool_name("mcp__server__tool_name"), "tool_name");
        assert_eq!(clean_tool_name("Read"), "Read");
    }

    #[test]
    fn extract_edit_context_finds_headings() {
        assert_eq!(
            extract_edit_context(None, Some("## My Heading\nsome content")),
            Some("My Heading".to_string())
        );
    }

    #[test]
    fn extract_edit_context_finds_bold() {
        assert_eq!(
            extract_edit_context(None, Some("update **Important Thing** here")),
            Some("Important Thing".to_string())
        );
    }
}
