// src/config.rs
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::types::{AgentConfig, SharedConfig};

/// Environment variable that overrides the default config-directory location.
pub const CONFIG_ENV_VAR: &str = "RAVEL_LITE_CONFIG";

/// Resolve the Ravel-Lite config directory using the precedence chain:
///   1. explicit `--config <path>` flag
///   2. `RAVEL_LITE_CONFIG` environment variable
///   3. XDG default at `<dirs::config_dir()>/ravel-lite/`
///   4. hard error (no walk-up, no magic, no registry)
///
/// The resolved path must be an existing directory; otherwise an
/// actionable error pointing at `ravel-lite init` is returned.
pub fn resolve_config_dir(explicit_flag: Option<PathBuf>) -> Result<PathBuf> {
    let env_var = std::env::var(CONFIG_ENV_VAR).ok().map(PathBuf::from);
    let xdg_default = dirs::config_dir().map(|p| p.join("ravel-lite"));
    select_config_dir(explicit_flag, env_var, xdg_default)
}

/// Pure resolution: picks the first non-None source in precedence order
/// and validates that the path is an existing directory. Broken out from
/// `resolve_config_dir` so it can be tested without touching the
/// environment or the filesystem-default lookup.
fn select_config_dir(
    explicit: Option<PathBuf>,
    env: Option<PathBuf>,
    default: Option<PathBuf>,
) -> Result<PathBuf> {
    let (source, candidate) = if let Some(path) = explicit {
        ("--config flag".to_string(), path)
    } else if let Some(path) = env {
        (format!("environment variable {CONFIG_ENV_VAR}"), path)
    } else if let Some(path) = default {
        ("default location (dirs::config_dir()/ravel-lite)".to_string(), path)
    } else {
        anyhow::bail!(
            "Could not resolve Ravel-Lite config directory.\n\
             No --config flag, no RAVEL_LITE_CONFIG environment variable, and no user config dir available on this platform.\n\
             Create one with `ravel-lite init <dir>` and either pass --config <dir> or set RAVEL_LITE_CONFIG=<dir>."
        );
    };

    if !candidate.is_dir() {
        anyhow::bail!(
            "Ravel-Lite config directory {} (from {}) does not exist or is not a directory.\n\
             Create it with `ravel-lite init {}`.",
            candidate.display(),
            source,
            candidate.display()
        );
    }

    Ok(candidate)
}

/// Recursively merges `overlay` into `base`: scalar collisions are won
/// by overlay, map collisions recurse key-by-key, so keys present only
/// in `base` survive. Mirrors the shape of JSON-merge-patch for the
/// subset of YAML we actually use (maps and scalars).
fn merge_yaml(base: &mut serde_yaml::Value, overlay: serde_yaml::Value) {
    use serde_yaml::Value;
    match (base, overlay) {
        (Value::Mapping(base_map), Value::Mapping(overlay_map)) => {
            for (key, value) in overlay_map {
                if let Some(existing) = base_map.get_mut(&key) {
                    merge_yaml(existing, value);
                } else {
                    base_map.insert(key, value);
                }
            }
        }
        (base_slot, overlay) => {
            *base_slot = overlay;
        }
    }
}

/// Load a YAML config with an optional `*.local.yaml` overlay that is
/// deep-merged into the embedded-default base before deserialization.
///
/// The overlay is the single-user escape hatch for `init --force`:
/// `init` only writes files in `EMBEDDED_FILES`, so any path ending in
/// `.local.yaml` is untouched by `init --force`. This lets a user pin a
/// field (e.g. `models.work: ""` to defer to Claude Code's interactive
/// default, or `provider: openai`) without it being reset on the next
/// `init --force` sweep.
fn load_with_optional_overlay<T: serde::de::DeserializeOwned>(
    base_path: &Path,
    overlay_path: &Path,
) -> Result<T> {
    let base_content = std::fs::read_to_string(base_path)
        .with_context(|| format!("Failed to read {}", base_path.display()))?;
    let mut merged: serde_yaml::Value = serde_yaml::from_str(&base_content)
        .with_context(|| format!("Failed to parse {}", base_path.display()))?;

    if overlay_path.exists() {
        let overlay_content = std::fs::read_to_string(overlay_path)
            .with_context(|| format!("Failed to read {}", overlay_path.display()))?;
        let overlay: serde_yaml::Value = serde_yaml::from_str(&overlay_content)
            .with_context(|| format!("Failed to parse {}", overlay_path.display()))?;
        merge_yaml(&mut merged, overlay);
    }

    serde_yaml::from_value(merged).with_context(|| {
        format!(
            "Failed to deserialize merged config from {} (+ optional {})",
            base_path.display(),
            overlay_path.display()
        )
    })
}

pub fn load_shared_config(config_root: &Path) -> Result<SharedConfig> {
    load_with_optional_overlay(
        &config_root.join("config.yaml"),
        &config_root.join("config.local.yaml"),
    )
}

pub fn load_agent_config(config_root: &Path, agent_name: &str) -> Result<AgentConfig> {
    let agent_dir = config_root.join("agents").join(agent_name);
    load_with_optional_overlay(
        &agent_dir.join("config.yaml"),
        &agent_dir.join("config.local.yaml"),
    )
}

pub fn load_tokens(config_root: &Path, agent_name: &str) -> Result<std::collections::HashMap<String, String>> {
    let agent_dir = config_root.join("agents").join(agent_name);
    load_with_optional_overlay(
        &agent_dir.join("tokens.yaml"),
        &agent_dir.join("tokens.local.yaml"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_config_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            "agent: claude-code\nheadroom: 1500\n",
        ).unwrap();
        let agent_dir = dir.path().join("agents/claude-code");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("config.yaml"),
            "models:\n  work: claude-sonnet-4-6\n  reflect: claude-haiku-4-5\n",
        ).unwrap();
        fs::write(
            agent_dir.join("tokens.yaml"),
            "TOOL_READ: Read\nTOOL_WRITE: Write\n",
        ).unwrap();
        dir
    }

    #[test]
    fn loads_shared_config() {
        let dir = setup_config_dir();
        let config = load_shared_config(dir.path()).unwrap();
        assert_eq!(config.agent, "claude-code");
        assert_eq!(config.headroom, 1500);
    }

    #[test]
    fn loads_agent_config() {
        let dir = setup_config_dir();
        let config = load_agent_config(dir.path(), "claude-code").unwrap();
        assert_eq!(config.models.get("work").unwrap(), "claude-sonnet-4-6");
    }

    #[test]
    fn loads_tokens() {
        let dir = setup_config_dir();
        let tokens = load_tokens(dir.path(), "claude-code").unwrap();
        assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
    }

    #[test]
    fn missing_config_errors() {
        let dir = TempDir::new().unwrap();
        assert!(load_shared_config(dir.path()).is_err());
    }

    // ---- select_config_dir ----

    #[test]
    fn explicit_flag_takes_precedence_over_env_and_default() {
        let explicit = TempDir::new().unwrap();
        let env = TempDir::new().unwrap();
        let default = TempDir::new().unwrap();

        let resolved = select_config_dir(
            Some(explicit.path().to_path_buf()),
            Some(env.path().to_path_buf()),
            Some(default.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(resolved, explicit.path());
    }

    #[test]
    fn env_takes_precedence_over_default_when_no_explicit() {
        let env = TempDir::new().unwrap();
        let default = TempDir::new().unwrap();

        let resolved = select_config_dir(
            None,
            Some(env.path().to_path_buf()),
            Some(default.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(resolved, env.path());
    }

    #[test]
    fn default_used_when_no_explicit_and_no_env() {
        let default = TempDir::new().unwrap();

        let resolved = select_config_dir(None, None, Some(default.path().to_path_buf())).unwrap();

        assert_eq!(resolved, default.path());
    }

    #[test]
    fn nonexistent_explicit_errors_with_candidate_path() {
        let missing = PathBuf::from("/definitely/not/a/real/path/for/ravel-lite/test");
        let err = select_config_dir(Some(missing.clone()), None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains(&missing.display().to_string()));
        assert!(message.contains("--config flag"));
        assert!(message.contains("ravel-lite init"));
    }

    #[test]
    fn nonexistent_env_errors_with_env_var_name() {
        let missing = PathBuf::from("/definitely/not/a/real/path/for/ravel-lite/test");
        let err = select_config_dir(None, Some(missing), None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("RAVEL_LITE_CONFIG"));
    }

    #[test]
    fn all_sources_missing_errors_with_init_guidance() {
        let err = select_config_dir(None, None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("ravel-lite init"));
        assert!(message.contains("RAVEL_LITE_CONFIG"));
    }

    #[test]
    fn candidate_that_is_a_file_errors() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("not-a-dir");
        fs::write(&file_path, "").unwrap();

        let err = select_config_dir(Some(file_path.clone()), None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("not a directory") || message.contains("does not exist"));
    }

    // ---- merge_yaml / overlay loader ----

    #[test]
    fn merge_yaml_overrides_scalar_at_root() {
        let mut base: serde_yaml::Value = serde_yaml::from_str("headroom: 1500\n").unwrap();
        let overlay: serde_yaml::Value = serde_yaml::from_str("headroom: 9000\n").unwrap();
        merge_yaml(&mut base, overlay);
        assert_eq!(base["headroom"].as_u64().unwrap(), 9000);
    }

    #[test]
    fn merge_yaml_keeps_base_keys_absent_from_overlay() {
        let mut base: serde_yaml::Value =
            serde_yaml::from_str("agent: claude-code\nheadroom: 1500\n").unwrap();
        let overlay: serde_yaml::Value = serde_yaml::from_str("headroom: 3000\n").unwrap();
        merge_yaml(&mut base, overlay);
        assert_eq!(base["agent"].as_str().unwrap(), "claude-code");
        assert_eq!(base["headroom"].as_u64().unwrap(), 3000);
    }

    #[test]
    fn merge_yaml_recurses_into_nested_maps() {
        // This is the load-bearing case: overriding `models.work` must not
        // wipe `models.reflect` / `models.dream` etc.
        let mut base: serde_yaml::Value = serde_yaml::from_str(
            "models:\n  work: claude-opus-4-6\n  reflect: claude-haiku-4-5\n",
        )
        .unwrap();
        let overlay: serde_yaml::Value =
            serde_yaml::from_str("models:\n  work: \"\"\n").unwrap();
        merge_yaml(&mut base, overlay);
        assert_eq!(base["models"]["work"].as_str().unwrap(), "");
        assert_eq!(
            base["models"]["reflect"].as_str().unwrap(),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn load_agent_config_without_overlay_uses_base() {
        let dir = setup_config_dir();
        let config = load_agent_config(dir.path(), "claude-code").unwrap();
        assert_eq!(config.models.get("work").unwrap(), "claude-sonnet-4-6");
    }

    #[test]
    fn load_agent_config_overlay_merges_into_base() {
        // The operational use case: the user wants `models.work` blanked
        // so `ClaudeCodeAgent` skips `--model` and lets Claude Code's
        // interactive default (e.g. the 1M-context variant) win, without
        // losing the other phase models and without fearing `init --force`
        // stomping the edit.
        let dir = setup_config_dir();
        let agent_dir = dir.path().join("agents/claude-code");
        fs::write(
            agent_dir.join("config.local.yaml"),
            "models:\n  work: \"\"\n",
        )
        .unwrap();

        let config = load_agent_config(dir.path(), "claude-code").unwrap();
        assert_eq!(config.models.get("work").unwrap(), "");
        assert_eq!(
            config.models.get("reflect").unwrap(),
            "claude-haiku-4-5",
            "keys only in the base config must survive the overlay"
        );
    }

    #[test]
    fn load_shared_config_overlay_overrides_agent_choice() {
        let dir = setup_config_dir();
        fs::write(dir.path().join("config.local.yaml"), "agent: pi\n").unwrap();
        let config = load_shared_config(dir.path()).unwrap();
        assert_eq!(config.agent, "pi");
        assert_eq!(config.headroom, 1500, "unrelated base keys must survive");
    }

    #[test]
    fn load_tokens_overlay_augments_and_overrides() {
        let dir = setup_config_dir();
        let agent_dir = dir.path().join("agents/claude-code");
        fs::write(
            agent_dir.join("tokens.local.yaml"),
            "TOOL_READ: CustomRead\nTOOL_NEW: NewTool\n",
        )
        .unwrap();

        let tokens = load_tokens(dir.path(), "claude-code").unwrap();
        assert_eq!(tokens.get("TOOL_READ").unwrap(), "CustomRead");
        assert_eq!(
            tokens.get("TOOL_WRITE").unwrap(),
            "Write",
            "base tokens must survive when overlay adds siblings"
        );
        assert_eq!(tokens.get("TOOL_NEW").unwrap(), "NewTool");
    }

    #[test]
    fn load_agent_config_shape_mismatched_overlay_surfaces_path_in_error() {
        // Overlay parses as YAML but collapses `models` from a map to a
        // scalar. Deserialization into `AgentConfig` then fails and the
        // surfaced error must name the overlay file so the user can
        // track the bad edit down.
        let dir = setup_config_dir();
        let agent_dir = dir.path().join("agents/claude-code");
        fs::write(agent_dir.join("config.local.yaml"), "models: not_a_map\n").unwrap();
        let err = load_agent_config(dir.path(), "claude-code").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("config.local.yaml"),
            "error should name the overlay file: {msg}"
        );
    }
}
