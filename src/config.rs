// src/config.rs
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::types::{AgentConfig, SharedConfig};

/// Environment variable that overrides the default config-directory location.
pub const CONFIG_ENV_VAR: &str = "RAVELOOP_CONFIG";

/// Resolve the Raveloop config directory using the precedence chain:
///   1. explicit `--config <path>` flag
///   2. `RAVELOOP_CONFIG` environment variable
///   3. XDG default at `<dirs::config_dir()>/raveloop/`
///   4. hard error (no walk-up, no magic, no registry)
///
/// The resolved path must be an existing directory; otherwise an
/// actionable error pointing at `raveloop init` is returned.
pub fn resolve_config_dir(explicit_flag: Option<PathBuf>) -> Result<PathBuf> {
    let env_var = std::env::var(CONFIG_ENV_VAR).ok().map(PathBuf::from);
    let xdg_default = dirs::config_dir().map(|p| p.join("raveloop"));
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
        ("default location (dirs::config_dir()/raveloop)".to_string(), path)
    } else {
        anyhow::bail!(
            "Could not resolve Raveloop config directory.\n\
             No --config flag, no RAVELOOP_CONFIG environment variable, and no user config dir available on this platform.\n\
             Create one with `raveloop init <dir>` and either pass --config <dir> or set RAVELOOP_CONFIG=<dir>."
        );
    };

    if !candidate.is_dir() {
        anyhow::bail!(
            "Raveloop config directory {} (from {}) does not exist or is not a directory.\n\
             Create it with `raveloop init {}`.",
            candidate.display(),
            source,
            candidate.display()
        );
    }

    Ok(candidate)
}

pub fn load_shared_config(config_root: &Path) -> Result<SharedConfig> {
    let path = config_root.join("config.yaml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))
}

pub fn load_agent_config(config_root: &Path, agent_name: &str) -> Result<AgentConfig> {
    let path = config_root.join("agents").join(agent_name).join("config.yaml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))
}

pub fn load_tokens(config_root: &Path, agent_name: &str) -> Result<std::collections::HashMap<String, String>> {
    let path = config_root.join("agents").join(agent_name).join("tokens.yaml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))
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
        let missing = PathBuf::from("/definitely/not/a/real/path/for/raveloop/test");
        let err = select_config_dir(Some(missing.clone()), None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains(&missing.display().to_string()));
        assert!(message.contains("--config flag"));
        assert!(message.contains("raveloop init"));
    }

    #[test]
    fn nonexistent_env_errors_with_env_var_name() {
        let missing = PathBuf::from("/definitely/not/a/real/path/for/raveloop/test");
        let err = select_config_dir(None, Some(missing), None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("RAVELOOP_CONFIG"));
    }

    #[test]
    fn all_sources_missing_errors_with_init_guidance() {
        let err = select_config_dir(None, None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("raveloop init"));
        assert!(message.contains("RAVELOOP_CONFIG"));
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
}
