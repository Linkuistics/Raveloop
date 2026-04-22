//! LLM-driven discovery of cross-project relationships.
//!
//! Two-stage pipeline keyed from the global projects catalog:
//! * Stage 1 (per-project, cached): subagent reads the project tree and
//!   emits a structured interaction-surface record.
//! * Stage 2 (global, uncached): one LLM call over all N surface records
//!   proposes edges, written to `<config-dir>/discover-proposals.yaml`
//!   for review.
//!
//! Spec: `docs/r7-related-projects-discovery-design.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

pub mod apply;
pub mod cache;
pub mod schema;
pub mod stage1;
pub mod stage2;
pub mod tree_sha;

use crate::config::{load_agent_config, load_shared_config};
use crate::projects::{self, ProjectEntry};

use self::schema::{ProposalsFile, Stage1Failure, SurfaceFile};
use self::stage1::{run_stage1, Stage1Config, Stage1Outcome};
use self::stage2::{run_stage2, Stage2Config};

pub const PROPOSALS_FILE: &str = "discover-proposals.yaml";
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_DISCOVER_MODEL: &str = "claude-sonnet-4-6";

pub struct RunDiscoverOptions {
    pub project_filter: Option<String>,
    pub concurrency: Option<usize>,
    pub apply: bool,
}

pub async fn run_discover(config_root: &Path, options: RunDiscoverOptions) -> Result<()> {
    let catalog = projects::load_or_empty(config_root)?;
    if catalog.projects.is_empty() {
        bail!("projects catalog is empty; nothing to discover");
    }

    let to_analyse: Vec<ProjectEntry> = match &options.project_filter {
        Some(name) => vec![catalog
            .find_by_name(name)
            .with_context(|| format!("project '{name}' not in catalog"))?
            .clone()],
        None => catalog.projects.clone(),
    };
    let all_projects = catalog.projects.clone();

    let shared = load_shared_config(config_root)?;
    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = agent_config
        .models
        .get("discover")
        .cloned()
        .unwrap_or_else(|| DEFAULT_DISCOVER_MODEL.to_string());

    let stage1_prompt = load_prompt(config_root, "discover-stage1.md")?;
    let stage2_prompt = load_prompt(config_root, "discover-stage2.md")?;

    let concurrency = options.concurrency.unwrap_or(DEFAULT_CONCURRENCY).max(1);

    let stage1_cfg = Stage1Config {
        config_root: config_root.to_path_buf(),
        model: model.clone(),
        prompt_template: stage1_prompt,
        concurrency,
        timeout: Duration::from_secs(stage1::DEFAULT_STAGE1_TIMEOUT_SECS),
    };

    eprintln!(
        "Stage 1: analysing {} project(s) with concurrency={}...",
        to_analyse.len(),
        concurrency
    );
    let outcomes = run_stage1(&to_analyse, &stage1_cfg).await?;

    // Collect surfaces for Stage 2. For a `--project` filter, fill in
    // the other catalogued projects from their cache so Stage 2 still
    // has the full set. Projects with no cache yet are skipped from
    // Stage 2 and recorded as "not yet analysed" failures.
    let mut surfaces: Vec<SurfaceFile> = Vec::new();
    let mut failures: Vec<Stage1Failure> = Vec::new();
    for outcome in outcomes {
        match outcome {
            Stage1Outcome::Fresh(s) | Stage1Outcome::Cached(s) => surfaces.push(s),
            Stage1Outcome::Failed(f) => failures.push(f),
        }
    }
    if options.project_filter.is_some() {
        for project in &all_projects {
            if surfaces.iter().any(|s| s.project == project.name) {
                continue;
            }
            if failures.iter().any(|f| f.project == project.name) {
                continue;
            }
            match cache::load(config_root, &project.name)? {
                Some(cached) => surfaces.push(cached),
                None => failures.push(Stage1Failure {
                    project: project.name.clone(),
                    error: "not yet analysed; run discover without --project to populate".to_string(),
                }),
            }
        }
    }

    eprintln!(
        "Stage 2: inferring edges over {} surface(s)...",
        surfaces.len()
    );
    let stage2_cfg = Stage2Config {
        config_root: config_root.to_path_buf(),
        model,
        prompt_template: stage2_prompt,
        timeout: Duration::from_secs(stage2::DEFAULT_STAGE2_TIMEOUT_SECS),
    };
    let proposals = run_stage2(&surfaces, failures, &stage2_cfg).await?;

    save_proposals_atomic(config_root, &proposals)?;

    let had_failures = !proposals.failures.is_empty();
    eprintln!(
        "Wrote {} proposal(s) and {} failure(s) to {}",
        proposals.proposals.len(),
        proposals.failures.len(),
        proposals_path(config_root).display()
    );

    if options.apply {
        apply::run_discover_apply(config_root)?;
    }

    if had_failures {
        bail!("discover completed with Stage 1 failures — see the failures section of the proposals file");
    }
    Ok(())
}

pub fn proposals_path(config_root: &Path) -> PathBuf {
    config_root.join(PROPOSALS_FILE)
}

pub fn save_proposals_atomic(config_root: &Path, file: &ProposalsFile) -> Result<()> {
    let path = proposals_path(config_root);
    let tmp = config_root.join(format!(".{PROPOSALS_FILE}.tmp"));
    let yaml = serde_yaml::to_string(file).context("serialise ProposalsFile")?;
    std::fs::write(&tmp, yaml.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load_proposals(config_root: &Path) -> Result<ProposalsFile> {
    let path = proposals_path(config_root);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: ProposalsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if file.schema_version != schema::PROPOSALS_SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}",
            path.display(),
            file.schema_version,
            schema::PROPOSALS_SCHEMA_VERSION
        );
    }
    Ok(file)
}

fn load_prompt(config_root: &Path, filename: &str) -> Result<String> {
    let path = config_root.join(filename);
    std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read prompt {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::schema::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_proposals_round_trips() {
        let tmp = TempDir::new().unwrap();
        let file = ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "2026-04-22T00:00:00Z".to_string(),
            source_tree_shas: [("A".to_string(), "abc".to_string())]
                .into_iter()
                .collect(),
            proposals: vec![],
            failures: vec![Stage1Failure {
                project: "B".to_string(),
                error: "oops".to_string(),
            }],
        };
        save_proposals_atomic(tmp.path(), &file).unwrap();
        let loaded = load_proposals(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }
}
