// src/prompt.rs
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::types::{LlmPhase, PlanContext};

/// Replace template tokens like {{PLAN}}, {{PROJECT}}, etc.
pub fn substitute_tokens(
    content: &str,
    ctx: &PlanContext,
    tokens: &HashMap<String, String>,
) -> String {
    let mut result = content.to_string();
    result = result.replace("{{DEV_ROOT}}", &ctx.dev_root);
    result = result.replace("{{PROJECT}}", &ctx.project_dir);
    result = result.replace("{{PLAN}}", &ctx.plan_dir);
    result = result.replace("{{RELATED_PLANS}}", &ctx.related_plans);
    result = result.replace("{{ORCHESTRATOR}}", &ctx.config_root);

    for (key, value) in tokens {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }

    result
}

/// Load the phase prompt file from the config root.
pub fn load_phase_file(config_root: &Path, phase: LlmPhase) -> Result<String> {
    let path = config_root.join("phases").join(format!("{}.md", phase));
    fs::read_to_string(&path)
        .with_context(|| format!("Failed to read phase file: {}", path.display()))
}

/// Load an optional plan-specific prompt override.
pub fn load_plan_override(plan_dir: &Path, phase: LlmPhase) -> Option<String> {
    let path = plan_dir.join(format!("prompt-{}.md", phase));
    fs::read_to_string(&path).ok()
}

/// Compose the full prompt for a phase.
pub fn compose_prompt(
    config_root: &Path,
    phase: LlmPhase,
    ctx: &PlanContext,
    tokens: &HashMap<String, String>,
) -> Result<String> {
    let base = load_phase_file(config_root, phase)?;
    let override_text = load_plan_override(Path::new(&ctx.plan_dir), phase);

    let mut prompt = base;
    if let Some(ov) = override_text {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(&ov);
    }

    Ok(substitute_tokens(&prompt, ctx, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> PlanContext {
        PlanContext {
            plan_dir: "/plans/my-plan".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "related stuff".to_string(),
            config_root: "/config".to_string(),
        }
    }

    #[test]
    fn substitutes_built_in_tokens() {
        let ctx = test_ctx();
        let result = substitute_tokens(
            "plan={{PLAN}} project={{PROJECT}}",
            &ctx,
            &HashMap::new(),
        );
        assert_eq!(result, "plan=/plans/my-plan project=/project");
    }

    #[test]
    fn substitutes_custom_tokens() {
        let ctx = test_ctx();
        let mut tokens = HashMap::new();
        tokens.insert("TOOL_READ".to_string(), "Read".to_string());
        let result = substitute_tokens("Use {{TOOL_READ}}", &ctx, &tokens);
        assert_eq!(result, "Use Read");
    }

    #[test]
    fn compose_prompt_loads_and_substitutes() {
        let dir = tempfile::TempDir::new().unwrap();
        let phases_dir = dir.path().join("phases");
        std::fs::create_dir_all(&phases_dir).unwrap();
        std::fs::write(
            phases_dir.join("reflect.md"),
            "Reflect on {{PLAN}}",
        ).unwrap();

        let ctx = PlanContext {
            plan_dir: "/plans/test".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "".to_string(),
            config_root: dir.path().to_string_lossy().to_string(),
        };

        let result = compose_prompt(
            dir.path(),
            LlmPhase::Reflect,
            &ctx,
            &HashMap::new(),
        ).unwrap();

        assert_eq!(result, "Reflect on /plans/test");
    }
}
