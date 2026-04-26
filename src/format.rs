// src/format.rs
use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::state::filenames::{
    BACKLOG_FILENAME, LATEST_SESSION_FILENAME, MEMORY_FILENAME,
};
use crate::types::LlmPhase;

fn filename_suffix_regex(name: &str) -> Regex {
    Regex::new(&format!("{}$", regex::escape(name))).unwrap()
}

/// Infer the on-disk file a `ravel-lite state` mutation will write to,
/// from a Bash tool-call's `detail` text. Returns the canonical filename
/// so the existing `PHASE_HIGHLIGHTS` regexes match on it.
///
/// Required because `ravel-lite state` writes via `--body-file <tmp>`,
/// which carries no `> ` shell-redirect marker and no destination path
/// in the visible argv. Without this inference the highlight regexes
/// never fire for state-CLI invocations.
fn ravel_state_target_file(detail: &str) -> Option<&'static str> {
    let mut tokens = detail.split_whitespace();
    while let Some(tok) = tokens.next() {
        if tok == "ravel-lite" && tokens.next() == Some("state") {
            return match tokens.next()? {
                "session-log" => Some(LATEST_SESSION_FILENAME),
                "memory" => Some(MEMORY_FILENAME),
                "backlog" => Some(BACKLOG_FILENAME),
                _ => None,
            };
        }
    }
    None
}

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
        HighlightRule { pattern: filename_suffix_regex(LATEST_SESSION_FILENAME), label: "Writing session log" },
        HighlightRule { pattern: filename_suffix_regex("commits.yaml"), label: "Writing commit spec" },
    ]);
    m.insert(LlmPhase::Reflect, vec![
        HighlightRule { pattern: filename_suffix_regex(MEMORY_FILENAME), label: "Updating memory" },
    ]);
    m.insert(LlmPhase::Dream, vec![
        HighlightRule { pattern: filename_suffix_regex(MEMORY_FILENAME), label: "Rewriting memory" },
    ]);
    m.insert(LlmPhase::Triage, vec![
        HighlightRule { pattern: filename_suffix_regex(BACKLOG_FILENAME), label: "Updating backlog" },
        HighlightRule { pattern: filename_suffix_regex("subagent-dispatch.yaml"), label: "Dispatching to related plans" },
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
    m.insert("PROMOTED",      Some(Intent::Added));     // handoff → standalone task
    m.insert("ARCHIVED",      Some(Intent::Added));     // handoff → memory.yaml entry
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
    let is_bash = tool.name.eq_ignore_ascii_case("bash");
    let is_bash_redirect_write = is_bash
        && tool.detail.as_deref().is_some_and(|d| {
            d.contains("cat ") && d.contains("> ") || d.contains("echo ") && d.contains("> ")
        });
    let inferred_state_target: Option<&'static str> = if is_bash {
        tool.detail.as_deref().and_then(ravel_state_target_file)
    } else {
        None
    };

    if is_write || is_bash_redirect_write || inferred_state_target.is_some() {
        if let Some(phase) = phase {
            let path_to_check = tool.path.as_deref()
                .or(inferred_state_target)
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
/// Recognises [ACTION] markers, `→ …` continuations under the previous
/// action, and Insight blocks.
pub fn format_result_text(text: &str) -> Vec<StyledLine> {
    static ACTION_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[\s\-\*]*\[([A-Za-z ]+)\]\s*(.*)$").unwrap()
    });
    static CONTINUATION_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^\s*→\s*(.*)$").unwrap()
    });
    static PHASE_MD_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(?:^(?:`?phase\.md`?|Phase)\s+(?:set to|written|→))|(?:phase\.md.*`git-commit-)|(?:wrote.*phase\.md)").unwrap()
    });

    // Leading blank line separates progress from result.
    let mut out: Vec<StyledLine> = vec![StyledLine::empty()];
    let mut in_insight = false;
    // Intent of the most recent action marker. Enables `→ …` continuation
    // lines (dream two-line entries, triage hand-off pairs) to render
    // aligned under the detail column and inherit the action's intent.
    // Cleared on any blank or non-continuation intervening line.
    let mut last_action_intent: Option<Option<Intent>> = None;

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
        // Filter markdown horizontal-rule separators. LLMs in the headless
        // phases often place `---` between their reasoning preamble and the
        // labelled summary; the separator is noise once labels render with
        // their own visual weight.
        {
            let trimmed = line.trim();
            if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') { continue; }
        }

        // Structured action markers
        if let Some(caps) = ACTION_RE.captures(line) {
            let tag = caps[1].to_uppercase();
            let detail = caps[2].to_string();
            if let Some(intent_opt) = ACTION_INTENTS.get(tag.as_str()).copied() {
                // Visually separate consecutive action items. `last_action_intent`
                // stays Some across a title row, its reason row, and any `→`
                // continuations, so this check fires exactly when the previous
                // emitted line belonged to the previous action block.
                if last_action_intent.is_some() {
                    push(&mut out, StyledLine::empty());
                }
                let padded = format!("{:<width$}", tag, width = *LABEL_WIDTH);
                let (tag_style, detail_style) = match intent_opt {
                    Some(intent) => (Style::bold_intent(intent), Style::intent(intent)),
                    None => (Style::dim(), Style::dim()),
                };
                // `[LABEL] title — reason` splits into a coloured title row and
                // a dim reason row under the detail column. First ` — ` only;
                // further em-dashes stay in the reason. No separator → title-only.
                let (title, reason) = match detail.split_once(" — ") {
                    Some((t, r)) => (t.to_string(), Some(r.trim().to_string())),
                    None => (detail, None),
                };
                push(&mut out, StyledLine(vec![
                    Span::plain("  "),
                    Span::styled(padded, tag_style),
                    Span::plain("  "),
                    Span::styled(title, detail_style),
                ]));
                if let Some(reason) = reason {
                    if !reason.is_empty() {
                        let indent = " ".repeat(2 + *LABEL_WIDTH + 2);
                        push(&mut out, StyledLine(vec![
                            Span::plain(indent),
                            Span::dim(reason),
                        ]));
                    }
                }
                last_action_intent = Some(intent_opt);
                continue;
            }
        }

        // `→ …` continuation under the most recent action — re-indents to the
        // detail column so the post-change state sits directly below the label.
        if let Some(caps) = CONTINUATION_RE.captures(line) {
            if let Some(intent_opt) = last_action_intent {
                let rest = caps[1].to_string();
                let detail_style = match intent_opt {
                    Some(intent) => Style::intent(intent),
                    None => Style::dim(),
                };
                let indent = " ".repeat(2 + *LABEL_WIDTH + 2);
                push(&mut out, StyledLine(vec![
                    Span::plain(indent),
                    Span::styled(format!("→ {rest}"), detail_style),
                ]));
                continue;
            }
        }

        // Insight block opening
        if line.contains("★") && line.contains("Insight") && line.contains("─") {
            in_insight = true;
            last_action_intent = None;
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
            last_action_intent = None;
            push(&mut out, StyledLine::empty());
            continue;
        }
        // Insight content or regular text — indent, dim
        last_action_intent = None;
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
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.yaml".to_string()), detail: None },
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
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.yaml".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(r1.persist);

        let r2 = format_tool_call(
            &ToolCall { name: "Edit".to_string(), path: Some("/plan/memory.yaml".to_string()), detail: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(!r2.persist);
        assert!(r2.is_empty());
    }

    #[test]
    fn format_tool_call_highlights_ravel_state_session_log_set_latest() {
        // analyse-work writes latest-session.yaml via the state CLI, not a
        // raw shell redirect. The highlight rule must still fire.
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall {
                name: "Bash".to_string(),
                path: None,
                detail: Some(
                    "ravel-lite state session-log set-latest LLM_STATE/core --body-file /tmp/abc"
                        .to_string(),
                ),
            },
            Some(LlmPhase::AnalyseWork),
            &mut shown,
        );
        assert!(result.persist, "label must persist");
        assert!(flat_text(&result.lines[0]).contains("Writing session log"));
    }

    #[test]
    fn format_tool_call_highlights_ravel_state_memory_add() {
        // reflect mutates memory.yaml via `state memory add` etc.
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall {
                name: "Bash".to_string(),
                path: None,
                detail: Some(
                    "ravel-lite state memory add LLM_STATE/core --id foo --body-file /tmp/x"
                        .to_string(),
                ),
            },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.persist);
        assert!(flat_text(&result.lines[0]).contains("Updating memory"));
    }

    #[test]
    fn format_tool_call_highlights_ravel_state_backlog_set_status() {
        // triage flips task statuses via `state backlog set-status`.
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall {
                name: "Bash".to_string(),
                path: None,
                detail: Some(
                    "ravel-lite state backlog set-status LLM_STATE/core some-task done"
                        .to_string(),
                ),
            },
            Some(LlmPhase::Triage),
            &mut shown,
        );
        assert!(result.persist);
        assert!(flat_text(&result.lines[0]).contains("Updating backlog"));
    }

    #[test]
    fn ravel_state_target_file_extracts_subcommand_target() {
        assert_eq!(
            ravel_state_target_file("ravel-lite state memory add LLM_STATE/core"),
            Some(MEMORY_FILENAME)
        );
        assert_eq!(
            ravel_state_target_file("ravel-lite state backlog set-status x done"),
            Some(BACKLOG_FILENAME)
        );
        assert_eq!(
            ravel_state_target_file("ravel-lite state session-log set-latest x"),
            Some(LATEST_SESSION_FILENAME)
        );
        // Non-mutating subcommands and unrelated commands return None.
        assert_eq!(
            ravel_state_target_file("ravel-lite state phase-summary render x"),
            None
        );
        assert_eq!(ravel_state_target_file("git status"), None);
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
    fn format_result_text_filters_markdown_horizontal_rule() {
        // LLMs in reflect/dream/triage put a `---` separator between the
        // reasoning preamble and the labelled summary. The separator is
        // noise once the labels are already visually distinct — filter it.
        let lines = format_result_text(
            "I noticed the backlog had drifted.\n\n---\n\n[NEW] Real content",
        );
        let text = flat_text_all(&lines);
        assert!(text.contains("noticed the backlog"), "preamble must survive");
        assert!(text.contains("Real content"), "label must survive");
        for line in &lines {
            let flat = flat_text(line);
            assert!(flat.trim() != "---", "bare `---` line must be filtered: {flat:?}");
        }
    }

    #[test]
    fn format_result_text_filters_longer_dash_runs() {
        // Markdown accepts 3+ dashes as a horizontal rule; variants like
        // `----` or `-----` should filter the same as `---`.
        let lines = format_result_text("before\n----\nafter\n-----\nend");
        for line in &lines {
            let flat = flat_text(line);
            let trimmed = flat.trim();
            assert!(
                !trimmed.chars().all(|c| c == '-') || trimmed.is_empty(),
                "all-dashes line survived: {flat:?}"
            );
        }
    }

    #[test]
    fn format_result_text_preserves_lines_mixing_dashes_and_text() {
        // `--- foo` or `foo --- bar` are content, not separators. Only
        // pure dash runs are filtered.
        let lines = format_result_text("--- foo\nfoo --- bar\n-- two dashes only");
        let text = flat_text_all(&lines);
        assert!(text.contains("--- foo"), "dashes+text must survive");
        assert!(text.contains("foo --- bar"), "dashes in middle must survive");
        assert!(text.contains("-- two dashes"), "two-dash line is not an HR and must survive");
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

    #[test]
    fn format_result_text_promoted_and_archived_are_recognised_actions() {
        let lines = format_result_text(
            "[PROMOTED] Handoff A — from completed task\n\
             [ARCHIVED] Handoff B — to memory.yaml, from completed task",
        );
        let promoted = lines.iter().find(|l| flat_text(l).contains("PROMOTED")).unwrap();
        let promoted_tag = promoted.0.iter().find(|s| s.text.trim() == "PROMOTED").unwrap();
        assert_eq!(promoted_tag.style, Style::bold_intent(Intent::Added));

        let archived = lines.iter().find(|l| flat_text(l).contains("ARCHIVED")).unwrap();
        let archived_tag = archived.0.iter().find(|s| s.text.trim() == "ARCHIVED").unwrap();
        assert_eq!(archived_tag.style, Style::bold_intent(Intent::Added));

        // Brackets must be stripped (proof that the line went through the action
        // formatter, not the generic dim-text fallthrough).
        assert!(!flat_text(promoted).contains("[PROMOTED]"));
        assert!(!flat_text(archived).contains("[ARCHIVED]"));
    }

    #[test]
    fn format_result_text_splits_title_and_reason_on_em_dash() {
        // `[LABEL] title — reason` emits two rows: title on the action row,
        // reason indented to the detail column on the next row, dimmed.
        let lines = format_result_text("[NEW] Task title — reason for being new");
        let title_row = lines.iter().find(|l| flat_text(l).contains("NEW")).unwrap();
        let title_text = flat_text(title_row);
        assert!(title_text.contains("Task title"), "title on action row: {title_text:?}");
        assert!(!title_text.contains("reason for being new"), "reason must not appear on action row: {title_text:?}");
        assert!(!title_text.contains(" — "), "em-dash separator must be stripped: {title_text:?}");

        let reason_row = lines.iter().find(|l| flat_text(l).contains("reason for being new")).unwrap();
        let reason_text = flat_text(reason_row);
        let indent_width = 2 + *LABEL_WIDTH + 2;
        assert!(
            reason_text.starts_with(&" ".repeat(indent_width)),
            "reason must indent to detail column: {reason_text:?}"
        );
        let reason_span = reason_row.0.iter().find(|s| s.text.contains("reason for being new")).unwrap();
        assert_eq!(reason_span.style, Style::dim(), "reason must be dim");
    }

    #[test]
    fn format_result_text_action_without_em_dash_stays_single_row() {
        // No separator → single row, no dim reason row emitted.
        let lines = format_result_text("[AWKWARD] minor tweak");
        let rows_with_text: Vec<_> = lines
            .iter()
            .filter(|l| !l.is_blank())
            .collect();
        assert_eq!(rows_with_text.len(), 1, "single action row expected: {rows_with_text:?}");
        let text = flat_text(rows_with_text[0]);
        assert!(text.contains("AWKWARD"));
        assert!(text.contains("minor tweak"));
    }

    #[test]
    fn format_result_text_splits_on_first_em_dash_only() {
        // Reason may itself contain ` — `; only the first occurrence is a split.
        let lines = format_result_text("[STALE] entry — old behaviour — new behaviour");
        let title_row = lines.iter().find(|l| flat_text(l).contains("STALE")).unwrap();
        assert!(flat_text(title_row).contains("entry"));
        assert!(!flat_text(title_row).contains("old behaviour"));

        let reason_row = lines.iter().find(|l| flat_text(l).contains("old behaviour")).unwrap();
        let reason_text = flat_text(reason_row);
        assert!(reason_text.contains("old behaviour — new behaviour"), "inner em-dash preserved: {reason_text:?}");
    }

    #[test]
    fn format_result_text_empty_reason_skips_reason_row() {
        // `[NEW] title — ` (trailing em-dash, empty reason) must not emit a blank dim row.
        let lines = format_result_text("[NEW] title — ");
        let rows_with_text: Vec<_> = lines
            .iter()
            .filter(|l| !l.is_blank())
            .collect();
        assert_eq!(rows_with_text.len(), 1, "empty reason must not emit a row: {rows_with_text:?}");
    }

    #[test]
    fn format_result_text_reason_row_does_not_break_arrow_continuation() {
        // After emitting title + dim reason, a subsequent `→ …` still chains
        // to the action's intent (dream format relies on this).
        let lines = format_result_text("[VERBOSE] heading — what was wordy\n    → tightened form");
        let arrow_row = lines.iter().find(|l| flat_text(l).contains("tightened form")).unwrap();
        let arrow_span = arrow_row.0.iter().find(|s| s.text.contains("→")).expect("arrow span");
        assert_eq!(arrow_span.style, Style::intent(Intent::Meta));
    }

    #[test]
    fn format_result_text_continuation_aligns_under_detail_column() {
        let input = "[AWKWARD] heading — old phrasing\n   → new phrasing";
        let lines = format_result_text(input);

        let cont_row = lines
            .iter()
            .find(|l| {
                let t = flat_text(l);
                t.contains("→") && t.contains("new phrasing")
            })
            .expect("continuation row");
        let cont_text = flat_text(cont_row);
        let indent_width = 2 + *LABEL_WIDTH + 2;
        assert!(
            cont_text.starts_with(&" ".repeat(indent_width)),
            "continuation must indent to detail column: got {cont_text:?}"
        );
        assert!(cont_text[indent_width..].starts_with("→ new phrasing"));
    }

    #[test]
    fn format_result_text_continuation_inherits_action_intent() {
        // OVERLAPPING → Meta; continuation detail span must carry the same intent.
        let lines = format_result_text("[OVERLAPPING] A + B\n   → merged heading");
        let cont_row = lines
            .iter()
            .find(|l| flat_text(l).contains("merged heading"))
            .unwrap();
        let arrow_span = cont_row
            .0
            .iter()
            .find(|s| s.text.contains("→"))
            .expect("arrow span");
        assert_eq!(arrow_span.style, Style::intent(Intent::Meta));
    }

    #[test]
    fn format_result_text_continuation_without_prior_action_falls_through() {
        // A stray arrow line with no preceding action stays as ordinary dim text.
        let lines = format_result_text("just prose\n   → orphan arrow");
        let orphan = lines
            .iter()
            .find(|l| flat_text(l).contains("orphan arrow"))
            .unwrap();
        let text = flat_text(orphan);
        let indent_width = 2 + *LABEL_WIDTH + 2;
        assert!(
            !text.starts_with(&" ".repeat(indent_width)),
            "orphan arrow must not be indented to detail column: {text:?}"
        );
    }

    #[test]
    fn format_result_text_inserts_blank_between_consecutive_actions() {
        // Two actions back-to-back must render with a blank row between them.
        let lines = format_result_text("[NEW] first — reason one\n[OBSOLETE] second — reason two");
        let first_idx = lines.iter().position(|l| flat_text(l).contains("first")).unwrap();
        let second_idx = lines.iter().position(|l| flat_text(l).contains("second")).unwrap();
        // Expect: first-title, first-reason, blank, second-title, second-reason.
        assert!(second_idx > first_idx + 2, "second action immediately follows first: {lines:?}");
        let between: Vec<_> = lines[first_idx + 1..second_idx].iter().collect();
        assert!(
            between.iter().any(|l| l.is_blank()),
            "expected blank row between actions: {between:?}"
        );
    }

    #[test]
    fn format_result_text_no_blank_before_first_action() {
        // Output starts with one leading blank (progress/result separator); no
        // extra blank should appear just because the first line is an action.
        let lines = format_result_text("[NEW] only action");
        let action_idx = lines.iter().position(|l| flat_text(l).contains("only action")).unwrap();
        assert_eq!(action_idx, 1, "expected leading blank then action row: {lines:?}");
    }

    #[test]
    fn format_result_text_preserves_user_blank_between_actions() {
        // User already put a blank line between actions → must not double it.
        let lines = format_result_text("[NEW] a\n\n[OBSOLETE] b");
        let a_idx = lines.iter().position(|l| flat_text(l).contains(" a")).unwrap();
        let b_idx = lines.iter().position(|l| flat_text(l).contains(" b")).unwrap();
        let blanks_between = lines[a_idx + 1..b_idx].iter().filter(|l| l.is_blank()).count();
        assert_eq!(blanks_between, 1, "exactly one blank between actions: {lines:?}");
    }

    #[test]
    fn format_result_text_blank_line_breaks_continuation_association() {
        let input = "[AWKWARD] heading — old\n\n   → stray arrow";
        let lines = format_result_text(input);
        let stray = lines.iter().find(|l| flat_text(l).contains("stray")).unwrap();
        let text = flat_text(stray);
        let indent_width = 2 + *LABEL_WIDTH + 2;
        assert!(
            !text.starts_with(&" ".repeat(indent_width)),
            "blank line must break continuation chain: {text:?}"
        );
    }

    /// End-to-end pin on the two-part output contract: narrative preamble
    /// (LLM-authored, intent-bearing labels carrying the *why*) + blank-line
    /// separator + structural label list (renderer-derived, carrying the
    /// *what*). Surface area covered:
    ///
    /// 1. Every intent-bearing label (`PROMOTED`, `ARCHIVED`, `REPRIORITISED`,
    ///    `BLOCKER`, `DISPATCH`) renders with bold-intent tag + dim reason row.
    /// 2. Structural labels (`DONE`, `NEW`, `OBSOLETE`, `STATS`) render after
    ///    the blank-line separator.
    /// 3. `REPRIORITISED` appears in *both* halves (intent preamble + structural
    ///    diff) and both occurrences render as styled tag spans.
    /// 4. Prose paragraph survives unchanged as dim text.
    /// 5. The two halves are separated by ≥1 blank line.
    /// 6. The whole output is ANSI-free (renderer-agnostic styling contract).
    ///
    /// Targets the regression class introduced by ce240f9 (a prompt-side
    /// drift that stopped the LLM emitting intent labels and went undetected
    /// because no test exercised the full pipeline).
    #[test]
    fn format_result_text_pins_two_part_triage_output_shape() {
        let input = "\
Triage swept the backlog after the cycle's verification work. Two hand-offs were ripe for promotion and one task moved up after the new dependency landed.

[PROMOTED] Decompose ingest pipeline — design settled in this cycle's reflection
[ARCHIVED] Datalog cross-artifact queries — strategic but not yet concrete
[REPRIORITISED] Wire health endpoint — moved up after ingest dependency landed
[BLOCKER] Schema migration spike — extracted from Decompose ingest pipeline
[DISPATCH] child: APIAnyware-MacOS — propagate verification-tooling pattern

[DONE] Wire health endpoint
[NEW] Schema migration spike
[REPRIORITISED] Wire health endpoint
[OBSOLETE] Legacy ingest path
[STATS] 2 promoted, 1 archived, 1 dispatched";

        let lines = format_result_text(input);

        // 1. Intent-bearing labels: tag span carries bold-intent, reason span is dim.
        let intent_cases = [
            ("PROMOTED",      Intent::Added,   "design settled in this cycle's reflection"),
            ("ARCHIVED",      Intent::Added,   "strategic but not yet concrete"),
            ("REPRIORITISED", Intent::Changed, "moved up after ingest dependency landed"),
            ("BLOCKER",       Intent::Changed, "extracted from Decompose ingest pipeline"),
            ("DISPATCH",      Intent::Added,   "propagate verification-tooling pattern"),
        ];
        for (label, intent, reason) in intent_cases {
            let tag_span = lines
                .iter()
                .flat_map(|l| l.0.iter())
                .find(|s| s.text.trim() == label && s.style == Style::bold_intent(intent))
                .unwrap_or_else(|| panic!("missing bold-intent tag span for {label}"));
            let _ = tag_span;
            let reason_span = lines
                .iter()
                .flat_map(|l| l.0.iter())
                .find(|s| s.text.contains(reason))
                .unwrap_or_else(|| panic!("missing reason span for {label}: {reason}"));
            assert_eq!(reason_span.style, Style::dim(), "{label}: reason span must be dim");
        }

        // 2. Structural labels render with their own intent (DONE → Added,
        //    NEW → Added, OBSOLETE → Removed, STATS → dim/none).
        let structural_cases = [
            ("DONE",     Some(Style::bold_intent(Intent::Added))),
            ("NEW",      Some(Style::bold_intent(Intent::Added))),
            ("OBSOLETE", Some(Style::bold_intent(Intent::Removed))),
            ("STATS",    Some(Style::dim())),
        ];
        for (label, expected_style) in structural_cases {
            let tag_span = lines
                .iter()
                .flat_map(|l| l.0.iter())
                .find(|s| s.text.trim() == label)
                .unwrap_or_else(|| panic!("missing structural tag span for {label}"));
            assert_eq!(
                Some(tag_span.style),
                expected_style,
                "{label}: tag span style mismatch",
            );
        }

        // 3. REPRIORITISED appears twice (intent preamble + structural diff).
        let reprior_tag_count = lines
            .iter()
            .flat_map(|l| l.0.iter())
            .filter(|s| s.text.trim() == "REPRIORITISED")
            .count();
        assert_eq!(
            reprior_tag_count, 2,
            "REPRIORITISED should appear twice (intent + structural), got {reprior_tag_count}",
        );

        // 4. Preamble prose paragraph survives as dim text.
        let prose_anchor = "Two hand-offs were ripe for promotion";
        let prose_span = lines
            .iter()
            .flat_map(|l| l.0.iter())
            .find(|s| s.text.contains(prose_anchor))
            .expect("preamble prose must survive");
        assert_eq!(prose_span.style, Style::dim(), "preamble prose must render as dim");

        // 5. Preamble half ends and structural half begins separated by ≥1 blank.
        let last_intent_idx = lines
            .iter()
            .rposition(|l| l.0.iter().any(|s| s.text.trim() == "DISPATCH"))
            .expect("DISPATCH row");
        let first_structural_idx = lines
            .iter()
            .position(|l| l.0.iter().any(|s| s.text.trim() == "DONE"))
            .expect("DONE row");
        assert!(
            first_structural_idx > last_intent_idx,
            "structural half must follow intent half",
        );
        let between_blanks = lines[last_intent_idx + 1..first_structural_idx]
            .iter()
            .filter(|l| l.is_blank())
            .count();
        assert!(
            between_blanks >= 1,
            "preamble and structural halves must be separated by ≥1 blank line, got 0",
        );

        // 6. Renderer is ANSI-free end-to-end.
        for line in &lines {
            for span in &line.0 {
                assert!(
                    !span.text.contains('\x1b'),
                    "ANSI byte leaked into rendered output: {span:?}",
                );
            }
        }
    }
}
