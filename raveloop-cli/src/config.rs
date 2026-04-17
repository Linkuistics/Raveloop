// src/config.rs
use std::path::Path;

use anyhow::{Context, Result};

use crate::types::{AgentConfig, SharedConfig};

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
}
