use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::task::JoinSet;

use crate::agent::Agent;
use crate::types::SubagentDispatch;
use crate::ui::UI;

#[derive(serde::Deserialize)]
struct DispatchFile {
    #[serde(default)]
    dispatches: Vec<SubagentDispatch>,
}

pub fn parse_dispatch_file(plan_dir: &Path) -> Result<Vec<SubagentDispatch>> {
    let path = plan_dir.join("subagent-dispatch.yaml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let file: DispatchFile = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(file.dispatches)
}

fn build_subagent_prompt(dispatch: &SubagentDispatch) -> String {
    format!(
        "Plan at {}.\n\
         This is a {} plan that may be affected by recent learnings.\n\n\
         Summary of learnings to apply:\n{}\n\n\
         Read the target plan's backlog.md and memory.md.\n\
         Apply relevant updates: add/modify tasks in backlog.md, update memory.md if needed.\n\
         Be conservative — only change what the summary warrants.",
        dispatch.target, dispatch.kind, dispatch.summary
    )
}

pub async fn dispatch_subagents(
    agent: Arc<dyn Agent>,
    plan_dir: &Path,
    ui: &UI,
) -> Result<()> {
    let dispatches = parse_dispatch_file(plan_dir)?;
    if dispatches.is_empty() {
        return Ok(());
    }

    ui.log(&format!("\n▶ Dispatching {} subagent(s)...", dispatches.len()));

    let mut join_set: JoinSet<(String, Result<()>)> = JoinSet::new();

    for dispatch in &dispatches {
        let agent_id = Path::new(&dispatch.target)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dispatch.target.clone());

        ui.register_agent(
            &agent_id,
            &format!("  → {}: {}", dispatch.kind, dispatch.target),
        );
        ui.log(&format!("  → {}: {}", dispatch.kind, dispatch.target));

        let agent = Arc::clone(&agent);
        let prompt = build_subagent_prompt(dispatch);
        let target = dispatch.target.clone();
        let id = agent_id.clone();
        let tx = ui.sender();

        join_set.spawn(async move {
            let result = agent.dispatch_subagent(&prompt, &target, &id, tx).await;
            (id, result)
        });
    }

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((agent_id, Ok(()))) => {
                ui.log(&format!("  ✓ {agent_id}"));
            }
            Ok((agent_id, Err(e))) => {
                ui.log(&format!("  ✗ {agent_id}: {e}"));
            }
            Err(e) => {
                ui.log(&format!("  ✗ join error: {e}"));
            }
        }
    }

    let dispatch_path = plan_dir.join("subagent-dispatch.yaml");
    if dispatch_path.exists() {
        fs::remove_file(&dispatch_path).ok();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_empty_dispatch() {
        let dir = TempDir::new().unwrap();
        let result = parse_dispatch_file(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_dispatch_file_with_entries() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("subagent-dispatch.yaml"),
            "dispatches:\n  - target: /plans/sub-B\n    kind: child\n    summary: Update backlog\n",
        ).unwrap();
        let result = parse_dispatch_file(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].target, "/plans/sub-B");
        assert_eq!(result[0].kind, "child");
    }
}
