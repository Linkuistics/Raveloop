// src/survey/compose.rs
//
// Plan → bundle → prompt. Renders discovered plans as a single
// Markdown block and loads the survey prompt template.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::discover::PlanSnapshot;

/// Relative path to the survey prompt template inside a config dir.
pub const SURVEY_PROMPT_PATH: &str = "survey.md";

/// Render all discovered plans as a single Markdown block to append
/// after the survey prompt. Missing backlog/memory files are noted
/// explicitly rather than silently elided, so the LLM can distinguish
/// "empty" from "absent".
pub fn render_survey_input(plans: &[PlanSnapshot]) -> String {
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

/// Read the survey prompt template from `<config_root>/survey.md`.
pub fn load_survey_prompt(config_root: &Path) -> Result<String> {
    let path = config_root.join(SURVEY_PROMPT_PATH);
    fs::read_to_string(&path)
        .with_context(|| format!("Failed to read survey prompt at {}", path.display()))
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
