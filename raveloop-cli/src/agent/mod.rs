// src/agent/mod.rs
pub mod claude_code;
pub mod pi;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

use crate::types::PlanContext;
use crate::types::LlmPhase;
use crate::ui::UISender;

#[async_trait]
pub trait Agent: Send + Sync {
    /// Interactive phase — agent owns the terminal.
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()>;

    /// Headless phase — streams events to the TUI.
    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()>;

    /// Dispatch a subagent to a target plan.
    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()>;

    fn tokens(&self) -> HashMap<String, String>;

    async fn setup(&self, _ctx: &PlanContext) -> Result<()> {
        Ok(())
    }
}
