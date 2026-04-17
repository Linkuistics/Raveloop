// src/survey.rs
//
// Multi-project plan status survey. Gathers `phase.md`, `backlog.md`,
// and `memory.md` from every plan directory under one or more roots,
// renders them as a single prompt, and shells out to a headless
// `claude` session for LLM-driven summarisation and prioritisation.
//
// The command is intentionally single-shot and read-only: no tool use,
// no file writes, no session persistence. Fresh context per invocation
// by construction.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};
use crate::git::find_project_root;

/// Fallback model when neither `--model` nor `models.survey` is
/// configured. A cheap, fast model is appropriate: survey is a
/// summarisation task over plain-text inputs.
pub const DEFAULT_SURVEY_MODEL: &str = "claude-haiku-4-5";

/// Relative path to the survey prompt template inside a config dir.
pub const SURVEY_PROMPT_PATH: &str = "survey.md";

/// A single plan's state, bundled for inclusion in the survey prompt.
#[derive(Debug)]
pub struct PlanSnapshot {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub backlog: Option<String>,
    pub memory: Option<String>,
}

/// Derive the project name for a plan by walking up from the plan's
/// own directory to the nearest ancestor containing `.git`, then
/// taking that ancestor's basename. Hard errors if no `.git` is found
/// above the plan — plans outside a git repo are unsupported.
fn project_name_for_plan(plan_path: &Path) -> Result<String> {
    let git_root = find_project_root(plan_path)?;
    Path::new(&git_root)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .with_context(|| format!("Could not derive project name from git root {git_root}"))
}

/// Walk `root` looking for plan directories. A directory is a plan iff
/// it contains a `phase.md` file; this matches the convention used
/// everywhere else in Raveloop. For each plan, the project name is the
/// basename of the nearest ancestor containing `.git` — not the root
/// basename — so plans from different repos under the same `--root`
/// are labelled correctly. Returned plans are sorted by plan name for
/// deterministic output.
pub fn discover_plans(root: &Path) -> Result<Vec<PlanSnapshot>> {
    let mut plans = Vec::new();

    let entries = fs::read_dir(root)
        .with_context(|| format!("Failed to read plan root {}", root.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let phase_file = path.join("phase.md");
        if !phase_file.exists() {
            continue;
        }

        let plan = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let project = project_name_for_plan(&path)?;
        let phase = fs::read_to_string(&phase_file)
            .with_context(|| format!("Failed to read {}", phase_file.display()))?
            .trim()
            .to_string();
        let backlog = fs::read_to_string(path.join("backlog.md")).ok();
        let memory = fs::read_to_string(path.join("memory.md")).ok();

        plans.push(PlanSnapshot {
            project,
            plan,
            phase,
            backlog,
            memory,
        });
    }

    plans.sort_by(|a, b| a.plan.cmp(&b.plan));
    Ok(plans)
}

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

// -------- Structured response from the LLM --------

/// Typed deserialisation target for the YAML document the LLM emits.
/// The LLM does classification and reasoning; the tool owns rendering.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct SurveyResponse {
    pub plans: Vec<PlanRow>,
    #[serde(default)]
    pub cross_project_blockers: Vec<Blocker>,
    #[serde(default)]
    pub recommended_invocation_order: Vec<Recommendation>,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct PlanRow {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub unblocked: usize,
    pub blocked: usize,
    pub done: usize,
    pub received: usize,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct Blocker {
    pub blocked: String,
    pub blocker: String,
    pub rationale: String,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub struct Recommendation {
    pub plan: String,
    pub rationale: String,
}

/// Parse the LLM's stdout as a YAML survey response. Tolerates an
/// optional ```yaml / ``` code fence that some models emit despite
/// instructions otherwise.
pub fn parse_survey_response(stdout: &str) -> Result<SurveyResponse> {
    let content = strip_code_fence(stdout);
    serde_yaml::from_str(content).with_context(|| {
        format!(
            "Failed to parse survey response as YAML. Raw output from claude:\n---\n{stdout}\n---"
        )
    })
}

/// Strip a leading ```yaml or ``` fence and matching trailing fence,
/// if present. Otherwise returns the input trimmed of outer whitespace.
fn strip_code_fence(s: &str) -> &str {
    let trimmed = s.trim();
    let body = trimmed
        .strip_prefix("```yaml")
        .or_else(|| trimmed.strip_prefix("```yml"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let body = body.trim_start_matches('\n').trim_end();
    body.strip_suffix("```").unwrap_or(body).trim_end()
}

// -------- Rendering --------

/// Column width target for wrapped prose sections. Chosen to fit a
/// standard 80-column terminal with a small margin.
const WRAP_WIDTH: usize = 78;

/// Render the complete survey output: top heading, the three
/// sections, each with its own renderer. Deterministic and unit-tested.
pub fn render_survey_output(response: &SurveyResponse) -> String {
    let mut out = String::new();
    out.push_str("# Plan Status Survey\n\n");

    out.push_str("## Per-plan summary\n\n");
    out.push_str(&render_plan_table(&response.plans));
    out.push('\n');

    out.push_str("## Cross-project blockers\n\n");
    out.push_str(&render_blockers(&response.cross_project_blockers));
    out.push('\n');

    out.push_str("## Recommended invocation order\n\n");
    out.push_str(&render_recommendations(&response.recommended_invocation_order));

    out
}

/// Render a space-padded, monospace-aligned table of plans. Each
/// column is padded to `max(header, longest value) + 2` so values
/// align regardless of string length. The final NOTES column is not
/// right-padded — trailing whitespace adds no value.
fn render_plan_table(plans: &[PlanRow]) -> String {
    const HEADERS: [&str; 8] = [
        "PROJECT", "PLAN", "PHASE", "UNBLOCKED", "BLOCKED", "DONE", "RECEIVED", "NOTES",
    ];
    const LAST: usize = HEADERS.len() - 1;

    let rows: Vec<[String; 8]> = plans
        .iter()
        .map(|p| {
            [
                p.project.clone(),
                p.plan.clone(),
                p.phase.clone(),
                p.unblocked.to_string(),
                p.blocked.to_string(),
                p.done.to_string(),
                p.received.to_string(),
                p.notes.clone(),
            ]
        })
        .collect();

    let mut widths = [0usize; 8];
    for (i, h) in HEADERS.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    let mut out = String::new();
    append_table_row(&mut out, &HEADERS.map(String::from), &widths, LAST);
    for row in &rows {
        append_table_row(&mut out, row, &widths, LAST);
    }
    out
}

fn append_table_row(out: &mut String, row: &[String; 8], widths: &[usize; 8], last: usize) {
    for (i, cell) in row.iter().enumerate() {
        if i == last {
            out.push_str(cell);
        } else {
            out.push_str(&format!("{:<width$}  ", cell, width = widths[i]));
        }
    }
    out.push('\n');
}

/// Render the cross-project blockers section as indented bullets with
/// a blank line between entries. Each bullet wraps at WRAP_WIDTH with
/// continuation lines indented to align under the bullet's content.
fn render_blockers(blockers: &[Blocker]) -> String {
    if blockers.is_empty() {
        return "  None detected.\n".to_string();
    }
    let mut out = String::new();
    for (i, b) in blockers.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let text = format!("`{}` blocked on `{}`: {}", b.blocked, b.blocker, b.rationale);
        out.push_str(&render_wrapped_bullet("  - ", &text));
        out.push('\n');
    }
    out
}

/// Render recommended invocations as indented numbered items with
/// blank lines between them.
fn render_recommendations(recs: &[Recommendation]) -> String {
    if recs.is_empty() {
        return "  None available.\n".to_string();
    }
    let mut out = String::new();
    for (i, r) in recs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let prefix = format!("  {}. ", i + 1);
        let text = format!("`{}` — {}", r.plan, r.rationale);
        out.push_str(&render_wrapped_bullet(&prefix, &text));
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

/// Resolve which model to use for the survey call. Precedence:
///   1. explicit `--model` flag on the CLI
///   2. `models.survey` in the agent's config
///   3. `DEFAULT_SURVEY_MODEL` constant
fn resolve_model(
    agent_config: &crate::types::AgentConfig,
    flag_override: Option<String>,
) -> String {
    flag_override
        .or_else(|| agent_config.models.get("survey").cloned())
        .unwrap_or_else(|| DEFAULT_SURVEY_MODEL.to_string())
}

/// End-to-end survey runner. Gathers plans across every `--root`,
/// composes the prompt, invokes the `claude` CLI headlessly, and
/// prints the LLM's response to stdout.
pub async fn run_survey(
    config_root: &Path,
    roots: &[PathBuf],
    model_override: Option<String>,
) -> Result<()> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "survey currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = resolve_model(&agent_config, model_override);

    let mut all_plans = Vec::new();
    for root in roots {
        if !root.is_dir() {
            anyhow::bail!(
                "Plan root {} does not exist or is not a directory.",
                root.display()
            );
        }
        let plans = discover_plans(root)?;
        if plans.is_empty() {
            eprintln!(
                "warning: plan root {} contained no plan directories (no phase.md found)",
                root.display()
            );
        }
        all_plans.extend(plans);
    }
    if all_plans.is_empty() {
        anyhow::bail!("No plans discovered in any of the supplied --root directories.");
    }

    let survey_prompt = load_survey_prompt(config_root)?;
    let plan_input = render_survey_input(&all_plans);
    let full_prompt = format!("{survey_prompt}\n\n---\n{plan_input}");

    eprintln!(
        "Surveying {} plan(s) across {} root(s) using model {}...",
        all_plans.len(),
        roots.len(),
        model
    );

    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(&full_prompt)
        .arg("--model")
        .arg(&model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn 'claude' CLI. Ensure it is installed and on PATH.")?;

    let mut stdout = child
        .stdout
        .take()
        .context("claude CLI stdout pipe was unexpectedly unavailable")?;
    let mut output = String::new();
    stdout.read_to_string(&mut output).await?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude CLI exited with status {status}");
    }

    let response = parse_survey_response(&output)?;
    print!("{}", render_survey_output(&response));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use crate::types::AgentConfig;

    fn write_plan(root: &Path, name: &str, phase: &str, backlog: Option<&str>, memory: Option<&str>) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("phase.md"), phase).unwrap();
        if let Some(b) = backlog {
            fs::write(dir.join("backlog.md"), b).unwrap();
        }
        if let Some(m) = memory {
            fs::write(dir.join("memory.md"), m).unwrap();
        }
    }

    /// Create a fake git project at `project_dir` with an empty `.git`
    /// directory — `find_project_root` only checks for `.git`'s
    /// existence, not its validity, so this is sufficient for tests.
    fn mark_as_git_project(project_dir: &Path) {
        fs::create_dir_all(project_dir.join(".git")).unwrap();
    }

    fn empty_agent_config(models: &[(&str, &str)]) -> AgentConfig {
        let mut m = HashMap::new();
        for (k, v) in models {
            m.insert(k.to_string(), v.to_string());
        }
        AgentConfig {
            models: m,
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: None,
        }
    }

    #[test]
    fn discover_plans_finds_directories_with_phase_md() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("my-project");
        let root = project.join("LLM_STATE");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "work\n", Some("# backlog a\n"), Some("# memory a\n"));
        write_plan(&root, "plan-b", "triage\n", Some("# backlog b\n"), None);
        // A directory WITHOUT phase.md is ignored.
        fs::create_dir_all(root.join("not-a-plan")).unwrap();
        fs::write(root.join("not-a-plan").join("backlog.md"), "noise").unwrap();

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].plan, "plan-a");
        assert_eq!(plans[1].plan, "plan-b");
    }

    #[test]
    fn discover_plans_derives_project_from_ancestor_git_dir() {
        // Project layout:
        //   tmp/my-project/.git          <- project marker
        //   tmp/my-project/LLM_STATE/    <- the --root argument
        //   tmp/my-project/LLM_STATE/plan-x/phase.md
        // The project name should be "my-project", NOT "LLM_STATE".
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("my-project");
        let root = project.join("LLM_STATE");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-x", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].project, "my-project");
    }

    #[test]
    fn discover_plans_errors_when_no_git_above_plan() {
        // Tempdir has no `.git` anywhere above the plan → hard error.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("rogue-state");
        fs::create_dir_all(&root).unwrap();
        write_plan(&root, "plan-x", "work\n", None, None);

        let err = discover_plans(&root).unwrap_err();
        assert!(format!("{err:#}").contains("No .git found"));
    }

    #[test]
    fn discover_plans_trims_phase_whitespace() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "  \n work \n\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans[0].phase, "work");
    }

    #[test]
    fn discover_plans_records_missing_backlog_and_memory_as_none() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert!(plans[0].backlog.is_none());
        assert!(plans[0].memory.is_none());
    }

    #[test]
    fn discover_plans_returns_sorted_by_plan_name() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "zeta", "work\n", None, None);
        write_plan(&root, "alpha", "work\n", None, None);
        write_plan(&root, "mu", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        let names: Vec<_> = plans.iter().map(|p| p.plan.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn discover_plans_errors_when_root_unreadable() {
        let missing = PathBuf::from("/definitely/not/a/path/for/survey/test");
        assert!(discover_plans(&missing).is_err());
    }

    #[test]
    fn render_survey_input_includes_project_and_plan_names() {
        let plans = vec![PlanSnapshot {
            project: "Mnemosyne".into(),
            plan: "sub-A".into(),
            phase: "work".into(),
            backlog: Some("# backlog".into()),
            memory: Some("# memory".into()),
        }];
        let out = render_survey_input(&plans);
        assert!(out.contains("## Plan: Mnemosyne/sub-A"));
        assert!(out.contains("### phase\nwork"));
        assert!(out.contains("### backlog.md\n# backlog"));
        assert!(out.contains("### memory.md\n# memory"));
    }

    #[test]
    fn render_survey_input_marks_missing_files_explicitly() {
        let plans = vec![PlanSnapshot {
            project: "P".into(),
            plan: "x".into(),
            phase: "work".into(),
            backlog: None,
            memory: None,
        }];
        let out = render_survey_input(&plans);
        assert!(out.contains("### backlog.md\n(missing)"));
        assert!(out.contains("### memory.md\n(missing)"));
    }

    #[test]
    fn render_survey_input_separates_plans_with_horizontal_rule() {
        let plans = vec![
            PlanSnapshot {
                project: "P".into(),
                plan: "a".into(),
                phase: "work".into(),
                backlog: None,
                memory: None,
            },
            PlanSnapshot {
                project: "P".into(),
                plan: "b".into(),
                phase: "triage".into(),
                backlog: None,
                memory: None,
            },
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

    #[test]
    fn resolve_model_prefers_cli_flag() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, Some("flag-model".into()));
        assert_eq!(resolved, "flag-model");
    }

    #[test]
    fn resolve_model_falls_back_to_agent_config_survey_key() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, "configured-model");
    }

    #[test]
    fn resolve_model_uses_default_when_nothing_configured() {
        let cfg = empty_agent_config(&[]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, DEFAULT_SURVEY_MODEL);
    }

    // ---- parse_survey_response ----

    fn sample_yaml() -> &'static str {
        r#"
plans:
  - project: Mnemosyne
    plan: sub-A-global-store
    phase: work
    unblocked: 1
    blocked: 15
    done: 0
    received: 0
    notes: Task 0 gate unblocked

cross_project_blockers:
  - blocked: Mnemosyne/sub-F-hierarchy
    blocker: Mnemosyne/sub-B-phase-cycle
    rationale: |
      Sub-B's downstream task list must be rewritten before
      Sub-F's Task 0 readiness gate can fire.

recommended_invocation_order:
  - plan: Mnemosyne/mnemosyne-orchestrator
    rationale: Dispatch Sub-C work-phase cycle.
"#
    }

    #[test]
    fn parse_survey_response_parses_valid_yaml() {
        let resp = parse_survey_response(sample_yaml()).unwrap();
        assert_eq!(resp.plans.len(), 1);
        assert_eq!(resp.plans[0].plan, "sub-A-global-store");
        assert_eq!(resp.plans[0].unblocked, 1);
        assert_eq!(resp.cross_project_blockers.len(), 1);
        assert!(resp.cross_project_blockers[0].rationale.contains("readiness gate"));
        assert_eq!(resp.recommended_invocation_order.len(), 1);
    }

    #[test]
    fn parse_survey_response_strips_yaml_code_fence() {
        let wrapped = format!("```yaml\n{}\n```\n", sample_yaml().trim());
        let resp = parse_survey_response(&wrapped).unwrap();
        assert_eq!(resp.plans.len(), 1);
    }

    #[test]
    fn parse_survey_response_strips_plain_code_fence() {
        let wrapped = format!("```\n{}\n```\n", sample_yaml().trim());
        let resp = parse_survey_response(&wrapped).unwrap();
        assert_eq!(resp.plans.len(), 1);
    }

    #[test]
    fn parse_survey_response_errors_include_raw_output_for_debugging() {
        let malformed = "not: valid: yaml: at: all:\n  - [";
        let err = parse_survey_response(malformed).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Raw output from claude"));
    }

    #[test]
    fn parse_survey_response_allows_missing_optional_sections() {
        let minimal = r#"
plans:
  - project: P
    plan: x
    phase: work
    unblocked: 0
    blocked: 0
    done: 0
    received: 0
"#;
        let resp = parse_survey_response(minimal).unwrap();
        assert!(resp.cross_project_blockers.is_empty());
        assert!(resp.recommended_invocation_order.is_empty());
        assert_eq!(resp.plans[0].notes, "");
    }

    // ---- rendering ----

    fn row(project: &str, plan: &str, phase: &str, u: usize, b: usize, d: usize, r: usize, notes: &str) -> PlanRow {
        PlanRow {
            project: project.into(),
            plan: plan.into(),
            phase: phase.into(),
            unblocked: u,
            blocked: b,
            done: d,
            received: r,
            notes: notes.into(),
        }
    }

    #[test]
    fn render_plan_table_aligns_columns_across_rows() {
        let plans = vec![
            row("P", "short", "work", 1, 2, 3, 0, "note one"),
            row("P", "a-much-longer-plan-name", "triage", 10, 20, 30, 4, "note two"),
        ];
        let table = render_plan_table(&plans);
        let lines: Vec<&str> = table.lines().collect();
        // Header + two data rows
        assert_eq!(lines.len(), 3);
        // Position of "PLAN" header in first line equals position of
        // "short" in second line (both columns start at the same index).
        let plan_header_col = lines[0].find("PLAN").unwrap();
        let plan_row0_col = lines[1].find("short").unwrap();
        let plan_row1_col = lines[2].find("a-much-longer-plan-name").unwrap();
        assert_eq!(plan_header_col, plan_row0_col);
        assert_eq!(plan_header_col, plan_row1_col);
        // Every data column start aligns with its header. Use rfind so
        // "BLOCKED" picks the column header rather than the substring
        // inside "UNBLOCKED".
        for header in ["PHASE", "UNBLOCKED", "BLOCKED", "DONE", "RECEIVED"] {
            let header_col = lines[0].rfind(header).unwrap();
            assert!(
                lines[1].chars().nth(header_col).map(|c| c != ' ').unwrap_or(false),
                "column {header} misaligned on row 0; line: {:?}",
                lines[1]
            );
            assert!(
                lines[2].chars().nth(header_col).map(|c| c != ' ').unwrap_or(false),
                "column {header} misaligned on row 1; line: {:?}",
                lines[2]
            );
        }
    }

    #[test]
    fn render_plan_table_has_header_row_in_all_caps() {
        let plans = vec![row("P", "x", "work", 0, 0, 0, 0, "")];
        let table = render_plan_table(&plans);
        let first = table.lines().next().unwrap();
        assert!(first.contains("PROJECT"));
        assert!(first.contains("UNBLOCKED"));
        assert!(first.contains("NOTES"));
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
        // Blank line between the two bullet blocks.
        assert!(out.contains("\n\n  - `P/c`"), "missing blank line separator: {out}");
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
    fn render_blockers_continuation_lines_indent_to_content_start() {
        let blockers = vec![Blocker {
            blocked: "P/a".into(),
            blocker: "Q/b".into(),
            rationale: "word ".repeat(40),
        }];
        let out = render_blockers(&blockers);
        let lines: Vec<&str> = out.lines().collect();
        // First line starts with "  - "
        assert!(lines[0].starts_with("  - "));
        // Subsequent non-empty lines start with four spaces (alignment
        // under the "-" bullet's content, not under the "-" itself).
        for line in &lines[1..] {
            if !line.is_empty() {
                assert!(line.starts_with("    "), "continuation not indented: {line:?}");
            }
        }
    }

    #[test]
    fn render_recommendations_numbers_entries_in_order() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), rationale: "first.".into() },
            Recommendation { plan: "P/b".into(), rationale: "second.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("  1. "));
        assert!(out.contains("  2. "));
        let one_idx = out.find("  1. ").unwrap();
        let two_idx = out.find("  2. ").unwrap();
        assert!(one_idx < two_idx);
    }

    #[test]
    fn render_recommendations_separates_entries_with_blank_lines() {
        let recs = vec![
            Recommendation { plan: "P/a".into(), rationale: "first.".into() },
            Recommendation { plan: "P/b".into(), rationale: "second.".into() },
        ];
        let out = render_recommendations(&recs);
        assert!(out.contains("\n\n  2. "));
    }

    #[test]
    fn render_survey_output_contains_all_three_sections() {
        let response = SurveyResponse {
            plans: vec![row("P", "x", "work", 1, 0, 0, 0, "")],
            cross_project_blockers: vec![],
            recommended_invocation_order: vec![Recommendation {
                plan: "P/x".into(),
                rationale: "do it.".into(),
            }],
        };
        let out = render_survey_output(&response);
        assert!(out.contains("# Plan Status Survey"));
        assert!(out.contains("## Per-plan summary"));
        assert!(out.contains("## Cross-project blockers"));
        assert!(out.contains("## Recommended invocation order"));
        assert!(out.contains("None detected."));
    }

    // ---- wrap_at ----

    #[test]
    fn wrap_at_keeps_short_text_on_one_line() {
        let lines = wrap_at("one two three", 80);
        assert_eq!(lines, vec!["one two three".to_string()]);
    }

    #[test]
    fn wrap_at_breaks_at_word_boundary_within_width() {
        let lines = wrap_at("one two three four", 10);
        // "one two" = 7, adding " three" → 13 > 10; wrap.
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
        // The first word exceeds width but stands alone rather than being broken.
        assert_eq!(lines[0], "supercalifragilistic");
    }
}
