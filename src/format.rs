// src/format.rs
use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::LlmPhase;

// ── Semantic styling types ────────────────────────────────────────────────────
// Renderer-agnostic: no ANSI codes, no ratatui. Consumers (TUI or otherwise)
// translate these into their own styling primitives.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Added,    // constructive / positive delta
    Removed,  // destructive
    Changed,  // modification / reprioritisation
    Meta,     // informational / structural
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub intent: Option<Intent>,
    pub dim: bool,
    pub bold: bool,
}

impl Style {
    pub const fn plain() -> Self { Self { intent: None, dim: false, bold: false } }
    pub const fn dim() -> Self { Self { intent: None, dim: true, bold: false } }
    pub const fn intent(i: Intent) -> Self { Self { intent: Some(i), dim: false, bold: false } }
    pub const fn bold_intent(i: Intent) -> Self { Self { intent: Some(i), dim: false, bold: true } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: Style::plain() }
    }
    pub fn dim(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: Style::dim() }
    }
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self { text: text.into(), style }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StyledLine(pub Vec<Span>);

impl StyledLine {
    pub fn empty() -> Self { Self::default() }
    /// Kept as a public constructor even though main code no longer wraps plain
    /// log text in StyledLine (it's inserted directly into scrollback). Tests
    /// use it, and it's a natural symmetry with `empty()`.
    #[allow(dead_code)]
    pub fn plain(text: impl Into<String>) -> Self {
        Self(vec![Span::plain(text)])
    }
    pub fn is_blank(&self) -> bool {
        self.0.iter().all(|s| s.text.is_empty())
    }
}

/// Formatted output from tool call or result parsing.
pub struct FormattedOutput {
    pub lines: Vec<StyledLine>,
    pub persist: bool,
}

impl FormattedOutput {
    pub fn empty() -> Self { Self { lines: Vec::new(), persist: false } }
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_blank())
    }
}

/// A tool call to format for display.
pub struct ToolCall {
    pub name: String,
    pub path: Option<String>,
    pub detail: Option<String>,
}

// ── Highlight / action tables ────────────────────────────────────────────────

struct HighlightRule {
    pattern: Regex,
    label: &'static str,
}

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

/// Intent per action tag. Labels describe the *state* that caused the change,
/// not the action itself (e.g. OBSOLETE, not REMOVED). `None` = neutral/dim.
static ACTION_INTENTS: Lazy<HashMap<&'static str, Option<Intent>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    // memory entry states (reflect + dream)
    m.insert("NEW",           Some(Intent::Added));     // entry didn't exist
    m.insert("IMPRECISE",     Some(Intent::Meta));      // was imprecise → sharpened
    m.insert("STALE",         Some(Intent::Changed));   // was stale → replaced
    m.insert("OBSOLETE",      Some(Intent::Removed));   // no longer relevant → removed
    m.insert("OVERLAPPING",   Some(Intent::Meta));      // overlapped → merged
    m.insert("VERBOSE",       Some(Intent::Meta));      // was wordy → tightened
    m.insert("AWKWARD",       None);                    // phrasing off → reworded
    m.insert("STATS",         None);                    // metric, not a state
    // backlog task states (triage)
    m.insert("DONE",          Some(Intent::Added));     // task completed → closed
    m.insert("BLOCKER",       Some(Intent::Changed));   // was buried blocker → promoted
    m.insert("REPRIORITISED", Some(Intent::Changed));   // priority shifted (delta, kept)
    m.insert("DISPATCH",      Some(Intent::Added));     // handoff to another plan
    m.insert("NO DISPATCH",   None);                    // explicit none
    m
});

static LABEL_WIDTH: Lazy<usize> = Lazy::new(|| {
    ACTION_INTENTS.keys().map(|k| k.len()).max().unwrap_or(0)
});

// ── Phase info ───────────────────────────────────────────────────────────────

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

// ── Formatters ───────────────────────────────────────────────────────────────

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
        && tool.detail.as_deref().is_some_and(|d| {
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
                            return FormattedOutput::empty();
                        }
                        shown_highlights.insert(rule.label.to_string());
                        let line = StyledLine(vec![
                            Span::plain("  "),
                            Span::styled(format!("★  {}", rule.label), Style::bold_intent(Intent::Added)),
                        ]);
                        return FormattedOutput { lines: vec![line], persist: true };
                    }
                }
            }

            // Silently skip phase.md writes
            if path_to_check.contains("phase.md") {
                return FormattedOutput::empty();
            }
        }
    }

    let desc = tool.detail.as_deref()
        .or(tool.path.as_deref())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");

    let line = StyledLine(vec![
        Span::dim(format!("  ·  {} {desc}", tool.name)),
    ]);
    FormattedOutput { lines: vec![line], persist: false }
}

/// Format result text from a headless phase.
/// Recognises [ACTION] markers and Insight blocks.
pub fn format_result_text(text: &str) -> Vec<StyledLine> {
    static ACTION_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[\s\-\*]*\[([A-Za-z ]+)\]\s*(.*)$").unwrap()
    });
    static PHASE_MD_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(?:^(?:`?phase\.md`?|Phase)\s+(?:set to|written|→))|(?:phase\.md.*`git-commit-)|(?:wrote.*phase\.md)").unwrap()
    });

    // Leading blank line separates progress from result.
    let mut out: Vec<StyledLine> = vec![StyledLine::empty()];
    let mut in_insight = false;

    // Push `line`, collapsing runs of blank lines to at most one.
    let push = |out: &mut Vec<StyledLine>, line: StyledLine| {
        if line.is_blank() && out.last().is_some_and(StyledLine::is_blank) {
            return;
        }
        out.push(line);
    };

    for line in text.lines() {
        // Filter phase.md status lines
        if PHASE_MD_RE.is_match(line) { continue; }
        // Filter code fence lines
        if line.trim() == "```" { continue; }

        // Structured action markers
        if let Some(caps) = ACTION_RE.captures(line) {
            let tag = caps[1].to_uppercase();
            let detail = caps[2].to_string();
            if let Some(intent_opt) = ACTION_INTENTS.get(tag.as_str()).copied() {
                let padded = format!("{:<width$}", tag, width = *LABEL_WIDTH);
                let (tag_style, detail_style) = match intent_opt {
                    Some(intent) => (Style::bold_intent(intent), Style::intent(intent)),
                    None => (Style::dim(), Style::dim()),
                };
                push(&mut out, StyledLine(vec![
                    Span::plain("  "),
                    Span::styled(padded, tag_style),
                    Span::plain("  "),
                    Span::styled(detail, detail_style),
                ]));
                continue;
            }
        }

        // Insight block opening
        if line.contains("★") && line.contains("Insight") && line.contains("─") {
            in_insight = true;
            push(&mut out, StyledLine(vec![
                Span::plain("  "),
                Span::styled("★ Insight", Style::bold_intent(Intent::Meta)),
            ]));
            continue;
        }
        // Insight block closing
        if in_insight && line.chars().filter(|c| *c == '─').count() >= 10 {
            in_insight = false;
            continue;
        }
        // Blank input line — emit a real blank, not an indent-only span.
        if line.trim().is_empty() {
            push(&mut out, StyledLine::empty());
            continue;
        }
        // Insight content or regular text — indent, dim
        push(&mut out, StyledLine(vec![
            Span::plain("  "),
            Span::dim(line.to_string()),
        ]));
    }

    // Trim trailing blank lines
    while out.len() > 1 && out.last().is_some_and(StyledLine::is_blank) {
        out.pop();
    }

    out
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

    fn flat_text(line: &StyledLine) -> String {
        line.0.iter().map(|s| s.text.as_str()).collect::<String>()
    }

    fn flat_text_all(lines: &[StyledLine]) -> String {
        lines.iter().map(flat_text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn format_tool_call_highlight_memory() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.persist);
        assert_eq!(result.lines.len(), 1);
        assert!(flat_text(&result.lines[0]).contains("Updating memory"));
        // Emphasised span must carry BoldIntent(Added), not ANSI bytes.
        let emph = result.lines[0].0.iter()
            .find(|s| s.text.contains("Updating memory"))
            .expect("emph span");
        assert_eq!(emph.style, Style::bold_intent(Intent::Added));
    }

    #[test]
    fn format_tool_call_deduplicates_highlights() {
        let mut shown = HashSet::new();
        let r1 = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(r1.persist);

        let r2 = format_tool_call(
            &ToolCall { name: "Edit".to_string(), path: Some("/plan/memory.md".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(!r2.persist);
        assert!(r2.is_empty());
    }

    #[test]
    fn format_tool_call_skips_phase_md() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/phase.md".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn format_tool_call_regular_is_single_dim_span() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Read".to_string(), path: Some("/foo.md".to_string()), detail: None },
            Some(LlmPhase::Work),
            &mut shown,
        );
        assert!(!result.persist);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].0.len(), 1);
        assert_eq!(result.lines[0].0[0].style, Style::dim());
        assert!(result.lines[0].0[0].text.contains("Read /foo.md"));
        // No ANSI bytes.
        assert!(!result.lines[0].0[0].text.contains('\x1b'));
    }

    #[test]
    fn format_result_text_recognises_action_markers() {
        let lines = format_result_text("[NEW] Fresh entry — description\n[OBSOLETE] Old entry — no longer relevant");
        let text = flat_text_all(&lines);
        assert!(text.contains("NEW"));
        assert!(text.contains("OBSOLETE"));
        assert!(text.contains("description"));

        let new_row = lines.iter().find(|l| flat_text(l).contains("NEW")).unwrap();
        let new_tag = new_row.0.iter().find(|s| s.text.trim() == "NEW").unwrap();
        assert_eq!(new_tag.style, Style::bold_intent(Intent::Added));

        let obsolete = lines.iter().find(|l| flat_text(l).contains("OBSOLETE")).unwrap();
        let obsolete_tag = obsolete.0.iter().find(|s| s.text.trim() == "OBSOLETE").unwrap();
        assert_eq!(obsolete_tag.style, Style::bold_intent(Intent::Removed));
    }

    #[test]
    fn format_result_text_done_is_added_intent() {
        // DONE = task completed, styled as closure (green), not as loss (red).
        let lines = format_result_text("[DONE] Task title — captured in memory");
        let row = lines.iter().find(|l| flat_text(l).contains("DONE")).unwrap();
        let tag = row.0.iter().find(|s| s.text.trim() == "DONE").unwrap();
        assert_eq!(tag.style, Style::bold_intent(Intent::Added));
    }

    #[test]
    fn format_result_text_dim_action_has_no_intent() {
        let lines = format_result_text("[AWKWARD] minor tweak");
        let row = lines.iter().find(|l| flat_text(l).contains("AWKWARD")).unwrap();
        let tag = row.0.iter().find(|s| s.text.trim() == "AWKWARD").unwrap();
        assert_eq!(tag.style, Style::dim());
    }

    #[test]
    fn format_result_text_filters_phase_md() {
        let lines = format_result_text("phase.md set to git-commit-reflect\n[NEW] Real content");
        let text = flat_text_all(&lines);
        assert!(!text.contains("phase.md set to"));
        assert!(text.contains("Real content"));
    }

    #[test]
    fn format_result_text_is_ansi_free() {
        let lines = format_result_text("[ADDED] entry\n★ Insight ──────────\nbody\n──────────────────");
        for line in &lines {
            for span in &line.0 {
                assert!(!span.text.contains('\x1b'), "span has ANSI: {:?}", span);
            }
        }
    }

    #[test]
    fn format_result_text_collapses_blank_runs() {
        // Two input blank lines between paragraphs must render as one blank row.
        let lines = format_result_text("first paragraph\n\n\nsecond paragraph");
        let blanks_between: Vec<usize> = lines.iter().enumerate()
            .filter(|(_, l)| l.is_blank())
            .map(|(i, _)| i)
            .collect();
        // Expect: leading blank (idx 0), one blank separator, no consecutive blanks.
        for pair in blanks_between.windows(2) {
            assert!(pair[1] - pair[0] > 1, "consecutive blank lines at {pair:?}");
        }
        // And the old indent-only-span near-blank must no longer appear.
        for line in &lines {
            let only_indent = !line.0.is_empty()
                && line.0.iter().all(|s| s.text.trim().is_empty());
            if only_indent {
                assert!(line.is_blank(), "indent-only line should be is_blank(): {line:?}");
            }
        }
    }

    #[test]
    fn clean_tool_name_strips_mcp_prefix() {
        assert_eq!(clean_tool_name("mcp__server__tool_name"), "tool_name");
        assert_eq!(clean_tool_name("Read"), "Read");
    }

}
