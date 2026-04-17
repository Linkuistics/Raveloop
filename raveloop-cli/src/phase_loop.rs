use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::dream::{should_dream, update_dream_baseline};
use crate::format::phase_info;
use crate::git::{git_commit_plan, git_save_work_baseline};
use crate::prompt::compose_prompt;
use crate::subagent::dispatch_subagents;
use crate::types::*;
use crate::ui::UI;

const HR: &str = "────────────────────────────────────────────────────";

fn read_phase(plan_dir: &Path) -> Result<Phase> {
    let content = fs::read_to_string(plan_dir.join("phase.md"))
        .context("Failed to read phase.md")?;
    Phase::parse(content.trim())
        .with_context(|| format!("Unknown phase: {}", content.trim()))
}

fn write_phase(plan_dir: &Path, phase: Phase) {
    let _ = fs::write(plan_dir.join("phase.md"), phase.to_string());
}

fn plan_name(plan_dir: &Path) -> String {
    plan_dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn log_phase_header(ui: &UI, phase: LlmPhase, plan: &str) {
    let info = phase_info(phase);
    ui.log(&format!("\n{HR}"));
    ui.log(&format!("  ◆  {}  ·  {plan}", info.label));
    ui.log(&format!("  {}", info.description));
    ui.log(HR);
}

fn log_commit(ui: &UI, phase_name: &str, plan: &str, result: &crate::git::CommitResult) {
    if result.committed {
        let first_line = result.message.lines().next().unwrap_or("");
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  {first_line}"));
    } else {
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  nothing to commit"));
    }
}

async fn handle_script_phase(
    phase: ScriptPhase,
    plan_dir: &Path,
    headroom: usize,
    ui: &UI,
) -> Result<bool> {
    let name = plan_name(plan_dir);

    match phase {
        ScriptPhase::GitCommitWork => {
            let result = git_commit_plan(plan_dir, &name, "work")?;
            log_commit(ui, "work", &name, &result);
            write_phase(plan_dir, Phase::Llm(LlmPhase::Reflect));
            Ok(ui.confirm("Proceed to reflect phase?").await)
        }
        ScriptPhase::GitCommitReflect => {
            let result = git_commit_plan(plan_dir, &name, "reflect")?;
            log_commit(ui, "reflect", &name, &result);
            if should_dream(plan_dir, headroom) {
                write_phase(plan_dir, Phase::Llm(LlmPhase::Dream));
            } else {
                ui.log("  ⏭  Dream skipped (memory within headroom)");
                write_phase(plan_dir, Phase::Script(ScriptPhase::GitCommitDream));
            }
            Ok(true)
        }
        ScriptPhase::GitCommitDream => {
            let result = git_commit_plan(plan_dir, &name, "dream")?;
            log_commit(ui, "dream", &name, &result);
            write_phase(plan_dir, Phase::Llm(LlmPhase::Triage));
            Ok(true)
        }
        ScriptPhase::GitCommitTriage => {
            let result = git_commit_plan(plan_dir, &name, "triage")?;
            log_commit(ui, "triage", &name, &result);
            write_phase(plan_dir, Phase::Llm(LlmPhase::Work));
            Ok(ui.confirm("Proceed to next work phase?").await)
        }
    }
}

pub async fn phase_loop(
    agent: Arc<dyn Agent>,
    ctx: &PlanContext,
    config: &SharedConfig,
    ui: &UI,
) -> Result<()> {
    let tokens = agent.tokens();
    let plan_dir = Path::new(&ctx.plan_dir);
    let config_root = Path::new(&ctx.config_root);
    let name = plan_name(plan_dir);

    if let Err(e) = agent.setup(ctx).await {
        ui.log(&format!("  ✗  Setup failed: {e}"));
    }

    loop {
        let phase = read_phase(plan_dir)?;

        match phase {
            Phase::Script(sp) => {
                if !handle_script_phase(sp, plan_dir, config.headroom, ui).await? {
                    ui.log("\nExiting.");
                    return Ok(());
                }
                continue;
            }
            Phase::Llm(lp) => {
                let agent_id = "main";
                log_phase_header(ui, lp, &name);

                ui.set_status(StatusInfo {
                    project: Path::new(&ctx.project_dir)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    plan: name.clone(),
                    phase: lp.as_str().to_string(),
                    agent: config.agent.clone(),
                    cycle: None,
                });

                if lp == LlmPhase::Work {
                    git_save_work_baseline(plan_dir);
                    let _ = fs::remove_file(plan_dir.join("latest-session.md"));
                }

                let prompt = compose_prompt(config_root, lp, ctx, &tokens)?;
                let tx = ui.sender();

                ui.register_agent(agent_id, &format!("  ◆  {}  ·  {name}", phase_info(lp).label));

                if lp == LlmPhase::Work {
                    ui.suspend();
                    agent.invoke_interactive(&prompt, ctx).await?;
                    ui.resume();
                } else {
                    agent.invoke_headless(&prompt, ctx, lp, agent_id, tx).await?;
                }

                let new_phase = read_phase(plan_dir)?;
                if new_phase == phase {
                    ui.log(&format!("\n  ✗  Phase did not advance from {phase}. Stopping."));
                    return Ok(());
                }

                if lp == LlmPhase::Dream {
                    update_dream_baseline(plan_dir);
                }

                if lp == LlmPhase::Triage {
                    dispatch_subagents(agent.clone(), plan_dir, ui).await?;
                }
            }
        }
    }
}
