// src/survey/compose.rs
//
// Plan → bundle → prompt. Renders discovered plans as a single
// Markdown block and loads the survey prompt template.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::discover::PlanSnapshot;

/// Relative path to the cold survey prompt template inside a config dir.
pub const SURVEY_PROMPT_PATH: &str = "survey.md";

/// Relative path to the incremental (warm) survey prompt, used when
/// `run_survey` is called with a `--prior` file and has a non-empty
/// delta to analyse.
pub const SURVEY_INCREMENTAL_PROMPT_PATH: &str = "survey-incremental.md";

/// Render all discovered plans as a single Markdown block to append
/// after the survey prompt. Missing backlog/memory files are noted
/// explicitly rather than silently elided, so the LLM can distinguish
/// "empty" from "absent".
pub fn render_survey_input(plans: &[PlanSnapshot]) -> String {
    render_plan_blocks(plans.iter())
}

/// Render the incremental-mode LLM payload: the prior survey YAML
/// carried verbatim as context, followed by only the changed+added
/// plan bundles, followed by a short list of removed plan keys (so the
/// LLM can drop them from annotations).
///
/// The three sections are separated by labelled H2 headings so the
/// prompt can reference them directly. Keeping the full prior in view
/// is deliberate: the LLM must be able to revisit cross-plan blockers
/// and parallel streams when deltas affect plans that never changed
/// themselves.
pub fn render_survey_input_incremental(
    plans_to_analyse: &[&PlanSnapshot],
    prior_yaml: &str,
    removed_keys: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("## Prior survey (context)\n\n");
    out.push_str("The prior survey below is provided in full so you can revisit\n");
    out.push_str("cross-plan blockers, parallel streams, and the recommended\n");
    out.push_str("invocation order when deltas affect them. Plans whose rows\n");
    out.push_str("appear here but are NOT in \"Changed or added plans\" below\n");
    out.push_str("are unchanged — do not re-analyse them, but you may reference\n");
    out.push_str("them in the annotation sections.\n\n");
    out.push_str("```yaml\n");
    out.push_str(prior_yaml.trim_end());
    out.push_str("\n```\n\n");

    out.push_str("## Plans removed since prior\n\n");
    if removed_keys.is_empty() {
        out.push_str("(none)\n\n");
    } else {
        for key in removed_keys {
            out.push_str(&format!("- {key}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Changed or added plans\n\n");
    if plans_to_analyse.is_empty() {
        out.push_str("(none — annotations may still need regeneration \
                      because of removed plans)\n");
    } else {
        out.push_str(&render_plan_blocks(plans_to_analyse.iter().copied()));
    }
    out
}

fn render_plan_blocks<'a>(plans: impl Iterator<Item = &'a PlanSnapshot>) -> String {
    let mut out = String::new();
    for plan in plans {
        out.push_str(&format!(
            "\n## Plan: {}/{}\n\n### phase\n{}\n\n",
            plan.project, plan.plan, plan.phase
        ));
        match &plan.backlog {
            Some(b) => out.push_str(&format!("### backlog.md\n{b}\n\n")),
            None => out.push_str("### backlog.md\n(missing)\n\n"),
        }
        match &plan.memory {
            Some(m) => out.push_str(&format!("### memory.md\n{m}\n\n")),
            None => out.push_str("### memory.md\n(missing)\n\n"),
        }
        out.push_str("---\n");
    }
    out
}

/// Read the cold survey prompt template from `<config_root>/survey.md`.
pub fn load_survey_prompt(config_root: &Path) -> Result<String> {
    let path = config_root.join(SURVEY_PROMPT_PATH);
    fs::read_to_string(&path)
        .with_context(|| format!("Failed to read survey prompt at {}", path.display()))
}

/// Read the incremental survey prompt template from
/// `<config_root>/survey-incremental.md`.
pub fn load_survey_incremental_prompt(config_root: &Path) -> Result<String> {
    let path = config_root.join(SURVEY_INCREMENTAL_PROMPT_PATH);
    fs::read_to_string(&path).with_context(|| {
        format!("Failed to read incremental survey prompt at {}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn snap(
        project: &str,
        plan: &str,
        phase: &str,
        backlog: Option<&str>,
        memory: Option<&str>,
    ) -> PlanSnapshot {
        PlanSnapshot {
            project: project.into(),
            plan: plan.into(),
            phase: phase.into(),
            backlog: backlog.map(|s| s.into()),
            memory: memory.map(|s| s.into()),
            input_hash: String::new(),
            task_counts: None,
        }
    }

    #[test]
    fn render_survey_input_includes_project_and_plan_names() {
        let plans = vec![snap("Ravel", "sub-A", "work", Some("# backlog"), Some("# memory"))];
        let out = render_survey_input(&plans);
        assert!(out.contains("## Plan: Ravel/sub-A"));
        assert!(out.contains("### phase\nwork"));
        assert!(out.contains("### backlog.md\n# backlog"));
        assert!(out.contains("### memory.md\n# memory"));
    }

    #[test]
    fn render_survey_input_marks_missing_files_explicitly() {
        let plans = vec![snap("P", "x", "work", None, None)];
        let out = render_survey_input(&plans);
        assert!(out.contains("### backlog.md\n(missing)"));
        assert!(out.contains("### memory.md\n(missing)"));
    }

    #[test]
    fn render_survey_input_separates_plans_with_horizontal_rule() {
        let plans = vec![
            snap("P", "a", "work", None, None),
            snap("P", "b", "triage", None, None),
        ];
        let out = render_survey_input(&plans);
        assert_eq!(out.matches("\n---\n").count(), 2);
    }

    #[test]
    fn render_survey_input_incremental_embeds_prior_yaml_verbatim() {
        let plans_vec = [snap("P", "x", "work", Some("# b"), Some("# m"))];
        let refs: Vec<&PlanSnapshot> = plans_vec.iter().collect();
        let prior_yaml = "schema_version: 1\nplans:\n  - project: P\n    plan: x\n";
        let out = render_survey_input_incremental(&refs, prior_yaml, &[]);
        assert!(out.contains("## Prior survey (context)"));
        assert!(out.contains("```yaml\n") && out.contains("\n```"));
        assert!(out.contains(prior_yaml.trim_end()));
    }

    #[test]
    fn render_survey_input_incremental_lists_only_delta_plans() {
        let plans_vec = [
            snap("P", "changed", "work", Some("# b"), Some("# m")),
            snap("P", "added", "triage", None, None),
        ];
        let refs: Vec<&PlanSnapshot> = plans_vec.iter().collect();
        let out = render_survey_input_incremental(&refs, "plans: []", &[]);
        assert!(out.contains("## Plan: P/changed"));
        assert!(out.contains("## Plan: P/added"));
    }

    #[test]
    fn render_survey_input_incremental_surfaces_removed_keys() {
        let out = render_survey_input_incremental(
            &[],
            "plans: []",
            &["P/gone-a".to_string(), "P/gone-b".to_string()],
        );
        assert!(out.contains("## Plans removed since prior"));
        assert!(out.contains("- P/gone-a"));
        assert!(out.contains("- P/gone-b"));
    }

    #[test]
    fn render_survey_input_incremental_marks_empty_sections_explicitly() {
        let out = render_survey_input_incremental(&[], "plans: []", &[]);
        // Removed section shows "(none)" rather than the heading alone,
        // so the LLM cannot mistake a missing section for a missing
        // instruction.
        assert!(out.contains("## Plans removed since prior\n\n(none)"));
        assert!(out.contains("## Changed or added plans\n\n(none"));
    }

    #[test]
    fn load_survey_incremental_prompt_reads_from_config_root() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("survey-incremental.md"), "warm prompt").unwrap();
        assert_eq!(
            load_survey_incremental_prompt(tmp.path()).unwrap(),
            "warm prompt"
        );
    }

    #[test]
    fn load_survey_incremental_prompt_errors_when_missing() {
        let tmp = TempDir::new().unwrap();
        let err = load_survey_incremental_prompt(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("survey-incremental.md"));
    }

    #[test]
    fn load_survey_prompt_reads_from_config_root() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("survey.md"), "hello prompt").unwrap();
        assert_eq!(load_survey_prompt(tmp.path()).unwrap(), "hello prompt");
    }

    #[test]
    fn load_survey_prompt_errors_when_missing() {
        let tmp = TempDir::new().unwrap();
        let err = load_survey_prompt(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("survey.md"));
    }
}
