// src/survey/invoke.rs
//
// Spawn + read the `claude` CLI for the survey, and the end-to-end
// orchestrator that ties plan loading, composition, invocation,
// parsing, hash injection, and YAML emission together. Markdown
// rendering is now a separate concern delegated to the
// `ravel-lite survey-format` subcommand.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};
use crate::types::AgentConfig;

use super::compose::{
    load_survey_incremental_prompt, load_survey_prompt, render_survey_input,
    render_survey_input_incremental,
};
use super::delta::{merge_delta, PlanClassification};
use super::discover::{load_plan, PlanSnapshot};
use super::render::render_survey_output;
use super::schema::{
    emit_survey_yaml, inject_input_hashes, inject_plan_row_counts, inject_task_counts,
    parse_survey_response, plan_key, SurveyResponse, SCHEMA_VERSION,
};
use crate::state::backlog::{PlanRowCounts, TaskCounts};

/// Fallback model when neither `--model` nor `models.survey` is
/// configured. A cheap, fast model is appropriate: survey is a
/// summarisation task over plain-text inputs.
pub const DEFAULT_SURVEY_MODEL: &str = "claude-haiku-4-5";

/// Default ceiling on how long the `claude` subprocess may run before
/// survey gives up. Survey is advertised as single-shot and read-only;
/// a hang with no feedback is a worse failure mode than a loud error
/// that the user can retry. Five minutes is generous for any model
/// summarising plain-text inputs and short enough to surface problems
/// before the user walks away.
pub const DEFAULT_SURVEY_TIMEOUT_SECS: u64 = 300;

fn resolve_timeout(flag_override: Option<u64>) -> Duration {
    Duration::from_secs(flag_override.unwrap_or(DEFAULT_SURVEY_TIMEOUT_SECS))
}

/// Resolve which model to use for the survey call. Precedence:
///   1. explicit `--model` flag on the CLI
///   2. `models.survey` in the agent's config
///   3. `DEFAULT_SURVEY_MODEL` constant
fn resolve_model(agent_config: &AgentConfig, flag_override: Option<String>) -> String {
    flag_override
        .or_else(|| agent_config.models.get("survey").cloned())
        .unwrap_or_else(|| DEFAULT_SURVEY_MODEL.to_string())
}

/// End-to-end survey runner for the `ravel-lite survey` CLI subcommand.
/// Produces a canonical `SurveyResponse` (see `compute_survey_response`)
/// and writes it to stdout as YAML. Callers that need the response
/// in-memory — notably the multi-plan runner — should use
/// `compute_survey_response` directly and persist the YAML themselves.
pub async fn run_survey(
    config_root: &Path,
    plan_dirs: &[PathBuf],
    model_override: Option<String>,
    timeout_override_secs: Option<u64>,
    prior_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let response = compute_survey_response(
        config_root,
        plan_dirs,
        model_override,
        timeout_override_secs,
        prior_path,
        force,
    )
    .await?;
    print!("{}", emit_survey_yaml(&response)?);
    Ok(())
}

/// Core survey pipeline: loads each named plan directory, composes the
/// prompt, invokes the `claude` CLI headlessly, parses the YAML response,
/// injects Rust-computed `input_hash` values into each row, and returns
/// the canonical `SurveyResponse`.
///
/// With `prior_path = Some(path)` and `force = false` the runner uses the
/// incremental path: it hashes each plan's state files, compares against
/// the prior's per-row `input_hash`, and sends only the changed+added
/// plans to the LLM. Unchanged rows are carried forward verbatim.
///
/// `force = true` short-circuits the hash comparison and re-analyses
/// every plan as if no prior were supplied — but still validates the
/// prior's `schema_version` matches the binary's `SCHEMA_VERSION`, so
/// a schema-bumped prior fails loudly rather than silently losing
/// fields on merge. With `prior_path = None`, `force` is a no-op.
///
/// The plan-root walk is gone: each positional argument on the CLI
/// names exactly one plan directory (a directory containing `phase.md`).
/// Routing responsibility stays in the caller.
pub async fn compute_survey_response(
    config_root: &Path,
    plan_dirs: &[PathBuf],
    model_override: Option<String>,
    timeout_override_secs: Option<u64>,
    prior_path: Option<&Path>,
    force: bool,
) -> Result<SurveyResponse> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "survey currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = resolve_model(&agent_config, model_override);

    if plan_dirs.is_empty() {
        anyhow::bail!("No plan directories supplied.");
    }
    let mut all_plans = Vec::with_capacity(plan_dirs.len());
    for plan_dir in plan_dirs {
        let snapshot = load_plan(plan_dir)
            .with_context(|| format!("Failed to load plan at {}", plan_dir.display()))?;
        all_plans.push(snapshot);
    }
    all_plans.sort_by(|a, b| (&a.project, &a.plan).cmp(&(&b.project, &b.plan)));

    let prior = match prior_path {
        Some(path) => Some(load_and_validate_prior(path)?),
        None => None,
    };

    // Incremental-eligible: prior present AND user did not force a
    // full re-analysis.
    if let Some(prior) = prior.as_ref() {
        if !force {
            return run_incremental_survey(
                config_root,
                &model,
                timeout_override_secs,
                prior,
                all_plans,
            )
            .await;
        }
    }

    // Cold path: no prior, OR `--force` overrides the incremental path.
    run_cold_survey(
        config_root,
        &model,
        timeout_override_secs,
        &all_plans,
    )
    .await
}

async fn run_cold_survey(
    config_root: &Path,
    model: &str,
    timeout_override_secs: Option<u64>,
    all_plans: &[PlanSnapshot],
) -> Result<SurveyResponse> {
    let survey_prompt = load_survey_prompt(config_root)?;
    let plan_input = render_survey_input(all_plans);
    let full_prompt = format!("{survey_prompt}\n\n---\n{plan_input}");

    eprintln!(
        "Surveying {} plan(s) using model {}...",
        all_plans.len(),
        model
    );

    let output = spawn_claude_and_read(&full_prompt, model, timeout_override_secs).await?;
    let mut response = parse_survey_response(&output)?;
    let hashes: HashMap<String, String> = all_plans
        .iter()
        .map(|p| (plan_key(&p.project, &p.plan), p.input_hash.clone()))
        .collect();
    inject_input_hashes(&mut response, &hashes)?;
    inject_task_counts(&mut response, &collect_task_counts(all_plans.iter()));
    inject_plan_row_counts(&mut response, &collect_plan_row_counts(all_plans.iter()));
    // Cold-path response carries whatever schema_version the LLM
    // emitted (or the default); pin it to the binary's current value
    // so persisted YAML is always labelled with the producer version.
    response.schema_version = SCHEMA_VERSION;
    Ok(response)
}

/// Collect `(plan_key, TaskCounts)` entries for snapshots whose
/// `backlog.yaml` parsed successfully. Snapshots with `task_counts: None`
/// are skipped; downstream `inject_task_counts` leaves their rows'
/// counts as `None` so the LLM's `notes` can explain the gap.
fn collect_task_counts<'a, I>(snapshots: I) -> HashMap<String, TaskCounts>
where
    I: Iterator<Item = &'a PlanSnapshot>,
{
    snapshots
        .filter_map(|p| {
            p.task_counts
                .map(|counts| (plan_key(&p.project, &p.plan), counts))
        })
        .collect()
}

/// Collect `(plan_key, PlanRowCounts)` entries for snapshots whose
/// `backlog.yaml` parsed successfully. Same skip-on-None semantics as
/// `collect_task_counts`; downstream `inject_plan_row_counts` leaves
/// unmatched rows at their serde defaults (zeros) so a missing backlog
/// shows up as `0/0/0` alongside the existing `task_counts: None`.
fn collect_plan_row_counts<'a, I>(snapshots: I) -> HashMap<String, PlanRowCounts>
where
    I: Iterator<Item = &'a PlanSnapshot>,
{
    snapshots
        .filter_map(|p| {
            p.plan_row_counts
                .map(|counts| (plan_key(&p.project, &p.plan), counts))
        })
        .collect()
}

async fn run_incremental_survey(
    config_root: &Path,
    model: &str,
    timeout_override_secs: Option<u64>,
    prior: &SurveyResponse,
    all_plans: Vec<PlanSnapshot>,
) -> Result<SurveyResponse> {
    let classification = PlanClassification::classify(prior, &all_plans);

    // Nothing changed relative to prior: skip the LLM call entirely
    // and return the prior through as the new output. This is the
    // fast-path that makes per-cycle surveying affordable.
    //
    // Even in the fast-path, re-inject task_counts and plan_row_counts
    // from the current snapshots. The prior's values came from whichever
    // binary last ran — and for a pre-extraction prior those counts may
    // be LLM-inferred rather than Rust-computed. Same-snapshot input
    // re-injection produces identical values for a current-binary prior
    // and fixes up any migration drift for an older one.
    if classification.is_noop() {
        eprintln!(
            "Incremental survey: all {} plan(s) unchanged — carrying prior forward.",
            all_plans.len()
        );
        let mut carried = prior.clone();
        inject_task_counts(&mut carried, &collect_task_counts(all_plans.iter()));
        inject_plan_row_counts(&mut carried, &collect_plan_row_counts(all_plans.iter()));
        carried.schema_version = SCHEMA_VERSION;
        return Ok(carried);
    }

    eprintln!(
        "Incremental survey: {} unchanged, {} changed, {} added, {} removed — \
         analysing {} plan(s) using model {}...",
        classification.unchanged_rows.len(),
        classification.changed.len(),
        classification.added.len(),
        classification.removed_keys.len(),
        classification.changed.len() + classification.added.len(),
        model,
    );

    let to_analyse = classification.plans_to_analyse();
    let prior_yaml = emit_survey_yaml(prior)?;
    let prompt_template = load_survey_incremental_prompt(config_root)?;
    let plan_input =
        render_survey_input_incremental(&to_analyse, &prior_yaml, &classification.removed_keys);
    let full_prompt = format!("{prompt_template}\n\n---\n{plan_input}");

    let output = spawn_claude_and_read(&full_prompt, model, timeout_override_secs).await?;
    let mut delta_response = parse_survey_response(&output)?;

    // Inject hashes into the delta rows before merge: the LLM doesn't
    // emit hashes, and the merged result must carry them for the next
    // cycle's delta classification.
    let delta_hashes: HashMap<String, String> = classification
        .changed
        .iter()
        .chain(classification.added.iter())
        .map(|s| (plan_key(&s.project, &s.plan), s.input_hash.clone()))
        .collect();
    inject_input_hashes(&mut delta_response, &delta_hashes)?;
    // Inject task counts for the delta rows from their snapshots;
    // unchanged rows carry their task_counts forward from `prior`
    // verbatim via `merge_delta`, so no injection is needed there.
    inject_task_counts(
        &mut delta_response,
        &collect_task_counts(
            classification
                .changed
                .iter()
                .chain(classification.added.iter())
                .copied(),
        ),
    );
    inject_plan_row_counts(
        &mut delta_response,
        &collect_plan_row_counts(
            classification
                .changed
                .iter()
                .chain(classification.added.iter())
                .copied(),
        ),
    );

    let mut merged = merge_delta(classification, delta_response)?;
    merged.schema_version = SCHEMA_VERSION;
    Ok(merged)
}

/// Load and parse a prior survey YAML, and verify its
/// `schema_version` matches the binary. A mismatched version is a
/// hard error that hints at `--force` as the remediation path —
/// `--force` bypasses hash comparison but still validates the schema,
/// so the remediation for a genuinely incompatible prior is to
/// re-run without `--prior`.
fn load_and_validate_prior(path: &Path) -> Result<SurveyResponse> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read prior survey at {}", path.display()))?;
    let prior = parse_survey_response(&content)
        .with_context(|| format!("Failed to parse prior survey at {}", path.display()))?;
    if prior.schema_version != SCHEMA_VERSION {
        anyhow::bail!(
            "prior survey at {} declares schema_version={}, but this binary \
             speaks schema_version={}. Re-run without `--prior` to produce a \
             fresh baseline.",
            path.display(),
            prior.schema_version,
            SCHEMA_VERSION,
        );
    }
    Ok(prior)
}

/// Spawn `claude -p <prompt>` headlessly, wait for stdout with the
/// configured timeout, and return captured output. Factored out of
/// the cold and incremental runners so both share identical spawn,
/// timeout, and error-surfacing behaviour.
async fn spawn_claude_and_read(
    prompt: &str,
    model: &str,
    timeout_override_secs: Option<u64>,
) -> Result<String> {
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
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
    let timeout = resolve_timeout(timeout_override_secs);
    let start = Instant::now();
    let read_result = tokio::time::timeout(timeout, stdout.read_to_string(&mut output)).await;

    match read_result {
        Ok(Ok(_)) => {}
        Ok(Err(io_err)) => {
            let _ = child.kill().await;
            return Err(io_err).context("failed reading stdout from claude");
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            anyhow::bail!(
                "claude CLI did not produce a result within {}s timeout (elapsed {}s).\n\
                 Captured {} bytes of stdout before timing out:\n{}\n\n\
                 Try one of:\n  \
                 * re-run the command (transient hangs sometimes clear)\n  \
                 * swap the model with --model <other>\n  \
                 * check network / API reachability\n  \
                 * extend the limit with --timeout-secs <N>",
                timeout.as_secs(),
                start.elapsed().as_secs(),
                output.len(),
                output,
            );
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude CLI exited with status {status}");
    }
    Ok(output)
}

/// Render a saved YAML survey file as human-readable markdown on
/// stdout. Parsing goes through the same schema + `parse_survey_response`
/// path as `run_survey`, then delegates to `render_survey_output` —
/// separates presentation from the (possibly expensive) LLM call and
/// lets the user re-render a stored survey cheaply.
pub fn run_survey_format(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read survey file at {}", path.display()))?;
    let response = parse_survey_response(&content)?;
    print!("{}", render_survey_output(&response));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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

    #[test]
    fn resolve_timeout_uses_default_when_no_override() {
        assert_eq!(resolve_timeout(None), Duration::from_secs(DEFAULT_SURVEY_TIMEOUT_SECS));
    }

    #[test]
    fn resolve_timeout_honours_override() {
        assert_eq!(resolve_timeout(Some(42)), Duration::from_secs(42));
    }

    #[test]
    fn run_survey_format_renders_markdown_from_yaml_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("survey.yaml");
        std::fs::write(
            &path,
            "plans:\n  - project: P\n    plan: x\n    phase: work\n    unblocked: 1\n\
             \n    blocked: 0\n    done: 0\n    received: 0\n    notes: ''\n",
        )
        .unwrap();
        // run_survey_format writes to stdout, so this test only checks
        // the happy path doesn't error. Content-level golden rendering
        // is covered by render_survey_output's own tests.
        run_survey_format(&path).unwrap();
    }

    #[test]
    fn run_survey_format_errors_on_missing_file() {
        let missing = std::path::PathBuf::from("/definitely/not/a/survey/file.yaml");
        let err = run_survey_format(&missing).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to read survey file"));
    }

    #[test]
    fn load_and_validate_prior_accepts_matching_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("prior.yaml");
        let yaml = format!(
            "schema_version: {SCHEMA_VERSION}\nplans:\n  - project: P\n    plan: x\n    \
             phase: work\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n"
        );
        std::fs::write(&path, yaml).unwrap();
        let prior = load_and_validate_prior(&path).unwrap();
        assert_eq!(prior.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn load_and_validate_prior_accepts_missing_version_field() {
        // 5a-emitted YAML has no schema_version — the serde default
        // should inject SCHEMA_VERSION so the validation passes. This
        // is the one-time 5a→5b amnesty.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("prior.yaml");
        std::fs::write(
            &path,
            "plans:\n  - project: P\n    plan: x\n    phase: work\n    \
             unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n",
        )
        .unwrap();
        let prior = load_and_validate_prior(&path).unwrap();
        assert_eq!(prior.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn load_and_validate_prior_rejects_mismatched_version_with_remediation_hint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("prior.yaml");
        // A future version marker that this binary doesn't speak.
        std::fs::write(
            &path,
            "schema_version: 9999\nplans:\n  - project: P\n    plan: x\n    phase: work\n    \
             unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n",
        )
        .unwrap();
        let err = load_and_validate_prior(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version=9999"), "got: {msg}");
        assert!(
            msg.contains("Re-run without `--prior`"),
            "error should point at the remediation path; got: {msg}"
        );
    }

    #[test]
    fn load_and_validate_prior_errors_on_missing_file() {
        let missing = std::path::PathBuf::from("/definitely/not/a/prior/survey.yaml");
        let err = load_and_validate_prior(&missing).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to read prior survey"));
    }

    #[test]
    fn load_and_validate_prior_errors_on_malformed_yaml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.yaml");
        std::fs::write(&path, "not: valid: yaml: at: all:\n  - [").unwrap();
        let err = load_and_validate_prior(&path).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to parse prior survey"));
    }

    #[test]
    fn run_survey_format_errors_on_malformed_yaml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.yaml");
        std::fs::write(&path, "not: valid: yaml: at: all:\n  - [").unwrap();
        let err = run_survey_format(&path).unwrap_err();
        assert!(format!("{err:#}").contains("Failed to parse survey response"));
    }
}
