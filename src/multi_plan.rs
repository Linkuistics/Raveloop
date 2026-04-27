// src/multi_plan.rs
//
// Multi-plan run mode: `ravel-lite run` with N > 1 plan directories.
// The loop is survey → select → dispatch one phase cycle → repeat.
// Selection is code-driven (parsing `recommended_invocation_order`
// from the survey YAML), with a plain-stdout numbered prompt and a
// single stdin read — no ratatui widget. Each dispatched cycle
// brings the TUI up, runs one full `phase_loop` pass (work through
// git-commit-triage), and tears the TUI down before the next
// selection.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::agent::claude_code::ClaudeCodeAgent;
use crate::agent::pi::PiAgent;
use crate::agent::Agent;
use crate::config::{load_agent_config, load_shared_config};
use crate::git::project_root_for_plan;
use crate::phase_loop;
use crate::related_components::read_related_plans_markdown;
use crate::state::filenames::PHASE_FILENAME;
use crate::survey::{
    compute_survey_response, emit_survey_yaml, load_plan, plan_key, SurveyResponse,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{run_tui, UI};

/// A plan option presented to the user for selection. Produced from
/// `recommended_invocation_order` when the LLM supplied one, or from
/// the full discovered plan set as a sorted fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOption {
    /// `project/plan` key — matches both `plan_key(...)` and the
    /// `recommendation.plan` string in the survey YAML.
    pub key: String,
    /// Short rationale shown next to the ordinal. Empty for fallback
    /// options (no LLM commentary available).
    pub rationale: String,
}

/// Validate and build the plan-directory lookup table that
/// `select_plan_from_response` needs. Every plan dir must contain
/// `phase.md`, and `load_plan` must succeed so the project/plan
/// derivation matches the survey's discovery keys exactly.
pub fn build_plan_dir_map(plan_dirs: &[PathBuf]) -> Result<HashMap<String, PathBuf>> {
    let mut map = HashMap::with_capacity(plan_dirs.len());
    for plan_dir in plan_dirs {
        if !plan_dir.join(PHASE_FILENAME).exists() {
            anyhow::bail!(
                "{}/{PHASE_FILENAME} not found. Is this a valid plan directory?",
                plan_dir.display()
            );
        }
        let snapshot = load_plan(plan_dir)
            .with_context(|| format!("Failed to load plan at {}", plan_dir.display()))?;
        let key = plan_key(&snapshot.project, &snapshot.plan);
        if let Some(existing) = map.insert(key.clone(), plan_dir.clone()) {
            anyhow::bail!(
                "two plan directories resolve to the same project/plan key '{key}': \
                 {} and {}. Rename or move one so each plan has a unique identifier.",
                existing.display(),
                plan_dir.display(),
            );
        }
    }
    Ok(map)
}

/// Turn a `SurveyResponse` into the ordered list of selection options
/// the user sees. `recommended_invocation_order` is the happy path;
/// when it is empty (LLM omitted the section, or a carried-forward
/// prior had no recommendations), fall back to every discovered plan
/// in alphabetical order so the user can still pick something.
///
/// Any recommendation naming a plan not in `plan_dir_by_key` is a hard
/// error — an LLM-drift signal the user should see directly rather
/// than a silent skip, per the "surface drift to the user" memory
/// from 5b.
pub fn options_from_response(
    response: &SurveyResponse,
    plan_dir_by_key: &HashMap<String, PathBuf>,
) -> Result<Vec<SelectionOption>> {
    if response.recommended_invocation_order.is_empty() {
        let mut keys: Vec<&String> = plan_dir_by_key.keys().collect();
        keys.sort();
        return Ok(keys
            .into_iter()
            .map(|k| SelectionOption {
                key: k.clone(),
                rationale: String::new(),
            })
            .collect());
    }

    let mut options = Vec::with_capacity(response.recommended_invocation_order.len());
    for rec in &response.recommended_invocation_order {
        if !plan_dir_by_key.contains_key(&rec.plan) {
            anyhow::bail!(
                "survey recommended plan '{}' but no such plan directory was supplied \
                 on the command line. This is an LLM-drift signal — re-run the survey \
                 (delete the --survey-state file to force a cold start) and, if it \
                 recurs, check the prompt or the plan_dirs arguments.",
                rec.plan,
            );
        }
        options.push(SelectionOption {
            key: rec.plan.clone(),
            rationale: rec.rationale.clone(),
        });
    }
    Ok(options)
}

/// Maximum number of invalid numeric inputs accepted before the
/// selection loop gives up and returns an error. Prevents a broken
/// stdin (e.g. a closed pipe in CI) from spinning forever.
const MAX_INVALID_SELECTION_ATTEMPTS: usize = 3;

/// Render the selection prompt, read a numeric choice from `input`, and
/// return the plan directory the user picked. Pure function over
/// `(SurveyResponse, plan_dir_by_key, output, input)` — no
/// stdin/stdout dependency in the signature so tests can drive it
/// with in-memory buffers.
///
/// Entering `0`, `q`, or `quit` signals "exit the multi-plan run"
/// and returns `Ok(None)`. Valid selections return `Ok(Some(path))`.
pub fn select_plan_from_response<R: BufRead, W: Write>(
    response: &SurveyResponse,
    plan_dir_by_key: &HashMap<String, PathBuf>,
    output: &mut W,
    input: &mut R,
) -> Result<Option<PathBuf>> {
    let options = options_from_response(response, plan_dir_by_key)?;
    if options.is_empty() {
        anyhow::bail!(
            "no plans available for selection — `plan_dir_by_key` is empty. \
             This is a programming error in the multi-plan runner."
        );
    }

    writeln!(output, "\nSelect next plan to dispatch:")?;
    for (idx, opt) in options.iter().enumerate() {
        let ordinal = idx + 1;
        if opt.rationale.is_empty() {
            writeln!(output, "  {ordinal}. {}", opt.key)?;
        } else {
            writeln!(output, "  {ordinal}. {}  —  {}", opt.key, opt.rationale)?;
        }
    }
    writeln!(output, "  0. exit")?;

    let mut attempts = 0;
    loop {
        write!(output, "\nEnter choice (1-{} or 0 to exit): ", options.len())?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input.read_line(&mut line)?;
        if bytes == 0 {
            // EOF — treat like "exit" so a closed stdin doesn't hang.
            return Ok(None);
        }

        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("q") || trimmed.eq_ignore_ascii_case("quit") {
            return Ok(None);
        }

        match trimmed.parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(n) if n >= 1 && n <= options.len() => {
                let key = &options[n - 1].key;
                return Ok(Some(
                    plan_dir_by_key
                        .get(key)
                        .cloned()
                        .expect("options were built from plan_dir_by_key and kept valid keys"),
                ));
            }
            _ => {
                attempts += 1;
                if attempts >= MAX_INVALID_SELECTION_ATTEMPTS {
                    anyhow::bail!(
                        "{MAX_INVALID_SELECTION_ATTEMPTS} invalid selection attempts; aborting.",
                    );
                }
                writeln!(
                    output,
                    "  invalid input '{trimmed}' — enter a number between 1 and {} \
                     (or 0 to exit).",
                    options.len()
                )?;
            }
        }
    }
}

/// Force `dangerous: true` for every known LLM phase. Duplicates the
/// helper in `main.rs`; accepting one tiny duplication is cheaper than
/// plumbing the helper across module boundaries.
fn force_dangerous(config: &mut AgentConfig) {
    let phases = [
        LlmPhase::Work,
        LlmPhase::AnalyseWork,
        LlmPhase::Reflect,
        LlmPhase::Dream,
        LlmPhase::Triage,
    ];
    for phase in phases {
        let params = config.params.entry(phase.as_str().to_string()).or_default();
        params.insert("dangerous".to_string(), serde_yaml::Value::Bool(true));
    }
}

/// Run exactly one full phase cycle (work → ... → git-commit-triage)
/// for the selected plan. Handles TUI setup/teardown per cycle so the
/// plain-stdout survey/selection prompt renders cleanly between
/// dispatches.
async fn dispatch_one_cycle(
    config_root: &Path,
    plan_dir: &Path,
    dangerous: bool,
) -> Result<()> {
    let shared_config = load_shared_config(config_root)?;
    let mut agent_config = load_agent_config(config_root, &shared_config.agent)?;
    let project_dir = project_root_for_plan(plan_dir)?;

    if dangerous {
        if shared_config.agent == "claude-code" {
            force_dangerous(&mut agent_config);
        } else {
            eprintln!(
                "warning: --dangerous has no effect for agent '{}' (claude-code only)",
                shared_config.agent
            );
        }
    }

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: project_dir.clone(),
        dev_root: Path::new(&project_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        related_plans: read_related_plans_markdown(plan_dir),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let agent: Arc<dyn Agent> = match shared_config.agent.as_str() {
        "claude-code" => Arc::new(ClaudeCodeAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        "pi" => Arc::new(PiAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        other => anyhow::bail!("Unknown agent: {other}"),
    };

    let (tx, rx) = mpsc::unbounded_channel();
    let ui = UI::new(tx);
    let tui_handle = tokio::spawn(run_tui(rx));

    let result = phase_loop::phase_loop(agent, &ctx, &shared_config, &ui).await;

    if let Err(ref e) = result {
        ui.log("");
        ui.log(&format!("  ✗  Fatal error: {e:#}"));
        let _ = ui.confirm("Continue?").await;
    }
    ui.quit();
    tui_handle.await??;

    if let Err(ref e) = result {
        eprintln!("\nravel-lite (plan cycle) exited with error:\n{e:#}");
    }

    result
}

/// Orchestrate the multi-plan run loop: survey → present top-ranked
/// plans → read selection → dispatch one cycle → repeat. Exits cleanly
/// when the user chooses `0`/`q`/`quit`, or when stdin reaches EOF.
///
/// `survey_state_path` is both output (written at the end of every
/// survey) and input (passed as `--prior` to the next survey when it
/// already exists). This single-file round-trip is 5b's
/// incremental-survey integration point for multi-plan mode.
pub async fn run_multi_plan(
    config_root: &Path,
    plan_dirs: &[PathBuf],
    survey_state_path: &Path,
    dangerous: bool,
) -> Result<()> {
    if plan_dirs.len() < 2 {
        anyhow::bail!(
            "run_multi_plan requires at least 2 plan directories; got {}",
            plan_dirs.len()
        );
    }

    let plan_dir_by_key = build_plan_dir_map(plan_dirs)?;

    loop {
        let prior: Option<&Path> = if survey_state_path.exists() {
            Some(survey_state_path)
        } else {
            None
        };

        let response =
            compute_survey_response(config_root, plan_dirs, None, None, prior, false).await?;

        let yaml = emit_survey_yaml(&response)?;
        fs::write(survey_state_path, &yaml).with_context(|| {
            format!(
                "Failed to write --survey-state at {}",
                survey_state_path.display()
            )
        })?;

        let stdout = std::io::stdout();
        let stdin = std::io::stdin();
        let mut out = stdout.lock();
        let mut inp = stdin.lock();
        let selected = select_plan_from_response(&response, &plan_dir_by_key, &mut out, &mut inp)?;
        drop(out);
        drop(inp);

        let selected_dir = match selected {
            Some(p) => p,
            None => return Ok(()),
        };

        dispatch_one_cycle(config_root, &selected_dir, dangerous).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::survey::{PlanRow, SurveyResponse};
    use std::io::Cursor;

    fn make_response_with_order(order: &[(&str, &str)]) -> SurveyResponse {
        use crate::survey::parse_survey_response;
        // Build a minimal response with the given recommendation order.
        let mut yaml = String::from("plans: []\nrecommended_invocation_order:\n");
        for (plan, rationale) in order {
            yaml.push_str(&format!(
                "  - plan: {plan}\n    order: 1\n    rationale: \"{rationale}\"\n"
            ));
        }
        parse_survey_response(&yaml).unwrap()
    }

    fn make_empty_response() -> SurveyResponse {
        SurveyResponse {
            schema_version: 1,
            plans: vec![PlanRow {
                project: "Proj".into(),
                plan: "a".into(),
                phase: "work".into(),
                unblocked: 0,
                blocked: 0,
                done: 0,
                received: 0,
                notes: String::new(),
                input_hash: String::new(),
                task_counts: None,
            }],
            cross_plan_blockers: vec![],
            parallel_streams: vec![],
            recommended_invocation_order: vec![],
        }
    }

    fn plan_map(entries: &[(&str, &str)]) -> HashMap<String, PathBuf> {
        entries
            .iter()
            .map(|(key, path)| ((*key).to_string(), PathBuf::from(*path)))
            .collect()
    }

    #[test]
    fn options_from_response_uses_recommendation_order_when_present() {
        let response = make_response_with_order(&[
            ("Proj/b", "Top priority"),
            ("Proj/a", "Secondary"),
        ]);
        let map = plan_map(&[("Proj/a", "/tmp/a"), ("Proj/b", "/tmp/b")]);
        let options = options_from_response(&response, &map).unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].key, "Proj/b");
        assert_eq!(options[0].rationale, "Top priority");
        assert_eq!(options[1].key, "Proj/a");
    }

    #[test]
    fn options_from_response_falls_back_to_all_plans_sorted() {
        let response = make_empty_response();
        let map = plan_map(&[("Proj/z", "/tmp/z"), ("Proj/a", "/tmp/a"), ("Proj/m", "/tmp/m")]);
        let options = options_from_response(&response, &map).unwrap();
        assert_eq!(
            options.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
            vec!["Proj/a", "Proj/m", "Proj/z"],
            "fallback must sort alphabetically for deterministic output"
        );
        for opt in &options {
            assert!(opt.rationale.is_empty(), "fallback has no rationales");
        }
    }

    #[test]
    fn options_from_response_errors_on_hallucinated_plan() {
        let response = make_response_with_order(&[("Proj/ghost", "LLM made this up")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let err = options_from_response(&response, &map).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Proj/ghost"), "error should name the offender: {msg}");
        assert!(msg.contains("LLM-drift"), "error should flag the drift to the user: {msg}");
    }

    #[test]
    fn select_returns_the_chosen_plan_dir() {
        let response = make_response_with_order(&[
            ("Proj/b", "Top"),
            ("Proj/a", "Second"),
        ]);
        let map = plan_map(&[("Proj/a", "/tmp/a"), ("Proj/b", "/tmp/b")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("2\n");
        let picked =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap();
        assert_eq!(picked, Some(PathBuf::from("/tmp/a")));

        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("1. Proj/b"), "prompt should list ordinal 1: {rendered}");
        assert!(rendered.contains("2. Proj/a"), "prompt should list ordinal 2: {rendered}");
        assert!(rendered.contains("Top"), "prompt should include rationale: {rendered}");
    }

    #[test]
    fn select_retries_on_invalid_input_then_accepts() {
        let response = make_response_with_order(&[("Proj/a", "Only option")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("banana\n9\n1\n");
        let picked =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap();
        assert_eq!(picked, Some(PathBuf::from("/tmp/a")));
        let rendered = String::from_utf8(output).unwrap();
        assert!(
            rendered.contains("invalid input 'banana'"),
            "first rejection must explain itself: {rendered}"
        );
        assert!(
            rendered.contains("invalid input '9'"),
            "second rejection must explain itself: {rendered}"
        );
    }

    #[test]
    fn select_bails_after_too_many_invalid_inputs() {
        let response = make_response_with_order(&[("Proj/a", "Only")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("x\ny\nz\n");
        let err =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap_err();
        assert!(format!("{err:#}").contains("invalid selection attempts"));
    }

    #[test]
    fn select_treats_zero_as_exit() {
        let response = make_response_with_order(&[("Proj/a", "Only")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("0\n");
        let picked =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap();
        assert_eq!(picked, None);
    }

    #[test]
    fn select_treats_q_as_exit() {
        let response = make_response_with_order(&[("Proj/a", "Only")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("q\n");
        let picked =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap();
        assert_eq!(picked, None);
    }

    #[test]
    fn select_treats_eof_as_exit() {
        let response = make_response_with_order(&[("Proj/a", "Only")]);
        let map = plan_map(&[("Proj/a", "/tmp/a")]);
        let mut output = Vec::new();
        let mut input = Cursor::new("");
        let picked =
            select_plan_from_response(&response, &map, &mut output, &mut input).unwrap();
        assert_eq!(picked, None);
    }

    #[test]
    fn build_plan_dir_map_errors_on_duplicate_key() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        // Two distinct paths that collapse to the same (project, plan)
        // pair under `<plan>/../..`-based derivation:
        //   tmp/a/Proj/LLM_STATE/dup → project=Proj, plan=dup
        //   tmp/b/Proj/LLM_STATE/dup → project=Proj, plan=dup
        let plan_a = tmp.path().join("a").join("Proj").join("LLM_STATE").join("dup");
        let plan_b = tmp.path().join("b").join("Proj").join("LLM_STATE").join("dup");
        fs::create_dir_all(&plan_a).unwrap();
        fs::create_dir_all(&plan_b).unwrap();
        fs::write(plan_a.join(PHASE_FILENAME), "work").unwrap();
        fs::write(plan_b.join(PHASE_FILENAME), "work").unwrap();
        let err = build_plan_dir_map(&[plan_a, plan_b]).unwrap_err();
        assert!(format!("{err:#}").contains("same project/plan key"));
    }

    #[test]
    fn build_plan_dir_map_errors_on_missing_phase_md() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("no-phase");
        fs::create_dir_all(&plan).unwrap();
        let err = build_plan_dir_map(&[plan]).unwrap_err();
        assert!(format!("{err:#}").contains("phase.md not found"));
    }
}
