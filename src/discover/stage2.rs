//! Stage 2: global edge inference over Stage 1 surface records.
//!
//! The LLM emits each proposed edge through
//! `ravel-lite state discover-proposals add-proposal` rather than writing
//! a YAML file directly. `run_stage2` pre-initialises the proposals file
//! (so the CLI has something to append to), spawns `claude` with the
//! `Bash` tool allowed, and loads the accumulated file after claude
//! exits. A hallucinated `--kind` is rejected by clap on the single bad
//! call, leaving all the other good proposals intact — the fix that
//! made the whole refactor worthwhile.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use super::schema::{ProposalsFile, Stage1Failure, SurfaceFile, PROPOSALS_SCHEMA_VERSION};
use super::stage1::current_utc_rfc3339;
use super::tree_sha::ProjectState;
use super::{load_proposals, save_proposals_atomic};
use crate::ontology::render_embedded_kinds_for_prompt;

pub const DEFAULT_STAGE2_TIMEOUT_SECS: u64 = 300;

pub struct Stage2Config {
    pub config_root: PathBuf,
    pub model: String,
    pub prompt_template: String,
    pub timeout: Duration,
}

/// Run Stage 2 over `surfaces`. `failures` from Stage 1 are seeded into
/// the proposals file before the LLM runs, so they survive the CLI
/// round-trip regardless of what (or how little) Stage 2 emits.
pub async fn run_stage2(
    surfaces: &[SurfaceFile],
    failures: Vec<Stage1Failure>,
    cfg: &Stage2Config,
) -> Result<ProposalsFile> {
    let source_project_states = surfaces
        .iter()
        .map(|s| {
            (
                s.project.clone(),
                ProjectState {
                    tree_sha: s.tree_sha.clone(),
                    dirty_hash: s.dirty_hash.clone(),
                },
            )
        })
        .collect();

    let initial = ProposalsFile {
        schema_version: PROPOSALS_SCHEMA_VERSION,
        generated_at: current_utc_rfc3339(),
        source_project_states,
        proposals: Vec::new(),
        failures,
    };
    save_proposals_atomic(&cfg.config_root, &initial)
        .context("pre-initialise discover-proposals.yaml for Stage 2 CLI appends")?;

    let surfaces_yaml = render_surfaces_for_prompt(surfaces)?;
    let ontology_block = render_embedded_kinds_for_prompt()
        .context("render shipped ontology YAML for Stage 2 prompt")?;
    let prompt = cfg
        .prompt_template
        .replace("{{ONTOLOGY_KINDS}}", &ontology_block)
        .replace("{{CONFIG_ROOT}}", &cfg.config_root.to_string_lossy())
        .replace("{{SURFACE_RECORDS_YAML}}", &surfaces_yaml);
    assert_no_dangling_tokens(&prompt)
        .context("composing Stage 2 prompt")?;

    let success = spawn_claude_for_stage2(&prompt, &cfg.model, &cfg.config_root, cfg.timeout).await?;
    if !success {
        bail!("Stage 2 claude subprocess exited non-zero");
    }

    load_proposals(&cfg.config_root).context(
        "load discover-proposals.yaml after Stage 2 — claude may have failed to \
         invoke any `ravel-lite state discover-proposals add-proposal` commands",
    )
}

/// Hard-error if the composed prompt still carries any `{{NAME}}`
/// placeholder after Stage 2's replacements. Discover doesn't route
/// through `prompt::substitute_tokens` (it has no `PlanContext`), so
/// an unresolved token would otherwise reach the LLM silently — the
/// same class of bug that motivated the canonical-substitution-path
/// rule for phase prompts.
fn assert_no_dangling_tokens(prompt: &str) -> Result<()> {
    use std::collections::BTreeSet;
    use std::sync::OnceLock;

    use regex::Regex;

    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\{\{([A-Za-z0-9_]+)\}\}").unwrap());
    let unresolved: BTreeSet<&str> = re
        .captures_iter(prompt)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    if unresolved.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = unresolved
        .iter()
        .map(|n| format!("{{{{{n}}}}}"))
        .collect();
    bail!(
        "Stage 2 prompt has unresolved token(s) after substitution: {}",
        names.join(", ")
    )
}

fn render_surfaces_for_prompt(surfaces: &[SurfaceFile]) -> Result<String> {
    // Emit a single YAML document with a top-level `surfaces:` list for
    // unambiguous consumption by the LLM.
    #[derive(serde::Serialize)]
    struct Wrapped<'a> {
        surfaces: &'a [SurfaceFile],
    }
    Ok(serde_yaml::to_string(&Wrapped { surfaces })?)
}

async fn spawn_claude_for_stage2(
    prompt: &str,
    model: &str,
    config_root: &std::path::Path,
    timeout: Duration,
) -> Result<bool> {
    // The LLM emits proposals via `ravel-lite state discover-proposals
    // add-proposal` (Bash). cwd=config_root keeps relative paths short in
    // the shell; `--add-dir` grants the sandbox read/write coverage of
    // the config dir so the subprocess's writes aren't blocked when the
    // user's allowlist is excluded by `--setting-sources project,local`.
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .arg("--allowed-tools")
        .arg("Bash")
        .arg("--add-dir")
        .arg(config_root)
        .current_dir(config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `claude`")?;
    let mut stdout = child.stdout.take().context("claude stdout pipe unavailable")?;
    let mut drain = String::new();
    let wait = tokio::time::timeout(timeout, async {
        let _ = stdout.read_to_string(&mut drain).await;
        child.wait().await
    })
    .await;
    match wait {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(io_err)) => Err(io_err).context("waiting on claude"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            bail!("claude Stage 2 timed out after {}s", timeout.as_secs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::{SurfaceRecord, SURFACE_SCHEMA_VERSION};

    /// Content of the shipped `defaults/discover-stage2.md` is embedded
    /// at compile time so the drift tests below run off the same bytes
    /// the released binary will read at runtime.
    const SHIPPED_STAGE2_PROMPT: &str = include_str!("../../defaults/discover-stage2.md");

    #[test]
    fn shipped_stage2_prompt_substitutes_cleanly_with_no_dangling_tokens() {
        // End-to-end composition: apply every substitution Stage 2
        // performs and then run the dangling-token guard. A new
        // `{{X}}` added to the prompt without a matching replacement
        // fails here rather than at LLM invocation time.
        let ontology_block = render_embedded_kinds_for_prompt().unwrap();
        let composed = SHIPPED_STAGE2_PROMPT
            .replace("{{ONTOLOGY_KINDS}}", &ontology_block)
            .replace("{{CONFIG_ROOT}}", "/tmp/cfg")
            .replace("{{SURFACE_RECORDS_YAML}}", "surfaces: []\n");
        assert_no_dangling_tokens(&composed).unwrap();
    }

    #[test]
    fn shipped_stage2_prompt_instructs_llm_to_invoke_add_proposal_cli() {
        // Regression guard: the prompt must route structured data through
        // `ravel-lite state discover-proposals add-proposal`, not through
        // a Write-YAML instruction. A prompt rewrite that accidentally
        // reverts to YAML output (e.g. reintroduces
        // `{{PROPOSALS_OUTPUT_PATH}}`) would bypass the clap-validation
        // boundary that motivated this shape.
        assert!(
            SHIPPED_STAGE2_PROMPT.contains("ravel-lite state discover-proposals add-proposal"),
            "Stage 2 prompt must instruct the LLM to invoke the add-proposal CLI"
        );
        assert!(
            !SHIPPED_STAGE2_PROMPT.contains("{{PROPOSALS_OUTPUT_PATH}}"),
            "Stage 2 prompt must not carry the legacy {{PROPOSALS_OUTPUT_PATH}} token"
        );
        assert!(
            SHIPPED_STAGE2_PROMPT.contains("{{CONFIG_ROOT}}"),
            "Stage 2 prompt must template the config root so the CLI call is unambiguous"
        );
    }

    #[test]
    fn shipped_stage2_prompt_renders_every_ontology_kind_as_a_bullet() {
        // Bijection drift guard: every ontology kind must show up as a
        // bullet in the block the prompt expands to. Missing coverage
        // would leave the LLM without a choice for that kind at Stage
        // 2 proposal time.
        use crate::ontology::EdgeKind;

        let ontology_block = render_embedded_kinds_for_prompt().unwrap();
        let composed = SHIPPED_STAGE2_PROMPT.replace("{{ONTOLOGY_KINDS}}", &ontology_block);

        for kind in EdgeKind::all() {
            let bullet = format!("- **`{}`** (", kind.as_str());
            assert!(
                composed.contains(&bullet),
                "Stage 2 prompt is missing a bullet for kind `{}`",
                kind.as_str()
            );
        }
    }

    #[test]
    fn shipped_stage2_prompt_declares_role_hints_are_priors_not_verdicts() {
        // Lock the prose guarantee that Stage 2 does not promote
        // `interaction_role_hints` into edges without cross-referenced
        // evidence. Migration doc §5.2 is load-bearing here; future
        // prompt rewrites that drop this guidance silently would let
        // self-declared roles mint weak edges.
        assert!(
            SHIPPED_STAGE2_PROMPT.contains("priors, not verdicts"),
            "Stage 2 prompt must say hints are priors, not verdicts"
        );
        assert!(
            SHIPPED_STAGE2_PROMPT.contains("interaction_role_hints"),
            "Stage 2 prompt must name the interaction_role_hints field explicitly"
        );
    }

    #[test]
    fn assert_no_dangling_tokens_reports_leftovers() {
        let err = assert_no_dangling_tokens("hello {{MISSING}} world").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("{{MISSING}}"), "message was: {msg}");
    }

    #[test]
    fn render_surfaces_emits_expected_structure() {
        fn make_surface(project: &str, purpose: &str) -> SurfaceFile {
            SurfaceFile {
                schema_version: SURFACE_SCHEMA_VERSION,
                project: project.to_string(),
                tree_sha: format!("sha-{project}"),
                dirty_hash: String::new(),
                analysed_at: "t".to_string(),
                surface: SurfaceRecord {
                    purpose: purpose.to_string(),
                    ..Default::default()
                },
            }
        }

        let surfaces = vec![make_surface("A", "alpha"), make_surface("B", "beta")];
        let rendered = render_surfaces_for_prompt(&surfaces).unwrap();

        // Surface order and wrapper-key shape are load-bearing for prompt
        // stability; assert structural parity via round-trip rather than
        // substring matches that pass even under ordering bugs.
        #[derive(serde::Deserialize)]
        struct Wrapped {
            surfaces: Vec<SurfaceFile>,
        }
        let parsed: Wrapped = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(parsed.surfaces.len(), 2);
        assert_eq!(parsed.surfaces[0].project, "A");
        assert_eq!(parsed.surfaces[1].project, "B");
        assert_eq!(parsed.surfaces[1].surface.purpose, "beta");
    }
}
