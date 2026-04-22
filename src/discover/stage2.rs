//! Stage 2: global edge inference over Stage 1 surface records.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use super::schema::{
    ProposalRecord, ProposalsFile, Stage1Failure, SurfaceFile, PROPOSALS_SCHEMA_VERSION,
};

pub const DEFAULT_STAGE2_TIMEOUT_SECS: u64 = 300;

pub struct Stage2Config {
    pub config_root: PathBuf,
    pub model: String,
    pub prompt_template: String,
    pub timeout: Duration,
}

/// Run Stage 2 over `surfaces`. `failures` from Stage 1 are passed
/// through unchanged into the output.
pub async fn run_stage2(
    surfaces: &[SurfaceFile],
    failures: Vec<Stage1Failure>,
    cfg: &Stage2Config,
) -> Result<ProposalsFile> {
    let output_path = cfg
        .config_root
        .join(format!(".tmp-proposals-{}.yaml", std::process::id()));
    if output_path.exists() {
        std::fs::remove_file(&output_path).with_context(|| {
            format!("failed to remove stale tmp file {}", output_path.display())
        })?;
    }

    let surfaces_yaml = render_surfaces_for_prompt(surfaces)?;
    let prompt = cfg
        .prompt_template
        .replace("{{PROPOSALS_OUTPUT_PATH}}", &output_path.to_string_lossy())
        .replace("{{SURFACE_RECORDS_YAML}}", &surfaces_yaml);

    let success = spawn_claude_for_stage2(&prompt, &cfg.model, cfg.timeout).await?;
    if !success {
        bail!("Stage 2 claude subprocess exited non-zero");
    }
    if !output_path.exists() {
        bail!("Stage 2 did not create {}", output_path.display());
    }

    let raw = std::fs::read_to_string(&output_path).with_context(|| {
        format!("failed to read Stage 2 output {}", output_path.display())
    })?;
    let raw_parsed: RawStage2Output = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse Stage 2 output from {}", output_path.display()))?;
    let _ = std::fs::remove_file(&output_path);

    let source_tree_shas = surfaces
        .iter()
        .map(|s| (s.project.clone(), s.tree_sha.clone()))
        .collect();

    Ok(ProposalsFile {
        schema_version: PROPOSALS_SCHEMA_VERSION,
        generated_at: raw_parsed.generated_at,
        source_tree_shas,
        proposals: raw_parsed.proposals,
        failures,
    })
}

#[derive(serde::Deserialize)]
struct RawStage2Output {
    generated_at: String,
    #[serde(default)]
    proposals: Vec<ProposalRecord>,
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
    timeout: Duration,
) -> Result<bool> {
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

    #[test]
    fn render_surfaces_emits_expected_structure() {
        fn make_surface(project: &str, purpose: &str) -> SurfaceFile {
            SurfaceFile {
                schema_version: SURFACE_SCHEMA_VERSION,
                project: project.to_string(),
                tree_sha: format!("sha-{project}"),
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
