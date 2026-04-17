// src/types.rs
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

/// LLM phases — the agent subprocess runs these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmPhase {
    Work,
    AnalyseWork,
    Reflect,
    Dream,
    Triage,
}

impl LlmPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::AnalyseWork => "analyse-work",
            Self::Reflect => "reflect",
            Self::Dream => "dream",
            Self::Triage => "triage",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "work" => Some(Self::Work),
            "analyse-work" => Some(Self::AnalyseWork),
            "reflect" => Some(Self::Reflect),
            "dream" => Some(Self::Dream),
            "triage" => Some(Self::Triage),
            _ => None,
        }
    }
}

impl fmt::Display for LlmPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Script phases — handled inline by the orchestrator (git commits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptPhase {
    GitCommitWork,
    GitCommitReflect,
    GitCommitDream,
    GitCommitTriage,
}

impl ScriptPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitCommitWork => "git-commit-work",
            Self::GitCommitReflect => "git-commit-reflect",
            Self::GitCommitDream => "git-commit-dream",
            Self::GitCommitTriage => "git-commit-triage",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "git-commit-work" => Some(Self::GitCommitWork),
            "git-commit-reflect" => Some(Self::GitCommitReflect),
            "git-commit-dream" => Some(Self::GitCommitDream),
            "git-commit-triage" => Some(Self::GitCommitTriage),
            _ => None,
        }
    }
}

impl fmt::Display for ScriptPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A phase is either an LLM phase or a script phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Llm(LlmPhase),
    Script(ScriptPhase),
}

impl Phase {
    pub fn parse(s: &str) -> Option<Self> {
        LlmPhase::from_str(s)
            .map(Phase::Llm)
            .or_else(|| ScriptPhase::from_str(s).map(Phase::Script))
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Phase::Llm(p) => write!(f, "{p}"),
            Phase::Script(p) => write!(f, "{p}"),
        }
    }
}

/// Context for a plan execution.
#[derive(Debug, Clone)]
pub struct PlanContext {
    pub plan_dir: String,
    pub project_dir: String,
    pub dev_root: String,
    pub related_plans: String,
    pub config_root: String,
}

/// Top-level shared config (config.yaml).
#[derive(Debug, Deserialize)]
pub struct SharedConfig {
    pub agent: String,
    pub headroom: usize,
}

/// Per-agent config (agents/<name>/config.yaml).
#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    pub models: HashMap<String, String>,
    #[serde(default)]
    pub thinking: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, HashMap<String, serde_yaml::Value>>,
    pub provider: Option<String>,
}

/// A subagent dispatch entry from subagent-dispatch.yaml.
#[derive(Debug, Deserialize)]
pub struct SubagentDispatch {
    pub target: String,
    pub kind: String,
    pub summary: String,
}

/// Status bar information.
#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub agent: String,
    pub cycle: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_llm_phases() {
        assert_eq!(Phase::parse("work"), Some(Phase::Llm(LlmPhase::Work)));
        assert_eq!(Phase::parse("analyse-work"), Some(Phase::Llm(LlmPhase::AnalyseWork)));
        assert_eq!(Phase::parse("reflect"), Some(Phase::Llm(LlmPhase::Reflect)));
        assert_eq!(Phase::parse("dream"), Some(Phase::Llm(LlmPhase::Dream)));
        assert_eq!(Phase::parse("triage"), Some(Phase::Llm(LlmPhase::Triage)));
    }

    #[test]
    fn parse_script_phases() {
        assert_eq!(Phase::parse("git-commit-work"), Some(Phase::Script(ScriptPhase::GitCommitWork)));
        assert_eq!(Phase::parse("git-commit-reflect"), Some(Phase::Script(ScriptPhase::GitCommitReflect)));
        assert_eq!(Phase::parse("git-commit-dream"), Some(Phase::Script(ScriptPhase::GitCommitDream)));
        assert_eq!(Phase::parse("git-commit-triage"), Some(Phase::Script(ScriptPhase::GitCommitTriage)));
    }

    #[test]
    fn parse_invalid_phase() {
        assert_eq!(Phase::parse("invalid"), None);
        assert_eq!(Phase::parse(""), None);
    }

    #[test]
    fn phase_display() {
        assert_eq!(Phase::Llm(LlmPhase::AnalyseWork).to_string(), "analyse-work");
        assert_eq!(Phase::Script(ScriptPhase::GitCommitReflect).to_string(), "git-commit-reflect");
    }
}
