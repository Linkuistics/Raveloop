// src/prompt.rs
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use regex::Regex;

use crate::types::{LlmPhase, PlanContext};

/// Matches leftover `{{NAME}}` placeholders (ASCII letters, digits, `_`).
/// A failed substitution is almost always a typo in a phase prompt, so we
/// hard-error with the full set of names rather than log a warning — the
/// pi `{{MEMORY_DIR}}` bug slipped through precisely because a silent pass
/// reached the LLM unchanged.
fn unresolved_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{([A-Za-z0-9_]+)\}\}").unwrap())
}

/// Replace template tokens like {{PLAN}}, {{PROJECT}}, etc., then verify
/// no `{{NAME}}` placeholders remain. Returns `Err` listing every
/// unresolved token found, so drift in a phase prompt fails loudly at
/// compose time instead of silently reaching the LLM.
pub fn substitute_tokens(
    content: &str,
    ctx: &PlanContext,
    tokens: &HashMap<String, String>,
) -> Result<String> {
    let mut result = content.to_string();

    // Expand content macros first. These may inline authored content
    // (e.g. `related-plans.md`) that itself references path tokens; if
    // path tokens were substituted first, those inlined placeholders
    // would survive into the final output and trip the guard below.
    result = result.replace("{{RELATED_PLANS}}", &ctx.related_plans);
    for (key, value) in tokens {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }

    // Then expand atomic path tokens, so any placeholders surfaced by
    // the macro expansions above get resolved in the same pass.
    result = result.replace("{{DEV_ROOT}}", &ctx.dev_root);
    result = result.replace("{{PROJECT}}", &ctx.project_dir);
    result = result.replace("{{PLAN}}", &ctx.plan_dir);
    result = result.replace("{{ORCHESTRATOR}}", &ctx.config_root);

    let unresolved: BTreeSet<&str> = unresolved_token_regex()
        .captures_iter(&result)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if !unresolved.is_empty() {
        let names: Vec<String> = unresolved
            .iter()
            .map(|n| format!("{{{{{n}}}}}"))
            .collect();
        return Err(anyhow!(
            "Prompt contains unresolved token(s) after substitution: {}. \
             This usually indicates a typo in a phase prompt or a missing \
             agent-provided token.",
            names.join(", ")
        ));
    }

    Ok(result)
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

    substitute_tokens(&prompt, ctx, tokens)
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
        ).unwrap();
        assert_eq!(result, "plan=/plans/my-plan project=/project");
    }

    #[test]
    fn substitutes_custom_tokens() {
        let ctx = test_ctx();
        let mut tokens = HashMap::new();
        tokens.insert("TOOL_READ".to_string(), "Read".to_string());
        let result = substitute_tokens("Use {{TOOL_READ}}", &ctx, &tokens).unwrap();
        assert_eq!(result, "Use Read");
    }

    #[test]
    fn fails_on_unresolved_token() {
        let ctx = test_ctx();
        let err = substitute_tokens("needs {{MEMORY_DIR}} here", &ctx, &HashMap::new())
            .expect_err("unresolved token should fail");
        let msg = err.to_string();
        assert!(msg.contains("{{MEMORY_DIR}}"), "message was: {msg}");
    }

    #[test]
    fn lists_all_unresolved_tokens_sorted_and_deduped() {
        let ctx = test_ctx();
        let err = substitute_tokens(
            "{{UNKNOWN_B}} and {{UNKNOWN_A}} and {{UNKNOWN_A}} again",
            &ctx,
            &HashMap::new(),
        )
        .expect_err("unresolved tokens should fail");
        let msg = err.to_string();
        // BTreeSet ordering: A before B, duplicates collapsed.
        let a = msg.find("{{UNKNOWN_A}}").expect("missing UNKNOWN_A");
        let b = msg.find("{{UNKNOWN_B}}").expect("missing UNKNOWN_B");
        assert!(a < b, "names should be sorted: {msg}");
        assert_eq!(msg.matches("{{UNKNOWN_A}}").count(), 1, "dedup failed: {msg}");
    }

    #[test]
    fn substitutes_path_tokens_inside_related_plans() {
        // Regression: `related-plans.md` is documented (create-plan.md)
        // to use `{{DEV_ROOT}}` etc. for path references. Those tokens
        // must still resolve after the file content is inlined via
        // `{{RELATED_PLANS}}`, or every plan with a related-plans.md
        // hits a fatal "unresolved token" at prompt-compose time.
        let ctx = PlanContext {
            plan_dir: "/plans/my-plan".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "- {{DEV_ROOT}}/Peer — sibling project".to_string(),
            config_root: "/config".to_string(),
        };
        let result = substitute_tokens(
            "Related plans:\n{{RELATED_PLANS}}",
            &ctx,
            &HashMap::new(),
        )
        .expect("path tokens inside related_plans should resolve");
        assert_eq!(result, "Related plans:\n- /dev/Peer — sibling project");
    }

    #[test]
    fn accepts_single_brace_sequences() {
        // `{foo}` (rust format-style) and `{{foo}}` lowercase with a colon
        // inside shouldn't false-positive. The regex requires
        // [A-Za-z0-9_] names so punctuation breaks the match.
        let ctx = test_ctx();
        let result = substitute_tokens("keep {x} and {{not-a-token}}", &ctx, &HashMap::new())
            .unwrap();
        assert_eq!(result, "keep {x} and {{not-a-token}}");
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
