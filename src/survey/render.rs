// src/survey/render.rs
//
// Deterministic rendering of a parsed SurveyResponse into the
// human-readable survey output. Layout decisions (column widths,
// wrap points, section headings) live here; no I/O, no network.

use super::schema::{Blocker, ParallelStream, PlanRow, Recommendation, SurveyResponse};

/// Column width target for wrapped prose sections. Chosen to fit a
/// standard 80-column terminal with a small margin.
const WRAP_WIDTH: usize = 78;

/// Key describing the compact `U/B/D/R` task-counts column used in
/// the per-plan summary. Emitted at the top of that section so readers
/// see it before the data.
const PLAN_SUMMARY_KEY: &str = "\
Task counts column: U/B/D/R  (`-` = zero)
  U = unblocked  — not_started tasks with no unmet dependencies
  B = blocked    — status=blocked or not_started with unmet deps
  D = done       — status=done
  R = received   — items under `## Received` not yet triaged
";

/// Convert a task-count integer into the cell-rendering form: zeros
/// become `-` so the table's eye is drawn to non-zero values.
fn format_count(n: usize) -> String {
    if n == 0 {
        "-".to_string()
    } else {
        n.to_string()
    }
}

/// Render the complete survey output: top heading, the three
/// sections, each with its own renderer. Deterministic and unit-tested.
pub fn render_survey_output(response: &SurveyResponse) -> String {
    let mut out = String::new();
    out.push_str("# Plan Status Survey\n\n");

    out.push_str("## Per-plan summary\n\n");
    out.push_str(&render_plan_summary(&response.plans));
    out.push('\n');

    out.push_str("## Cross-plan blockers\n\n");
    out.push_str(&render_blockers(&response.cross_plan_blockers));
    out.push('\n');

    out.push_str("## Parallel streams\n\n");
    out.push_str(&render_streams(&response.parallel_streams));
    out.push('\n');

    out.push_str("## Recommended invocation order\n\n");
    out.push_str(&render_recommendations(&response.recommended_invocation_order));

    out
}

/// Render the per-plan summary grouped by project. Each project gets
/// a `### <project>` heading; under each heading, one indented line
/// per plan shows the plan name, phase, and compact `U/B/D/R` counts,
/// with notes (if any) as a wrapped body line below.
///
/// The previous wide monospace table — PROJECT/PLAN/PHASE and four
/// separate count columns + NOTES — ran well past 100 characters
/// wide in practice. Collapsing the four counts into a single
/// slash-separated field, moving project to a heading, and moving
/// notes to a body line keeps each line around 60 characters while
/// preserving all the information.
fn render_plan_summary(plans: &[PlanRow]) -> String {
    if plans.is_empty() {
        return "  No plans discovered.\n".to_string();
    }

    // Column widths are computed globally across all plans so columns
    // align vertically across project sections — easier to compare at
    // a glance.
    let plan_width = plans.iter().map(|p| p.plan.len()).max().unwrap_or(0);
    let phase_width = plans.iter().map(|p| p.phase.len()).max().unwrap_or(0);

    let mut out = String::new();
    out.push_str(PLAN_SUMMARY_KEY);
    out.push('\n');

    let mut current_project: Option<&str> = None;
    for plan in plans {
        if Some(plan.project.as_str()) != current_project {
            // Blank line before a new project section (except the
            // very first — the key above is followed by a blank
            // already).
            if current_project.is_some() {
                out.push('\n');
            }
            out.push_str(&format!("### {}\n\n", plan.project));
            current_project = Some(&plan.project);
        } else {
            // Blank line between plans within the same project.
            out.push('\n');
        }

        out.push_str(&format!(
            "  {:<plan_w$}  {:<phase_w$}  {}\n",
            plan.plan,
            plan.phase,
            compact_counts(plan),
            plan_w = plan_width,
            phase_w = phase_width,
        ));
        if !plan.notes.trim().is_empty() {
            out.push_str(&render_wrapped_bullet("      ", &plan.notes));
            out.push('\n');
        }
    }
    out
}

/// Format the four task counts as a compact `U/B/D/R` field with `-`
/// for zero values.
fn compact_counts(p: &PlanRow) -> String {
    format!(
        "{}/{}/{}/{}",
        format_count(p.unblocked),
        format_count(p.blocked),
        format_count(p.done),
        format_count(p.received),
    )
}

/// Render the cross-plan blockers section. Each entry is a header
/// line ("  - X blocked on Y") followed by an indented rationale body
/// that wraps at WRAP_WIDTH. Splitting header from body means the
/// body's wrap continuations can never be confused with a new logical
/// line — within the body, every line IS a wrap.
fn render_blockers(blockers: &[Blocker]) -> String {
    if blockers.is_empty() {
        return "  None detected.\n".to_string();
    }
    let mut out = String::new();
    for (i, b) in blockers.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("  - {} blocked on {}\n", b.blocked, b.blocker));
        out.push_str(&render_wrapped_bullet("      ", &b.rationale));
        out.push('\n');
    }
    out
}

/// Render parallel streams. Each entry is a header line
/// ("  Stream N: name") followed by two labeled sub-lines
/// ("Plans:" and "Rationale:"), both indented under the header and
/// wrapping with hanging indent under the label's content. Labels are
/// padded to the same width so the content columns align across
/// sub-lines — wraps land in that same content column, visibly deeper
/// than the label itself, so wraps are unambiguously wraps.
fn render_streams(streams: &[ParallelStream]) -> String {
    if streams.is_empty() {
        return "  None identified.\n".to_string();
    }
    // Pad to the widest label so "Plans:" and "Rationale:" content
    // columns line up.
    const PLANS_LABEL: &str = "      Plans:     ";
    const RATIONALE_LABEL: &str = "      Rationale: ";
    let mut out = String::new();
    for (i, stream) in streams.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("  Stream {}: {}\n", i + 1, stream.name));
        let plans_joined = stream.plans.join(", ");
        out.push_str(&render_wrapped_bullet(PLANS_LABEL, &plans_joined));
        out.push('\n');
        out.push_str(&render_wrapped_bullet(RATIONALE_LABEL, &stream.rationale));
        out.push('\n');
    }
    out
}

/// Render recommended invocations as numbered entries. The visible
/// number comes from each recommendation's `order` field — multiple
/// recommendations can share a number to indicate they're
/// parallelisable. A short explanatory note sits at the top so
/// duplicated numbers don't look like a bug.
fn render_recommendations(recs: &[Recommendation]) -> String {
    if recs.is_empty() {
        return "  None available.\n".to_string();
    }
    let mut out = String::new();
    out.push_str("Items sharing a number can run in parallel; within a number,\n");
    out.push_str("list order is a secondary priority (earlier entries unblock more).\n\n");
    for (i, r) in recs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("  {}. {}\n", r.order, r.plan));
        out.push_str(&render_wrapped_bullet("       ", &r.rationale));
        out.push('\n');
    }
    out
}

/// Render a single bulleted or numbered entry: first line starts with
/// `prefix`, continuation lines are indented to align under the first
/// content character. Whitespace in `text` (including embedded
/// newlines from YAML block scalars) is normalised via `split_whitespace`.
fn render_wrapped_bullet(prefix: &str, text: &str) -> String {
    let content_width = WRAP_WIDTH.saturating_sub(prefix.len()).max(1);
    let lines = wrap_at(text, content_width);
    if lines.is_empty() {
        return prefix.trim_end().to_string();
    }
    let continuation = " ".repeat(prefix.len());
    let mut out = String::new();
    out.push_str(prefix);
    out.push_str(&lines[0]);
    for line in &lines[1..] {
        out.push('\n');
        out.push_str(&continuation);
        out.push_str(line);
    }
    out
}

/// Greedy word wrap: split `text` on whitespace and pack words into
/// lines no wider than `content_width`. Single words longer than
/// `content_width` stand alone on their own line.
fn wrap_at(text: &str, content_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if candidate_len > content_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if current.is_empty() {
            current.push_str(word);
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn row(
        project: &str,
        plan: &str,
        phase: &str,
        u: usize,
        b: usize,
        d: usize,
        r: usize,
        notes: &str,
    ) -> PlanRow {
        PlanRow {
            project: project.into(),
            plan: plan.into(),
            phase: phase.into(),
            unblocked: u,
            blocked: b,
            done: d,
            received: r,
            notes: notes.into(),
            input_hash: String::new(),
        }
    }

    #[test]
    fn render_plan_summary_groups_plans_under_project_headings() {
        let plans = vec![
            row("ProjectA", "alpha", "work", 1, 0, 0, 0, ""),
            row("ProjectA", "beta",  "work", 0, 0, 0, 0, ""),
            row("ProjectB", "gamma", "work", 0, 0, 0, 0, ""),
        ];
        let out = render_plan_summary(&plans);
        // Each project appears as a `### Project` heading exactly once.
        assert_eq!(out.matches("### ProjectA").count(), 1);
        assert_eq!(out.matches("### ProjectB").count(), 1);
        // ProjectA appears before ProjectB (input order is preserved).
        let a_idx = out.find("### ProjectA").unwrap();
        let b_idx = out.find("### ProjectB").unwrap();
        assert!(a_idx < b_idx);
        // Plans appear under their headings.
        let alpha_idx = out.find("alpha").unwrap();
        let gamma_idx = out.find("gamma").unwrap();
        assert!(a_idx < alpha_idx && alpha_idx < b_idx);
        assert!(b_idx < gamma_idx);
    }

    #[test]
    fn render_plan_summary_aligns_plan_and_phase_columns_globally() {
        // Two projects with mixed plan-name lengths. The widest plan
        // name sets the column width for ALL plans, regardless of
        // project, so columns align across the whole section.
        let plans = vec![
            row("P", "short", "work", 1, 0, 0, 0, ""),
            row("P", "a-much-longer-plan-name", "triage", 2, 0, 0, 0, ""),
            row("Q", "mid-name", "reflect", 3, 0, 0, 0, ""),
        ];
        let out = render_plan_summary(&plans);
        let line_short = out.lines().find(|l| l.contains("  short ")).unwrap();
        let line_long = out.lines().find(|l| l.contains("a-much-longer-plan-name")).unwrap();
        let line_mid = out.lines().find(|l| l.contains("mid-name")).unwrap();
        let phase_col_short = line_short.find("work").unwrap();
        let phase_col_long = line_long.find("triage").unwrap();
        let phase_col_mid = line_mid.find("reflect").unwrap();
        assert_eq!(phase_col_short, phase_col_long);
        assert_eq!(phase_col_short, phase_col_mid);
    }

    #[test]
    fn render_plan_summary_renders_zero_counts_as_dash() {
        let plans = vec![row("P", "x", "work", 0, 0, 0, 0, "")];
        let out = render_plan_summary(&plans);
        assert!(out.contains("-/-/-/-"));
        for line in out.lines().filter(|l| l.starts_with("  ") && !l.starts_with("   ")) {
            assert!(!line.contains('0'), "unexpected zero digit: {line:?}");
        }
    }

    #[test]
    fn render_plan_summary_renders_nonzero_counts_as_digits() {
        let plans = vec![row("P", "x", "work", 3, 15, 5, 2, "")];
        let out = render_plan_summary(&plans);
        assert!(out.contains("3/15/5/2"));
    }

    #[test]
    fn render_plan_summary_renders_notes_as_indented_body_line() {
        let plans = vec![row("P", "x", "work", 1, 0, 0, 0, "a non-empty note")];
        let out = render_plan_summary(&plans);
        assert!(out.contains("      a non-empty note"));
    }

    #[test]
    fn render_plan_summary_omits_body_line_when_notes_empty() {
        let plans = vec![row("P", "x", "work", 1, 0, 0, 0, "")];
        let out = render_plan_summary(&plans);
        let has_body = out
            .lines()
            .any(|l| l.starts_with("      ") && !l.trim().is_empty() && !l.contains("="));
        assert!(!has_body, "unexpected body line when notes are empty:\n{out}");
    }

    #[test]
    fn render_plan_summary_emits_key_before_project_headings() {
        let plans = vec![row("P", "x", "work", 1, 0, 0, 0, "")];
        let out = render_plan_summary(&plans);
        let key_idx = out.find("Task counts column").unwrap();
        let project_idx = out.find("### P").unwrap();
        assert!(key_idx < project_idx);
    }

    #[test]
    fn render_blockers_empty_yields_none_detected() {
        let out = render_blockers(&[]);
        assert!(out.contains("None detected."));
    }

    #[test]
    fn render_blockers_separates_entries_with_blank_lines() {
        let blockers = vec![
            Blocker {
                blocked: "P/a".into(),
                blocker: "Q/b".into(),
                rationale: "short.".into(),
            },
            Blocker {
                blocked: "P/c".into(),
                blocker: "R/d".into(),
                rationale: "short.".into(),
            },
        ];
        let out = render_blockers(&blockers);
        assert!(out.contains("\n\n  - P/c"), "missing blank line separator: {out}");
    }

    #[test]
    fn render_blockers_wraps_long_rationales() {
        let long_text = "word ".repeat(40); // ~200 chars
        let blockers = vec![Blocker {
            blocked: "P/a".into(),
            blocker: "Q/b".into(),
            rationale: long_text,
        }];
        let out = render_blockers(&blockers);
        for line in out.lines() {
            assert!(
                line.chars().count() <= WRAP_WIDTH,
                "line exceeds wrap width: {line}"
            );
        }
    }

    #[test]
    fn render_blockers_body_indent_is_distinct_from_header() {
        let blockers = vec![Blocker {
            blocked: "P/a".into(),
            blocker: "Q/b".into(),
            rationale: "word ".repeat(40),
        }];
        let out = render_blockers(&blockers);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("  - "));
        assert!(lines[0].contains("P/a blocked on Q/b"));
        for line in &lines[1..] {
            if !line.is_empty() {
                assert!(line.starts_with("      "), "body not at 6-space indent: {line:?}");
                assert!(!line.starts_with("  - "), "body line looks like a new header: {line:?}");
            }
        }
    }

    #[test]
    fn render_streams_empty_yields_none_identified() {
        let out = render_streams(&[]);
        assert!(out.contains("None identified."));
    }

    #[test]
    fn render_streams_includes_name_and_plans_and_rationale() {
        let streams = vec![ParallelStream {
            name: "Critical path".into(),
            plans: vec!["P/a".into(), "P/b".into()],
            rationale: "Sequential chain.".into(),
        }];
        let out = render_streams(&streams);
        assert!(out.contains("Critical path"));
        assert!(out.contains("P/a"));
        assert!(out.contains("P/b"));
        assert!(out.contains("Sequential chain."));
    }

    #[test]
    fn render_streams_separates_entries_with_blank_lines() {
        let streams = vec![
            ParallelStream {
                name: "One".into(),
                plans: vec!["P/a".into()],
                rationale: "first.".into(),
            },
            ParallelStream {
                name: "Two".into(),
                plans: vec!["P/b".into()],
                rationale: "second.".into(),
            },
        ];
        let out = render_streams(&streams);
        assert!(out.contains("\n\n  Stream 2: Two"), "missing blank line separator: {out}");
    }

    #[test]
    fn render_streams_emits_labeled_sub_lines() {
        let streams = vec![ParallelStream {
            name: "Critical path".into(),
            plans: vec!["P/a".into()],
            rationale: "Do the thing.".into(),
        }];
        let out = render_streams(&streams);
        assert!(out.contains("  Stream 1: Critical path"));
        assert!(out.contains("      Plans:"));
        assert!(out.contains("      Rationale:"));
    }

    #[test]
    fn render_streams_wraps_long_plan_lists_at_width() {
        let streams = vec![ParallelStream {
            name: "Big stream".into(),
            plans: (0..10)
                .map(|i| format!("Ravel/sub-X-very-long-plan-name-{i}"))
                .collect(),
            rationale: "fine.".into(),
        }];
        let out = render_streams(&streams);
        for line in out.lines() {
            assert!(line.chars().count() <= WRAP_WIDTH, "line too wide: {line}");
        }
    }

    #[test]
    fn render_recommendations_uses_order_field_for_display_number() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), order: 1, rationale: "first.".into() },
            Recommendation { plan: "P/b".into(), order: 2, rationale: "second.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("  1. P/a"));
        assert!(out.contains("  2. P/b"));
        let one_idx = out.find("  1. P/a").unwrap();
        let two_idx = out.find("  2. P/b").unwrap();
        assert!(one_idx < two_idx);
    }

    #[test]
    fn render_recommendations_preserves_shared_order_numbers_as_parallelisable() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), order: 1, rationale: "first.".into() },
            Recommendation { plan: "P/b".into(), order: 1, rationale: "also first.".into() },
            Recommendation { plan: "P/c".into(), order: 2, rationale: "later.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("  1. P/a"));
        assert!(out.contains("  1. P/b"));
        assert!(out.contains("  2. P/c"));
        assert_eq!(out.matches("  1. ").count(), 2, "expected two items at order 1");
    }

    #[test]
    fn render_recommendations_includes_parallel_note_when_non_empty() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), order: 1, rationale: "go.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("Items sharing a number can run in parallel"));
    }

    #[test]
    fn render_recommendations_omits_parallel_note_when_empty() {
        let out = render_recommendations(&[]);
        assert!(!out.contains("Items sharing a number"));
    }

    #[test]
    fn render_recommendations_separates_entries_with_blank_lines() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), order: 1, rationale: "first.".into() },
            Recommendation { plan: "P/b".into(), order: 2, rationale: "second.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("\n\n  2. "));
    }

    #[test]
    fn render_survey_output_contains_all_four_sections_in_order() {
        let response = SurveyResponse {
            plans: vec![row("P", "x", "work", 1, 0, 0, 0, "")],
            cross_plan_blockers: vec![],
            parallel_streams: vec![],
            recommended_invocation_order: vec![Recommendation {
                plan: "P/x".into(),
                order: 1,
                rationale: "do it.".into(),
            }],
        };
        let out = render_survey_output(&response);
        assert!(out.contains("# Plan Status Survey"));
        let summary = out.find("## Per-plan summary").unwrap();
        let blockers = out.find("## Cross-plan blockers").unwrap();
        let streams = out.find("## Parallel streams").unwrap();
        let recommendations = out.find("## Recommended invocation order").unwrap();
        assert!(summary < blockers && blockers < streams && streams < recommendations);
        assert!(out.contains("None detected."));
        assert!(out.contains("None identified."));
    }

    #[test]
    fn render_survey_output_includes_counts_key_in_plan_summary() {
        let response = SurveyResponse {
            plans: vec![row("P", "x", "work", 1, 0, 0, 0, "")],
            cross_plan_blockers: vec![],
            parallel_streams: vec![],
            recommended_invocation_order: vec![],
        };
        let out = render_survey_output(&response);
        let summary_idx = out.find("## Per-plan summary").unwrap();
        let key_idx = out.find("Task counts column").unwrap();
        let blockers_idx = out.find("## Cross-plan blockers").unwrap();
        assert!(summary_idx < key_idx && key_idx < blockers_idx);
        assert!(out.contains("U = unblocked"));
        assert!(out.contains("B = blocked"));
        assert!(out.contains("D = done"));
        assert!(out.contains("R = received"));
    }

    #[test]
    fn render_blockers_does_not_wrap_plan_paths_in_backticks() {
        let blockers = vec![Blocker {
            blocked: "P/alpha".into(),
            blocker: "Q/beta".into(),
            rationale: "reason.".into(),
        }];
        let out = render_blockers(&blockers);
        assert!(!out.contains('`'), "unexpected backtick in blockers: {out}");
        assert!(out.contains("P/alpha"));
        assert!(out.contains("Q/beta"));
    }

    #[test]
    fn render_recommendations_does_not_wrap_plan_path_in_backticks() {
        let recs = vec![Recommendation {
            plan: "P/alpha".into(),
            order: 1,
            rationale: "go.".into(),
        }];
        let out = render_recommendations(&recs);
        assert!(!out.contains('`'), "unexpected backtick in recommendations: {out}");
        assert!(out.contains("P/alpha"));
    }

    #[test]
    fn wrap_at_keeps_short_text_on_one_line() {
        let lines = wrap_at("one two three", 80);
        assert_eq!(lines, vec!["one two three".to_string()]);
    }

    #[test]
    fn wrap_at_breaks_at_word_boundary_within_width() {
        let lines = wrap_at("one two three four", 10);
        assert_eq!(lines, vec!["one two".to_string(), "three four".to_string()]);
    }

    #[test]
    fn wrap_at_normalises_internal_whitespace_including_newlines() {
        let lines = wrap_at("one\n\n  two\tthree", 80);
        assert_eq!(lines, vec!["one two three".to_string()]);
    }

    #[test]
    fn wrap_at_allows_oversized_word_to_stand_alone() {
        let lines = wrap_at("supercalifragilistic expialidocious", 10);
        assert_eq!(lines[0], "supercalifragilistic");
    }
}
