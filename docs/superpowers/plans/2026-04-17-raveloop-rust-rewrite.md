# Raveloop Rust Rewrite — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the llm-context orchestrator as a Rust binary (`raveloop-cli`) with a Ratatui TUI, producing a single executable with zero runtime dependencies.

**Architecture:** Event-driven rendering — agents spawn CLI subprocesses, parse JSON streams, and emit `UIMessage` events through an mpsc channel. A Ratatui TUI receives all messages and renders a three-zone layout (scrolling log, live progress groups, status bar). The phase loop drives the state machine imperatively through a `UI` handle.

**Tech Stack:** Rust, tokio (async runtime), ratatui + crossterm (TUI), serde + serde_json + serde_yaml (serialization), clap (CLI), anyhow (error handling)

**Spec:** `docs/superpowers/specs/2026-04-16-rust-ratatui-rewrite-design.md`

**Existing TypeScript source:** Read the TS files in `src/` for exact logic to port. Every function has a direct Rust equivalent.

---

## File Structure

```
raveloop-cli/
├── Cargo.toml
├── defaults/                    # embedded at compile time, written by init
│   ├── config.yaml
│   ├── raveloop.sh              # trampoline template
│   ├── agents/
│   │   ├── claude-code/
│   │   │   ├── config.yaml
│   │   │   └── tokens.yaml
│   │   └── pi/
│   │       ├── config.yaml
│   │       ├── tokens.yaml
│   │       └── prompts/
│   │           ├── system-prompt.md
│   │           └── memory-prompt.md
│   ├── phases/
│   │   ├── work.md
│   │   ├── analyse-work.md
│   │   ├── reflect.md
│   │   ├── dream.md
│   │   └── triage.md
│   ├── fixed-memory/
│   │   ├── coding-style.md
│   │   ├── coding-style-rust.md
│   │   └── memory-style.md
│   └── skills/
│       ├── brainstorming.md
│       ├── tdd.md
│       └── writing-plans.md
├── src/
│   ├── main.rs                  # CLI entry point (clap)
│   ├── types.rs                 # Enums, structs, phase logic
│   ├── config.rs                # YAML config loading
│   ├── dream.rs                 # Dream guard (should_dream, update_baseline)
│   ├── git.rs                   # Git commit, baseline save
│   ├── prompt.rs                # Template loading + token substitution
│   ├── format.rs                # Pure formatting functions
│   ├── ui.rs                    # Ratatui TUI + UI handle
│   ├── phase_loop.rs            # Phase state machine
│   ├── subagent.rs              # Concurrent subagent dispatch
│   ├── init.rs                  # Init command (embed + write defaults)
│   └── agent/
│       ├── mod.rs               # Agent trait
│       ├── claude_code.rs       # Claude Code agent + stream parser
│       └── pi.rs                # Pi agent + stream parser + setup
└── tests/
    └── integration.rs           # End-to-end phase transition tests
```

---

### Task 1: Project Scaffold and Types

**Files:**
- Create: `raveloop-cli/Cargo.toml`
- Create: `raveloop-cli/src/main.rs`
- Create: `raveloop-cli/src/types.rs`

- [ ] **Step 1: Create the Cargo project**

```bash
cd /Users/antony/Development
cargo init raveloop-cli
cd raveloop-cli
```

- [ ] **Step 2: Write Cargo.toml with all dependencies**

Replace `Cargo.toml` with:

```toml
[package]
name = "raveloop-cli"
version = "0.1.0"
edition = "2021"
description = "An orchestration loop for LLM development cycles"

[dependencies]
tokio = { version = "1", features = ["full"] }
ratatui = "0.29"
crossterm = "0.28"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
indexmap = { version = "2", features = ["serde"] }
anyhow = "1"
async-trait = "0.1"
clap = { version = "4", features = ["derive"] }
regex = "1"
once_cell = "1"
dirs = "5"
```

- [ ] **Step 3: Write src/types.rs with all core types**

Port from the TypeScript `src/types.ts`. This is the foundation — every other module depends on it.

```rust
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
```

- [ ] **Step 4: Write a minimal main.rs stub**

```rust
// src/main.rs
mod types;

fn main() {
    println!("raveloop-cli");
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 6: Add unit tests for Phase parsing**

Add at the bottom of `src/types.rs`:

```rust
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
```

- [ ] **Step 7: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: project scaffold with types module"
```

---

### Task 2: Config Loading

**Files:**
- Create: `raveloop-cli/src/config.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/config.ts`. Loads YAML config files from the config root directory.

- [ ] **Step 1: Write failing tests**

```rust
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
```

- [ ] **Step 2: Add tempfile dev-dependency to Cargo.toml**

Add to `Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Add `mod config;` to main.rs**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config`
Expected: All 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: config loading from YAML files"
```

---

### Task 3: Dream Guard

**Files:**
- Create: `raveloop-cli/src/dream.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/dream.ts`. Pure file I/O — reads memory.md word count and compares to baseline.

- [ ] **Step 1: Write implementation and tests**

```rust
// src/dream.rs
use std::fs;
use std::path::Path;

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Returns true if memory.md has grown beyond baseline + headroom.
pub fn should_dream(plan_dir: &Path, headroom: usize) -> bool {
    let memory_path = plan_dir.join("memory.md");
    let baseline_path = plan_dir.join("dream-baseline");

    let Ok(memory) = fs::read_to_string(&memory_path) else {
        return false;
    };
    let Ok(baseline_str) = fs::read_to_string(&baseline_path) else {
        return false;
    };
    let Ok(baseline) = baseline_str.trim().parse::<usize>() else {
        return false;
    };

    word_count(&memory) > baseline + headroom
}

/// Update the dream baseline to the current word count of memory.md.
pub fn update_dream_baseline(plan_dir: &Path) {
    let memory_path = plan_dir.join("memory.md");
    let baseline_path = plan_dir.join("dream-baseline");

    if let Ok(memory) = fs::read_to_string(&memory_path) {
        let count = word_count(&memory);
        let _ = fs::write(&baseline_path, count.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn returns_false_when_no_memory() {
        let dir = TempDir::new().unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_when_no_baseline() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "hello world").unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_within_headroom() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(100)).unwrap();
        fs::write(dir.path().join("dream-baseline"), "50").unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_true_beyond_headroom() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(2000)).unwrap();
        fs::write(dir.path().join("dream-baseline"), "100").unwrap();
        assert!(should_dream(dir.path(), 1500));
    }

    #[test]
    fn update_baseline_writes_word_count() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(500)).unwrap();
        update_dream_baseline(dir.path());
        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim().parse::<usize>().unwrap(), 500);
    }
}
```

- [ ] **Step 2: Add `mod dream;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test dream`
Expected: All 5 tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: dream guard — should_dream and update_baseline"
```

---

### Task 4: Git Operations

**Files:**
- Create: `raveloop-cli/src/git.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/git.ts`. Runs git commands via `std::process::Command`.

- [ ] **Step 1: Write implementation**

```rust
// src/git.rs
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub struct CommitResult {
    pub committed: bool,
    pub message: String,
}

/// Stage plan directory and commit with the message from commit-message.md
/// (or a default message). Returns whether anything was committed.
pub fn git_commit_plan(plan_dir: &Path, plan_name: &str, phase_name: &str) -> Result<CommitResult> {
    let commit_msg_path = plan_dir.join("commit-message.md");
    let message = if commit_msg_path.exists() {
        let msg = fs::read_to_string(&commit_msg_path)
            .context("Failed to read commit-message.md")?
            .trim()
            .to_string();
        fs::remove_file(&commit_msg_path).ok();
        msg
    } else {
        format!("run-plan: {phase_name} ({plan_name})")
    };

    // Stage the plan directory
    Command::new("git")
        .args(["add", &plan_dir.to_string_lossy()])
        .output()
        .context("Failed to run git add")?;

    // Check if there are staged changes
    let diff = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()
        .context("Failed to run git diff")?;

    if diff.status.success() {
        // Exit code 0 means no changes
        return Ok(CommitResult {
            committed: false,
            message,
        });
    }

    // Commit
    Command::new("git")
        .args(["commit", "-m", &message])
        .output()
        .context("Failed to run git commit")?;

    Ok(CommitResult {
        committed: true,
        message,
    })
}

/// Save the current HEAD sha as the work baseline.
pub fn git_save_work_baseline(plan_dir: &Path) {
    let baseline_path = plan_dir.join("work-baseline");
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    let _ = fs::write(&baseline_path, &sha);
}

/// Find the project root by walking up from a directory to find .git.
pub fn find_project_root(start_dir: &Path) -> Result<String> {
    let mut dir = start_dir.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_string_lossy().to_string());
        }
        if !dir.pop() {
            anyhow::bail!("No .git found above {}", start_dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_project_root_finds_git() {
        // This test runs inside a git repo (the raveloop-cli project itself)
        let result = find_project_root(Path::new("."));
        assert!(result.is_ok());
    }

    #[test]
    fn find_project_root_errors_on_root() {
        let result = find_project_root(Path::new("/tmp/nonexistent-asdhjkasd"));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Add `mod git;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test git`
Expected: Both tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: git operations — commit, baseline, project root discovery"
```

---

### Task 5: Prompt Composition

**Files:**
- Create: `raveloop-cli/src/prompt.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/prompt-composer.ts`. Loads phase prompt files, applies token substitution.

- [ ] **Step 1: Write implementation and tests**

```rust
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
```

- [ ] **Step 2: Add `mod prompt;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test prompt`
Expected: All 3 tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: prompt composition with token substitution"
```

---

### Task 6: Formatting

**Files:**
- Create: `raveloop-cli/src/format.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/format.ts`. The largest pure function module — handles tool call formatting, result text parsing (action markers, insight blocks), phase highlights, and tool name cleaning. All functions are pure with no terminal I/O.

- [ ] **Step 1: Write the formatting module**

```rust
// src/format.rs
use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::LlmPhase;

/// Formatted output from tool call or result parsing.
pub struct FormattedOutput {
    pub text: String,
    pub persist: bool,
}

/// A tool call to format for display.
pub struct ToolCall {
    pub name: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub edit_context: Option<String>,
}

struct HighlightRule {
    pattern: Regex,
    label: &'static str,
}

struct ActionStyle {
    colour: &'static str,
}

const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

static PHASE_HIGHLIGHTS: Lazy<HashMap<LlmPhase, Vec<HighlightRule>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert(LlmPhase::AnalyseWork, vec![
        HighlightRule { pattern: Regex::new(r"latest-session\.md$").unwrap(), label: "Writing session log" },
        HighlightRule { pattern: Regex::new(r"commit-message\.md$").unwrap(), label: "Writing commit message" },
    ]);
    m.insert(LlmPhase::Reflect, vec![
        HighlightRule { pattern: Regex::new(r"memory\.md$").unwrap(), label: "Updating memory" },
    ]);
    m.insert(LlmPhase::Dream, vec![
        HighlightRule { pattern: Regex::new(r"memory\.md$").unwrap(), label: "Rewriting memory" },
    ]);
    m.insert(LlmPhase::Triage, vec![
        HighlightRule { pattern: Regex::new(r"backlog\.md$").unwrap(), label: "Updating backlog" },
        HighlightRule { pattern: Regex::new(r"subagent-dispatch\.yaml$").unwrap(), label: "Dispatching to related plans" },
    ]);
    m
});

static ACTION_STYLES: Lazy<HashMap<&'static str, ActionStyle>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("ADDED", ActionStyle { colour: GREEN });
    m.insert("SHARPENED", ActionStyle { colour: CYAN });
    m.insert("REPLACED", ActionStyle { colour: YELLOW });
    m.insert("REMOVED", ActionStyle { colour: RED });
    m.insert("MERGED", ActionStyle { colour: CYAN });
    m.insert("TIGHTENED", ActionStyle { colour: CYAN });
    m.insert("REWORDED", ActionStyle { colour: DIM });
    m.insert("STATS", ActionStyle { colour: DIM });
    m.insert("DELETED", ActionStyle { colour: RED });
    m.insert("PROMOTED", ActionStyle { colour: YELLOW });
    m.insert("REPRIORITISED", ActionStyle { colour: YELLOW });
    m.insert("DISPATCH", ActionStyle { colour: GREEN });
    m.insert("NO DISPATCH", ActionStyle { colour: DIM });
    m
});

static LABEL_WIDTH: Lazy<usize> = Lazy::new(|| {
    ACTION_STYLES.keys().map(|k| k.len()).max().unwrap_or(0)
});

pub struct PhaseInfo {
    pub label: &'static str,
    pub description: &'static str,
}

pub fn phase_info(phase: LlmPhase) -> PhaseInfo {
    match phase {
        LlmPhase::Work => PhaseInfo { label: "WORK", description: "Pick a task, implement it, record results" },
        LlmPhase::AnalyseWork => PhaseInfo { label: "ANALYSE", description: "Examine git diff, write session log and commit message" },
        LlmPhase::Reflect => PhaseInfo { label: "REFLECT", description: "Distil session learnings into durable memory" },
        LlmPhase::Dream => PhaseInfo { label: "DREAM", description: "Rewrite memory losslessly in tighter form" },
        LlmPhase::Triage => PhaseInfo { label: "TRIAGE", description: "Reprioritise backlog, propagate to related plans" },
    }
}

/// Format a tool call for display. Returns persist=true for highlight matches.
/// `shown_highlights` tracks which labels have already been emitted this phase.
pub fn format_tool_call(
    tool: &ToolCall,
    phase: Option<LlmPhase>,
    shown_highlights: &mut std::collections::HashSet<String>,
) -> FormattedOutput {
    let is_write = matches!(
        tool.name.to_lowercase().as_str(),
        "write" | "edit"
    );
    let is_bash_write = tool.name.to_lowercase() == "bash"
        && tool.detail.as_deref().map_or(false, |d| {
            d.contains("cat ") && d.contains("> ") || d.contains("echo ") && d.contains("> ")
        });

    if (is_write || is_bash_write) {
        if let Some(phase) = phase {
            let path_to_check = tool.path.as_deref()
                .or(tool.detail.as_deref())
                .unwrap_or("");

            if let Some(rules) = PHASE_HIGHLIGHTS.get(&phase) {
                for rule in rules {
                    if rule.pattern.is_match(path_to_check) {
                        if shown_highlights.contains(rule.label) {
                            return FormattedOutput { text: String::new(), persist: false };
                        }
                        shown_highlights.insert(rule.label.to_string());
                        return FormattedOutput {
                            text: format!("  {BOLD}{GREEN}★  {}{RESET}", rule.label),
                            persist: true,
                        };
                    }
                }
            }

            // Silently skip phase.md writes
            if path_to_check.contains("phase.md") {
                return FormattedOutput { text: String::new(), persist: false };
            }
        }
    }

    // Strip newlines from detail
    let desc = tool.detail.as_deref()
        .or(tool.path.as_deref())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");

    FormattedOutput {
        text: format!("{DIM}  ·  {} {desc}{RESET}", tool.name),
        persist: false,
    }
}

/// Format result text from a headless phase.
/// Recognises [ACTION] markers and Insight blocks.
pub fn format_result_text(text: &str) -> String {
    static ACTION_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[\s\-\*]*\[([A-Za-z ]+)\]\s*(.*)$").unwrap()
    });
    static PHASE_MD_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(?:^(?:`?phase\.md`?|Phase)\s+(?:set to|written|→))|(?:phase\.md.*`git-commit-)|(?:wrote.*phase\.md)").unwrap()
    });

    let mut formatted = vec![String::new()]; // blank line separates progress from result
    let mut in_insight = false;

    for line in text.lines() {
        // Filter phase.md status lines
        if PHASE_MD_RE.is_match(line) { continue; }
        // Filter code fence lines
        if line.trim() == "```" { continue; }

        // Structured action markers
        if let Some(caps) = ACTION_RE.captures(line) {
            let tag = caps[1].to_uppercase();
            let detail = &caps[2];
            if let Some(style) = ACTION_STYLES.get(tag.as_str()) {
                let padded = format!("{:<width$}", tag, width = *LABEL_WIDTH);
                formatted.push(format!(
                    "  {}{BOLD}{padded}{RESET}  {}{detail}{RESET}",
                    style.colour, style.colour
                ));
                continue;
            }
        }

        // Insight block opening
        if line.contains("★") && line.contains("Insight") && line.contains("─") {
            in_insight = true;
            formatted.push(format!("  {BOLD}{CYAN}★ Insight{RESET}"));
            continue;
        }
        // Insight block closing
        if in_insight && line.chars().filter(|c| *c == '─').count() >= 10 {
            in_insight = false;
            continue;
        }
        // Insight content or regular text — indent
        formatted.push(format!("  {DIM}{line}{RESET}"));
    }

    // Trim trailing blank lines
    while formatted.len() > 1 && formatted.last().map_or(false, |l| l.trim().is_empty()) {
        formatted.pop();
    }

    formatted.join("\n")
}

/// Extract a brief context string from Edit old/new strings.
pub fn extract_edit_context(old: Option<&str>, new: Option<&str>) -> Option<String> {
    let source = new.or(old)?;
    // Look for markdown headings
    static HEADING_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#{1,4}\s+(.{1,60})").unwrap());
    if let Some(caps) = HEADING_RE.captures(source) {
        return Some(caps[1].trim().to_string());
    }
    // Look for bold text
    static BOLD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*(.{1,60}?)\*\*").unwrap());
    if let Some(caps) = BOLD_RE.captures(source) {
        return Some(caps[1].trim().to_string());
    }
    None
}

/// Clean up a tool name — strip MCP prefixes.
pub fn clean_tool_name(name: &str) -> String {
    static MCP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^mcp__[^_]+(?:__)?(.+)$").unwrap());
    if let Some(caps) = MCP_RE.captures(name) {
        return caps[1].to_string();
    }
    name.to_string()
}

/// Extract a meaningful detail string from tool input parameters.
pub fn extract_tool_detail(input: &serde_json::Value) -> String {
    const DETAIL_KEYS: &[&str] = &["path", "file_path", "command", "pattern", "query", "url", "prompt", "mode"];
    if let Some(obj) = input.as_object() {
        for key in DETAIL_KEYS {
            if let Some(serde_json::Value::String(val)) = obj.get(*key) {
                if !val.is_empty() {
                    return if val.len() > 80 { format!("{}...", &val[..77]) } else { val.clone() };
                }
            }
        }
        // Fallback: first string value
        for val in obj.values() {
            if let serde_json::Value::String(s) = val {
                if !s.is_empty() {
                    return if s.len() > 60 { format!("{}...", &s[..57]) } else { s.clone() };
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn format_tool_call_highlight_memory() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.persist);
        assert!(result.text.contains("Updating memory"));
    }

    #[test]
    fn format_tool_call_deduplicates_highlights() {
        let mut shown = HashSet::new();
        let r1 = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(r1.persist);

        let r2 = format_tool_call(
            &ToolCall { name: "Edit".to_string(), path: Some("/plan/memory.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(!r2.persist);
        assert!(r2.text.is_empty());
    }

    #[test]
    fn format_tool_call_skips_phase_md() {
        let mut shown = HashSet::new();
        let result = format_tool_call(
            &ToolCall { name: "Write".to_string(), path: Some("/plan/phase.md".to_string()), detail: None, edit_context: None },
            Some(LlmPhase::Reflect),
            &mut shown,
        );
        assert!(result.text.is_empty());
    }

    #[test]
    fn format_result_text_recognises_action_markers() {
        let text = "[ADDED] New entry — description\n[REMOVED] Old entry — stale";
        let result = format_result_text(text);
        assert!(result.contains("ADDED"));
        assert!(result.contains("REMOVED"));
        assert!(result.contains("description"));
    }

    #[test]
    fn format_result_text_filters_phase_md() {
        let text = "phase.md set to git-commit-reflect\n[ADDED] Real content";
        let result = format_result_text(text);
        assert!(!result.contains("phase.md set to"));
        assert!(result.contains("Real content"));
    }

    #[test]
    fn clean_tool_name_strips_mcp_prefix() {
        assert_eq!(clean_tool_name("mcp__server__tool_name"), "tool_name");
        assert_eq!(clean_tool_name("Read"), "Read");
    }

    #[test]
    fn extract_edit_context_finds_headings() {
        assert_eq!(
            extract_edit_context(None, Some("## My Heading\nsome content")),
            Some("My Heading".to_string())
        );
    }

    #[test]
    fn extract_edit_context_finds_bold() {
        assert_eq!(
            extract_edit_context(None, Some("update **Important Thing** here")),
            Some("Important Thing".to_string())
        );
    }
}
```

- [ ] **Step 2: Add `mod format;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test format`
Expected: All 8 tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: formatting module — tool calls, result text, highlights"
```

---

### Task 7: UI Handle and Message Types

**Files:**
- Create: `raveloop-cli/src/ui.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

The `UI` struct wraps an `mpsc::UnboundedSender<UIMessage>` and provides ergonomic methods. The TUI rendering will be added in Task 12. This task sets up the channel types and the handle.

- [ ] **Step 1: Write the UI handle**

```rust
// src/ui.rs
use indexmap::IndexMap;
use tokio::sync::{mpsc, oneshot};

use crate::types::StatusInfo;

/// All messages to the TUI flow through this enum.
pub enum UIMessage {
    // From agents
    Progress { agent_id: String, text: String },
    Persist { agent_id: String, text: String },
    AgentDone { agent_id: String },

    // From the phase loop
    Log(String),
    RegisterAgent { agent_id: String, header: String },
    SetStatus(StatusInfo),
    Confirm { message: String, reply: oneshot::Sender<bool> },
    Suspend,
    Resume,
    Quit,
}

pub type UISender = mpsc::UnboundedSender<UIMessage>;

/// Handle for sending messages to the TUI.
/// Cloneable — the phase loop and agents all hold copies.
#[derive(Clone)]
pub struct UI {
    tx: UISender,
}

impl UI {
    pub fn new(tx: UISender) -> Self {
        Self { tx }
    }

    pub fn sender(&self) -> UISender {
        self.tx.clone()
    }

    pub fn log(&self, text: &str) {
        let _ = self.tx.send(UIMessage::Log(text.to_string()));
    }

    pub fn register_agent(&self, agent_id: &str, header: &str) {
        let _ = self.tx.send(UIMessage::RegisterAgent {
            agent_id: agent_id.to_string(),
            header: header.to_string(),
        });
    }

    pub fn clear_agent(&self, agent_id: &str) {
        let _ = self.tx.send(UIMessage::AgentDone {
            agent_id: agent_id.to_string(),
        });
    }

    pub fn set_status(&self, status: StatusInfo) {
        let _ = self.tx.send(UIMessage::SetStatus(status));
    }

    pub async fn confirm(&self, message: &str) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(UIMessage::Confirm {
            message: message.to_string(),
            reply: reply_tx,
        });
        reply_rx.await.unwrap_or(false)
    }

    pub fn suspend(&self) {
        let _ = self.tx.send(UIMessage::Suspend);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(UIMessage::Resume);
    }

    pub fn quit(&self) {
        let _ = self.tx.send(UIMessage::Quit);
    }
}

/// State for the TUI — used by the renderer (Task 12).
pub struct AppState {
    pub log_lines: Vec<String>,
    pub progress_groups: IndexMap<String, AgentProgress>,
    pub status: Option<StatusInfo>,
    pub confirm_prompt: Option<ConfirmState>,
}

pub struct AgentProgress {
    pub header: String,
    pub progress: Option<String>,
}

pub struct ConfirmState {
    pub message: String,
    pub reply: Option<oneshot::Sender<bool>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            log_lines: Vec::new(),
            progress_groups: IndexMap::new(),
            status: None,
            confirm_prompt: None,
        }
    }

    /// Process a UIMessage, updating state accordingly.
    pub fn handle_message(&mut self, msg: UIMessage) {
        match msg {
            UIMessage::Log(text) => {
                // Split multi-line log entries into individual lines
                for line in text.lines() {
                    self.log_lines.push(line.to_string());
                }
            }
            UIMessage::RegisterAgent { agent_id, header } => {
                self.progress_groups.insert(agent_id, AgentProgress {
                    header,
                    progress: None,
                });
            }
            UIMessage::Progress { agent_id, text } => {
                if let Some(group) = self.progress_groups.get_mut(&agent_id) {
                    group.progress = Some(text);
                }
            }
            UIMessage::Persist { agent_id: _, text } => {
                for line in text.lines() {
                    self.log_lines.push(line.to_string());
                }
            }
            UIMessage::AgentDone { agent_id } => {
                self.progress_groups.shift_remove(&agent_id);
            }
            UIMessage::SetStatus(status) => {
                self.status = Some(status);
            }
            UIMessage::Confirm { message, reply } => {
                self.confirm_prompt = Some(ConfirmState {
                    message,
                    reply: Some(reply),
                });
            }
            UIMessage::Suspend | UIMessage::Resume | UIMessage::Quit => {
                // Handled by the TUI event loop, not state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_handles_log() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::Log("hello\nworld".to_string()));
        assert_eq!(state.log_lines, vec!["hello", "world"]);
    }

    #[test]
    fn state_handles_agent_lifecycle() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::RegisterAgent {
            agent_id: "sub-B".to_string(),
            header: "→ child: sub-B".to_string(),
        });
        assert!(state.progress_groups.contains_key("sub-B"));

        state.handle_message(UIMessage::Progress {
            agent_id: "sub-B".to_string(),
            text: "Read memory.md".to_string(),
        });
        assert_eq!(
            state.progress_groups.get("sub-B").unwrap().progress.as_deref(),
            Some("Read memory.md")
        );

        state.handle_message(UIMessage::AgentDone {
            agent_id: "sub-B".to_string(),
        });
        assert!(!state.progress_groups.contains_key("sub-B"));
    }

    #[test]
    fn state_handles_persist() {
        let mut state = AppState::new();
        state.handle_message(UIMessage::Persist {
            agent_id: "main".to_string(),
            text: "★ Updating memory".to_string(),
        });
        assert_eq!(state.log_lines, vec!["★ Updating memory"]);
    }

    #[tokio::test]
    async fn ui_confirm_roundtrip() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ui = UI::new(tx);

        let confirm_handle = tokio::spawn(async move {
            ui.confirm("Proceed?").await
        });

        // Receive the confirm message and reply
        if let Some(UIMessage::Confirm { reply, .. }) = rx.recv().await {
            reply.send(true).unwrap();
        }

        assert!(confirm_handle.await.unwrap());
    }
}
```

- [ ] **Step 2: Add `mod ui;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test ui`
Expected: All 4 tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: UI handle and AppState for TUI message routing"
```

---

### Task 8: Agent Trait and Claude Code Agent

**Files:**
- Create: `raveloop-cli/src/agent/mod.rs`
- Create: `raveloop-cli/src/agent/claude_code.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/agents/claude-code/index.ts` and `stream-parser.ts`. The agent spawns `claude` as a subprocess and parses its stream-json output.

- [ ] **Step 1: Write the Agent trait**

```rust
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
```

- [ ] **Step 2: Write the Claude Code agent**

Port from `src/agents/claude-code/index.ts` and `stream-parser.ts`.

```rust
// src/agent/claude_code.rs
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::Agent;
use crate::config::load_tokens;
use crate::format::{
    self, FormattedOutput, ToolCall, clean_tool_name, extract_edit_context,
    extract_tool_detail, format_result_text, format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::{UIMessage, UISender};

pub struct ClaudeCodeAgent {
    config: AgentConfig,
    config_root: String,
}

impl ClaudeCodeAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn build_headless_args(&self, prompt: &str, phase: LlmPhase) -> Vec<String> {
        let mut args = vec![
            "--strict-mcp-config".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];

        if let Some(model) = self.config.models.get(phase.as_str()) {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        // Apply per-phase params
        if let Some(params) = self.config.params.get(phase.as_str()) {
            if params.get("dangerous").and_then(|v| v.as_bool()) == Some(true) {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }

        args
    }
}

/// Parse a single line of Claude's stream-json output.
fn parse_stream_line(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> Option<FormattedOutput> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let event: serde_json::Value = serde_json::from_str(line).ok()?;

    let event_type = event.get("type")?.as_str()?;

    // Assistant messages: extract tool_use blocks
    if event_type == "assistant" {
        if let Some(content) = event.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                    continue;
                }
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);

                let tool = match name {
                    "Read" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                        edit_context: None,
                    },
                    "Write" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                        edit_context: extract_edit_context(None, input.get("content").and_then(|v| v.as_str())),
                    },
                    "Edit" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                        edit_context: extract_edit_context(
                            input.get("old_string").and_then(|v| v.as_str()),
                            input.get("new_string").and_then(|v| v.as_str()),
                        ),
                    },
                    "Grep" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: Some(format!(
                            "\"{}\" in {}",
                            input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                            input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                        )),
                        edit_context: None,
                    },
                    "Glob" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("pattern").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        edit_context: None,
                    },
                    "Bash" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("command").and_then(|v| v.as_str()).map(|s| s.chars().take(120).collect()),
                        edit_context: None,
                    },
                    _ => ToolCall {
                        name: clean_tool_name(name),
                        path: None,
                        detail: Some(extract_tool_detail(&input)),
                        edit_context: None,
                    },
                };

                return Some(format_tool_call(&tool, phase, shown_highlights));
            }
        }
        return None;
    }

    // Final result
    if event_type == "result" {
        if let Some(result_text) = event.get("result").and_then(|r| r.as_str()) {
            return Some(FormattedOutput {
                text: format_result_text(result_text),
                persist: true,
            });
        }
    }

    None
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()> {
        let mut args = vec!["--output-format".to_string(), "stream-json".to_string()];

        if let Some(model) = self.config.models.get("work") {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if let Some(params) = self.config.params.get("work") {
            if params.get("dangerous").and_then(|v| v.as_bool()) == Some(true) {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }

        args.push(prompt.to_string());

        let status = std::process::Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn claude")?;

        if !status.success() {
            anyhow::bail!("claude exited with code {:?}", status.code());
        }
        Ok(())
    }

    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let args = self.build_headless_args(prompt, phase);

        let mut child = Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn claude")?;

        let stdout = child.stdout.take().context("No stdout")?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut shown_highlights = HashSet::new();

        while let Some(line) = lines.next_line().await? {
            if let Some(formatted) = parse_stream_line(&line, Some(phase), &mut shown_highlights) {
                if formatted.text.is_empty() {
                    continue;
                }
                if formatted.persist {
                    let _ = tx.send(UIMessage::Persist {
                        agent_id: agent_id.to_string(),
                        text: formatted.text,
                    });
                } else {
                    let _ = tx.send(UIMessage::Progress {
                        agent_id: agent_id.to_string(),
                        text: formatted.text,
                    });
                }
            }
        }

        let status = child.wait().await?;
        let _ = tx.send(UIMessage::AgentDone {
            agent_id: agent_id.to_string(),
        });

        if !status.success() {
            anyhow::bail!("claude exited with code {:?}", status.code());
        }
        Ok(())
    }

    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let project_dir = crate::git::find_project_root(Path::new(target_plan))?;

        let ctx = PlanContext {
            plan_dir: target_plan.to_string(),
            project_dir,
            dev_root: Path::new(target_plan)
                .parent().and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            related_plans: String::new(),
            config_root: self.config_root.clone(),
        };

        self.invoke_headless(prompt, &ctx, LlmPhase::Triage, agent_id, tx).await
    }

    fn tokens(&self) -> HashMap<String, String> {
        load_tokens(Path::new(&self.config_root), "claude-code")
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_use_read() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/foo/bar.md"}}]}}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(!formatted.persist);
        assert!(formatted.text.contains("Read"));
        assert!(formatted.text.contains("/foo/bar.md"));
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","result":"[ADDED] New entry — description"}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(formatted.persist);
        assert!(formatted.text.contains("ADDED"));
    }

    #[test]
    fn parse_highlight_write_memory() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/plan/memory.md","content":"stuff"}}]}}"#;
        let mut shown = HashSet::new();
        let result = parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown);
        assert!(result.is_some());
        assert!(result.unwrap().persist); // highlight
    }

    #[test]
    fn parse_ignores_empty_lines() {
        let mut shown = HashSet::new();
        assert!(parse_stream_line("", None, &mut shown).is_none());
        assert!(parse_stream_line("   ", None, &mut shown).is_none());
    }
}
```

- [ ] **Step 3: Create `src/agent/` directory and add `mod agent;` to main.rs**

- [ ] **Step 4: Run tests**

Run: `cargo test agent::claude_code`
Expected: All 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: Agent trait and Claude Code agent with stream parser"
```

---

### Task 9: Pi Agent

**Files:**
- Create: `raveloop-cli/src/agent/pi.rs`
- Modify: `raveloop-cli/src/agent/mod.rs` (already has `pub mod pi;`)

Port from `src/agents/pi/index.ts`, `stream-parser.ts`, and `setup.ts`. Same pattern as Claude Code but different stream format and includes the pi setup logic.

- [ ] **Step 1: Write the Pi stream parser**

The Pi agent uses a different JSON stream format from Claude Code. The parser handles three event types:

```rust
// In src/agent/pi.rs

fn parse_pi_stream_line(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> Option<FormattedOutput> {
    let line = line.trim();
    if line.is_empty() { return None; }

    let event: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = event.get("type")?.as_str()?;

    // Tool start → Progress
    if event_type == "tool_execution_start" {
        let name = event.get("tool_name")?.as_str().unwrap_or("");
        let input = event.get("tool_input").cloned().unwrap_or(serde_json::Value::Null);

        let tool = match name {
            "read" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None, edit_context: None,
            },
            "write" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
                edit_context: extract_edit_context(None, input.get("content").and_then(|v| v.as_str())),
            },
            "edit" => ToolCall {
                name: name.to_string(),
                path: input.get("file_path").or(input.get("path"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                detail: None,
                edit_context: extract_edit_context(
                    input.get("old_string").and_then(|v| v.as_str()),
                    input.get("new_string").and_then(|v| v.as_str()),
                ),
            },
            "grep" => ToolCall {
                name: name.to_string(), path: None,
                detail: Some(format!("\"{}\" in {}",
                    input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                    input.get("path").and_then(|v| v.as_str()).unwrap_or("."))),
                edit_context: None,
            },
            "find" => ToolCall {
                name: name.to_string(), path: None,
                detail: input.get("pattern").or(input.get("glob"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                edit_context: None,
            },
            "bash" => ToolCall {
                name: name.to_string(), path: None,
                detail: input.get("command").and_then(|v| v.as_str())
                    .map(|s| s.chars().take(120).collect()),
                edit_context: None,
            },
            _ => ToolCall {
                name: clean_tool_name(name), path: None,
                detail: Some(extract_tool_detail(&input)),
                edit_context: None,
            },
        };
        return Some(format_tool_call(&tool, phase, shown_highlights));
    }

    // Tool error → Persist
    if event_type == "tool_execution_end" {
        if event.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            return Some(FormattedOutput {
                text: format!("  \x1b[31m✗  tool error\x1b[0m"),
                persist: true,
            });
        }
        return None;
    }

    // Message end → Persist (result text)
    if event_type == "message_end" {
        if let Some(content) = event.get("content").and_then(|c| c.as_array()) {
            let text: String = content.iter()
                .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Some(FormattedOutput {
                    text: format_result_text(&text),
                    persist: true,
                });
            }
        }
    }

    None
}
```

- [ ] **Step 2: Write the PiAgent struct and Agent implementation**

The struct and CLI arg construction. Key differences from ClaudeCodeAgent:
- Uses `--no-session`, `--append-system-prompt`, `--provider`, `--mode json`
- Interactive mode loads system-prompt.md and memory-prompt.md from config
- Supports `--thinking` flag per phase

```rust
pub struct PiAgent {
    config: AgentConfig,
    config_root: String,
}

impl PiAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn load_prompt_file(&self, name: &str, ctx: &PlanContext) -> Result<String> {
        let path = Path::new(&self.config_root).join("agents/pi/prompts").join(name);
        let mut content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        content = content.replace("{{PROJECT}}", &ctx.project_dir);
        content = content.replace("{{DEV_ROOT}}", &ctx.dev_root);
        content = content.replace("{{PLAN}}", &ctx.plan_dir);
        Ok(content)
    }
}
```

The `invoke_headless` and `invoke_interactive` methods follow the same spawn+read pattern as ClaudeCodeAgent but with Pi's CLI args and `parse_pi_stream_line`.

- [ ] **Step 3: Write the setup method**

Ports `src/agents/pi/setup.ts`. Checks pi installation, installs subagent extension, generates agent definitions from skill files:

```rust
async fn setup(&self, ctx: &PlanContext) -> Result<()> {
    // Check pi is installed
    let which = std::process::Command::new("which").arg("pi").output();
    if which.map(|o| !o.status.success()).unwrap_or(true) {
        anyhow::bail!("pi is not installed. Install with: npm install -g @mariozechner/pi-coding-agent");
    }

    // Check/install subagent extension
    let settings_path = dirs::home_dir()
        .map(|h| h.join(".pi/agent/settings.json"))
        .unwrap_or_default();
    if settings_path.exists() {
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&settings_path).unwrap_or_default()
        ).unwrap_or_default();
        let has_subagent = settings.get("packages")
            .and_then(|p| p.as_array())
            .map(|pkgs| pkgs.iter().any(|p| p.as_str().map_or(false, |s| s.contains("pi-subagent"))))
            .unwrap_or(false);
        if !has_subagent {
            std::process::Command::new("pi")
                .args(["install", "npm:@mjakl/pi-subagent"])
                .status()?;
        }
    }

    // Generate agent definitions from skills
    let skills_dir = Path::new(&self.config_root).join("skills");
    if !skills_dir.exists() { return Ok(()); }
    let agents_dir = Path::new(&ctx.project_dir).join(".pi/agents");
    fs::create_dir_all(&agents_dir)?;
    for entry in fs::read_dir(&skills_dir)? {
        let entry = entry?;
        if entry.path().extension().map_or(true, |e| e != "md") { continue; }
        let content = fs::read_to_string(entry.path())?;
        // Parse YAML frontmatter and rewrite as agent definition
        if let Some(idx) = content.find("---\n") {
            if let Some(end) = content[idx+4..].find("\n---\n") {
                let frontmatter = &content[idx+4..idx+4+end];
                let body = &content[idx+4+end+5..];
                // Write agent definition (frontmatter + body)
                let dest = agents_dir.join(entry.file_name());
                fs::write(&dest, format!("---\n{frontmatter}\n---\n\n{body}"))?;
            }
        }
    }
    Ok(())
}
```

Note: Add `dirs = "5"` to Cargo.toml dependencies for home directory lookup.

- [ ] **Step 2: Add tests for Pi stream parsing**

Same test structure as claude_code tests but with Pi's JSON event format:
```rust
#[test]
fn parse_pi_tool_start() {
    let line = r#"{"type":"tool_execution_start","tool_name":"read","tool_input":{"file_path":"/foo.md"}}"#;
    // ...
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test agent::pi`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: Pi agent with stream parser and setup"
```

---

### Task 10: Phase Loop

**Files:**
- Create: `raveloop-cli/src/phase_loop.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/phase-loop.ts`. The main state machine that reads `phase.md`, dispatches to LLM or script phases, and drives the cycle.

- [ ] **Step 1: Write the phase loop**

```rust
// src/phase_loop.rs
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
                if !handle_script_phase(sp, plan_dir, config.headroom, &ui).await? {
                    ui.log("\nExiting.");
                    return Ok(());
                }
                continue;
            }
            Phase::Llm(lp) => {
                let agent_id = "main";
                log_phase_header(&ui, lp, &name);

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

                // Pre-work: save baseline
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

                // Check phase advanced
                let new_phase = read_phase(plan_dir)?;
                if new_phase == phase {
                    ui.log(&format!("\n  ✗  Phase did not advance from {phase}. Stopping."));
                    return Ok(());
                }

                // After dream, update baseline
                if lp == LlmPhase::Dream {
                    update_dream_baseline(plan_dir);
                }

                // After triage, dispatch subagents
                if lp == LlmPhase::Triage {
                    dispatch_subagents(agent.clone(), plan_dir, &ui).await?;
                }
            }
        }
    }
}
```

- [ ] **Step 2: Add `mod phase_loop;` to main.rs**

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: Compiles (no runtime tests for the phase loop — it requires live agent subprocesses)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: phase loop state machine"
```

---

### Task 11: Subagent Dispatch

**Files:**
- Create: `raveloop-cli/src/subagent.rs`
- Modify: `raveloop-cli/src/main.rs` (add mod)

Port from `src/subagent-dispatch.ts`. Concurrent dispatch using `JoinSet`.

- [ ] **Step 1: Write the subagent dispatch module**

```rust
// src/subagent.rs
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

    // Clean up dispatch file
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
```

- [ ] **Step 2: Add `mod subagent;` to main.rs**

- [ ] **Step 3: Run tests**

Run: `cargo test subagent`
Expected: Both tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: concurrent subagent dispatch with JoinSet"
```

---

### Task 12: TUI Rendering with Ratatui

**Files:**
- Modify: `raveloop-cli/src/ui.rs` (add rendering)

Add the Ratatui rendering loop to the existing `ui.rs`. This is the most complex task — it manages the terminal, renders the three-zone layout, handles suspend/resume, and processes keyboard input for confirmations.

- [ ] **Step 1: Add the TUI runner function**

Add to `src/ui.rs`:

```rust
use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Direction},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use tokio::sync::mpsc;

/// Run the TUI event loop. Spawned as a tokio task.
/// Returns when a Quit message is received.
pub async fn run_tui(mut rx: mpsc::UnboundedReceiver<UIMessage>) -> Result<(), anyhow::Error> {
    // Enter raw mode and alternate screen
    terminal::enable_raw_mode()?;
    let mut stdout = io::stderr();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new();
    let mut suspended = false;

    loop {
        if !suspended {
            terminal.draw(|f| draw_ui(f, &state))?;
        }

        // Poll for UI messages and keyboard events
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(UIMessage::Quit) | None => break,
                    Some(UIMessage::Suspend) => {
                        suspended = true;
                        terminal::disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    }
                    Some(UIMessage::Resume) => {
                        terminal::enable_raw_mode()?;
                        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                        terminal.clear()?;
                        suspended = false;
                    }
                    Some(msg) => {
                        state.handle_message(msg);
                    }
                }
            }
            // Check for keyboard input (for confirm prompts)
            _ = tokio::task::spawn_blocking(|| {
                event::poll(Duration::from_millis(50)).ok();
            }) => {
                if !suspended {
                    if let Ok(true) = event::poll(Duration::from_millis(0)) {
                        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
                            if let Some(ref mut confirm) = state.confirm_prompt {
                                let answer = match code {
                                    KeyCode::Char('n') | KeyCode::Char('N') => false,
                                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => true,
                                    _ => continue,
                                };
                                if let Some(reply) = confirm.reply.take() {
                                    let _ = reply.send(answer);
                                }
                                state.confirm_prompt = None;
                            }
                        }
                    }
                }
            }
        }
    }

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// Render the three-zone layout.
fn draw_ui(f: &mut Frame, state: &AppState) {
    let area = f.area();

    // Calculate progress area height (2 lines per agent group, 0 if empty)
    let progress_height = if state.progress_groups.is_empty() {
        0
    } else {
        state.progress_groups.len() as u16 * 2
    };

    // Add height for confirm prompt if present
    let confirm_height = if state.confirm_prompt.is_some() { 2 } else { 0 };

    let live_height = progress_height + confirm_height;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),           // Log area (fills remaining space)
            Constraint::Length(live_height), // Live progress area
            Constraint::Length(1),        // Status bar
        ])
        .split(area);

    // ── Log area ──
    let log_text: Vec<Line> = state.log_lines.iter()
        .map(|line| Line::raw(line.as_str()))
        .collect();
    let log_paragraph = Paragraph::new(log_text)
        .wrap(Wrap { trim: false })
        .scroll((
            state.log_lines.len().saturating_sub(chunks[0].height as usize) as u16,
            0,
        ));
    f.render_widget(log_paragraph, chunks[0]);

    // ── Live progress area ──
    if !state.progress_groups.is_empty() || state.confirm_prompt.is_some() {
        let mut progress_lines: Vec<Line> = Vec::new();

        for (_id, group) in &state.progress_groups {
            progress_lines.push(Line::raw(&group.header));
            if let Some(ref progress) = group.progress {
                progress_lines.push(Line::styled(
                    format!("      {progress}"),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            } else {
                progress_lines.push(Line::raw(""));
            }
        }

        if let Some(ref confirm) = state.confirm_prompt {
            progress_lines.push(Line::raw(""));
            progress_lines.push(Line::styled(
                format!("  ▶  {} [Y/n] ", confirm.message),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        let progress_paragraph = Paragraph::new(progress_lines);
        f.render_widget(progress_paragraph, chunks[1]);
    }

    // ── Status bar ──
    let status_text = if let Some(ref status) = state.status {
        format!(
            " {} · {} · {} · {}{}",
            status.project,
            status.plan,
            status.phase,
            status.agent,
            status.cycle.map(|c| format!(" · cycle {c}")).unwrap_or_default()
        )
    } else {
        " raveloop".to_string()
    };

    let status_bar = Paragraph::new(Line::styled(
        status_text,
        Style::default().add_modifier(Modifier::DIM),
    ))
    .block(Block::default().borders(Borders::TOP));
    f.render_widget(status_bar, chunks[2]);
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: Ratatui TUI with three-zone layout and confirm prompts"
```

---

### Task 13: Init Command

**Files:**
- Create: `raveloop-cli/src/init.rs`
- Create: `raveloop-cli/defaults/` directory tree with all default files
- Modify: `raveloop-cli/src/main.rs` (add mod)

This task creates the `init` subcommand that scaffolds a config directory from embedded defaults.

- [ ] **Step 1: Copy all default files from the existing TypeScript project**

Create the `defaults/` directory tree. Copy every file from the existing project that should be a default:

```bash
cd /Users/antony/Development/raveloop-cli
mkdir -p defaults/agents/claude-code defaults/agents/pi/prompts defaults/phases defaults/fixed-memory defaults/skills
```

Copy files from the existing TypeScript project:
- `config.yaml` → `defaults/config.yaml`
- `agents/claude-code/config.yaml` → `defaults/agents/claude-code/config.yaml`
- `agents/claude-code/tokens.yaml` → `defaults/agents/claude-code/tokens.yaml`
- `agents/pi/config.yaml` → `defaults/agents/pi/config.yaml`
- `agents/pi/tokens.yaml` → `defaults/agents/pi/tokens.yaml`
- `agents/pi/prompts/system-prompt.md` → `defaults/agents/pi/prompts/system-prompt.md`
- `agents/pi/prompts/memory-prompt.md` → `defaults/agents/pi/prompts/memory-prompt.md`
- `phases/*.md` → `defaults/phases/`
- `fixed-memory/*.md` → `defaults/fixed-memory/`
- `skills/*.md` → `defaults/skills/`

- [ ] **Step 2: Create the trampoline template**

Create `defaults/raveloop.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec raveloop-cli run --config "$SCRIPT_DIR" "$@"
```

- [ ] **Step 3: Write the init module**

```rust
// src/init.rs
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

struct EmbeddedFile {
    path: &'static str,
    content: &'static str,
}

const EMBEDDED_FILES: &[EmbeddedFile] = &[
    EmbeddedFile { path: "config.yaml", content: include_str!("../defaults/config.yaml") },
    EmbeddedFile { path: "agents/claude-code/config.yaml", content: include_str!("../defaults/agents/claude-code/config.yaml") },
    EmbeddedFile { path: "agents/claude-code/tokens.yaml", content: include_str!("../defaults/agents/claude-code/tokens.yaml") },
    EmbeddedFile { path: "agents/pi/config.yaml", content: include_str!("../defaults/agents/pi/config.yaml") },
    EmbeddedFile { path: "agents/pi/tokens.yaml", content: include_str!("../defaults/agents/pi/tokens.yaml") },
    EmbeddedFile { path: "agents/pi/prompts/system-prompt.md", content: include_str!("../defaults/agents/pi/prompts/system-prompt.md") },
    EmbeddedFile { path: "agents/pi/prompts/memory-prompt.md", content: include_str!("../defaults/agents/pi/prompts/memory-prompt.md") },
    EmbeddedFile { path: "phases/work.md", content: include_str!("../defaults/phases/work.md") },
    EmbeddedFile { path: "phases/analyse-work.md", content: include_str!("../defaults/phases/analyse-work.md") },
    EmbeddedFile { path: "phases/reflect.md", content: include_str!("../defaults/phases/reflect.md") },
    EmbeddedFile { path: "phases/dream.md", content: include_str!("../defaults/phases/dream.md") },
    EmbeddedFile { path: "phases/triage.md", content: include_str!("../defaults/phases/triage.md") },
    EmbeddedFile { path: "fixed-memory/coding-style.md", content: include_str!("../defaults/fixed-memory/coding-style.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-rust.md", content: include_str!("../defaults/fixed-memory/coding-style-rust.md") },
    EmbeddedFile { path: "fixed-memory/memory-style.md", content: include_str!("../defaults/fixed-memory/memory-style.md") },
    EmbeddedFile { path: "skills/brainstorming.md", content: include_str!("../defaults/skills/brainstorming.md") },
    EmbeddedFile { path: "skills/tdd.md", content: include_str!("../defaults/skills/tdd.md") },
    EmbeddedFile { path: "skills/writing-plans.md", content: include_str!("../defaults/skills/writing-plans.md") },
];

const TRAMPOLINE: &str = include_str!("../defaults/raveloop.sh");

pub fn run_init(target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create {}", target_dir.display()))?;

    let mut created = 0;
    let mut skipped = 0;

    for file in EMBEDDED_FILES {
        let dest = target_dir.join(file.path);
        if dest.exists() {
            skipped += 1;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, file.content)
            .with_context(|| format!("Failed to write {}", dest.display()))?;
        created += 1;
    }

    // Write trampoline script
    let trampoline_path = target_dir.join("raveloop");
    if !trampoline_path.exists() {
        fs::write(&trampoline_path, TRAMPOLINE)?;
        fs::set_permissions(&trampoline_path, fs::Permissions::from_mode(0o755))?;
        created += 1;
        println!("  ✓ Created trampoline: {}", trampoline_path.display());
    } else {
        skipped += 1;
    }

    println!("  ✓ Init complete: {created} created, {skipped} skipped (already exist)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_all_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target).unwrap();

        assert!(target.join("config.yaml").exists());
        assert!(target.join("phases/reflect.md").exists());
        assert!(target.join("agents/claude-code/config.yaml").exists());
        assert!(target.join("raveloop").exists());

        // Trampoline should be executable
        let perms = fs::metadata(target.join("raveloop")).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0);
    }

    #[test]
    fn init_skips_existing_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target).unwrap();

        // Modify a file
        fs::write(target.join("config.yaml"), "custom: true\n").unwrap();

        // Run init again
        run_init(&target).unwrap();

        // File should not be overwritten
        let content = fs::read_to_string(target.join("config.yaml")).unwrap();
        assert_eq!(content, "custom: true\n");
    }
}
```

- [ ] **Step 4: Add `mod init;` to main.rs**

- [ ] **Step 5: Run tests**

Run: `cargo test init`
Expected: Both tests pass

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: init command with embedded defaults and trampoline"
```

---

### Task 14: Wire Everything Together in Main

**Files:**
- Modify: `raveloop-cli/src/main.rs`

Set up the clap CLI with `init` and `run` subcommands, create the agent, spawn the TUI, and run the phase loop.

- [ ] **Step 1: Write the full main.rs**

```rust
// src/main.rs
mod agent;
mod config;
mod dream;
mod format;
mod git;
mod init;
mod phase_loop;
mod prompt;
mod subagent;
mod types;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use crate::agent::claude_code::ClaudeCodeAgent;
use crate::agent::pi::PiAgent;
use crate::agent::Agent;
use crate::config::{load_agent_config, load_shared_config};
use crate::git::find_project_root;
use crate::types::PlanContext;
use crate::ui::{run_tui, UI};

#[derive(Parser)]
#[command(name = "raveloop-cli", about = "An orchestration loop for LLM development cycles")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a config directory with default prompts and config
    Init {
        /// Target directory to create
        dir: PathBuf,
    },
    /// Run the phase loop on a plan directory
    Run {
        /// Path to the config directory
        #[arg(long)]
        config: PathBuf,
        /// Path to the plan directory
        plan_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir } => {
            init::run_init(&dir)
        }
        Commands::Run { config, plan_dir } => {
            run_phase_loop(&config, &plan_dir).await
        }
    }
}

async fn run_phase_loop(config_root: &Path, plan_dir: &Path) -> Result<()> {
    // Validate plan directory
    if !plan_dir.join("phase.md").exists() {
        anyhow::bail!(
            "{}/phase.md not found. Is this a valid plan directory?",
            plan_dir.display()
        );
    }

    let shared_config = load_shared_config(config_root)?;
    let agent_config = load_agent_config(config_root, &shared_config.agent)?;
    let project_dir = find_project_root(plan_dir)?;

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: project_dir.clone(),
        dev_root: Path::new(&project_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        related_plans: std::fs::read_to_string(plan_dir.join("related-plans.md"))
            .unwrap_or_default(),
        config_root: config_root.to_string_lossy().to_string(),
    };

    // Create the agent
    let agent: Arc<dyn Agent> = match shared_config.agent.as_str() {
        "claude-code" => Arc::new(ClaudeCodeAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        "pi" => Arc::new(PiAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        other => anyhow::bail!("Unknown agent: {other}"),
    };

    // Create the UI channel and handle
    let (tx, rx) = mpsc::unbounded_channel();
    let ui = UI::new(tx);

    // Spawn the TUI renderer
    let tui_handle = tokio::spawn(run_tui(rx));

    // Run the phase loop
    let result = phase_loop::phase_loop(agent, &ctx, &shared_config, &ui).await;

    // Shut down the TUI
    ui.quit();
    tui_handle.await??;

    result
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully. Fix any type mismatches between modules.

- [ ] **Step 3: Test the init command end-to-end**

```bash
cargo run -- init /tmp/test-raveloop-config
ls /tmp/test-raveloop-config/
cat /tmp/test-raveloop-config/raveloop
rm -rf /tmp/test-raveloop-config
```

Expected: Config directory created with all files and executable trampoline.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: wire up main with clap CLI, init and run subcommands"
```

---

### Task 15: Integration Testing

**Files:**
- Create: `raveloop-cli/tests/integration.rs`

Test the phase loop with a mock plan directory (no real agent subprocess). Verify phase transitions, dream guard, and commit logic work correctly end-to-end.

- [ ] **Step 1: Write integration tests for phase transitions**

```rust
// tests/integration.rs
use std::fs;
use tempfile::TempDir;

// Test the non-agent parts: config loading, phase parsing,
// dream guard, git operations, prompt composition.
// Full end-to-end testing with real agents requires claude/pi
// to be installed — those are manual verification steps.

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    // No memory → no dream
    assert!(!raveloop_cli::dream::should_dream(plan, 1500));

    // Create memory and baseline
    fs::write(plan.join("memory.md"), "word ".repeat(100)).unwrap();
    raveloop_cli::dream::update_dream_baseline(plan);

    // Within headroom → no dream
    fs::write(plan.join("memory.md"), "word ".repeat(200)).unwrap();
    assert!(!raveloop_cli::dream::should_dream(plan, 1500));

    // Beyond headroom → dream
    fs::write(plan.join("memory.md"), "word ".repeat(2000)).unwrap();
    assert!(raveloop_cli::dream::should_dream(plan, 1500));

    // After dream, update baseline → within headroom again
    raveloop_cli::dream::update_dream_baseline(plan);
    assert!(!raveloop_cli::dream::should_dream(plan, 1500));
}

#[test]
fn config_loading_integration() {
    let dir = TempDir::new().unwrap();
    let config_root = dir.path();

    // Create minimal config
    fs::write(config_root.join("config.yaml"), "agent: claude-code\nheadroom: 1500\n").unwrap();
    fs::create_dir_all(config_root.join("agents/claude-code")).unwrap();
    fs::write(
        config_root.join("agents/claude-code/config.yaml"),
        "models:\n  work: claude-sonnet-4-6\n  reflect: claude-haiku-4-5\nparams:\n  work:\n    dangerous: true\n",
    ).unwrap();
    fs::write(
        config_root.join("agents/claude-code/tokens.yaml"),
        "TOOL_READ: Read\n",
    ).unwrap();

    let shared = raveloop_cli::config::load_shared_config(config_root).unwrap();
    assert_eq!(shared.agent, "claude-code");
    assert_eq!(shared.headroom, 1500);

    let agent = raveloop_cli::config::load_agent_config(config_root, "claude-code").unwrap();
    assert_eq!(agent.models.get("work").unwrap(), "claude-sonnet-4-6");
    assert!(agent.params.get("work").unwrap().get("dangerous").is_some());

    let tokens = raveloop_cli::config::load_tokens(config_root, "claude-code").unwrap();
    assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
}
```

Note: For integration tests to work, make the crate modules public in `main.rs` by adding `pub` before each `mod` declaration, or create a `lib.rs` that re-exports them. The cleaner approach is to add a `src/lib.rs`:

```rust
// src/lib.rs
pub mod config;
pub mod dream;
pub mod format;
pub mod git;
pub mod init;
pub mod prompt;
pub mod types;
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test integration`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: integration tests for config, dream guard, and phase transitions"
```

---

### Task 16: Manual Verification with Real Agents

This task is not automated — it requires `claude` or `pi` to be installed.

- [ ] **Step 1: Create a test config directory**

```bash
cargo run -- init /tmp/raveloop-test
```

- [ ] **Step 2: Point it at an existing plan directory**

```bash
/tmp/raveloop-test/raveloop /path/to/existing/plan
```

Verify:
- Phase header renders correctly in the TUI
- Progress lines appear under the agent group and update
- Persist events (highlights, result text) appear in the log
- Commit messages appear after script phases
- Dream guard skips/runs correctly
- Confirm prompts work (Y/n)
- Interactive work phase suspends TUI and restores it after

- [ ] **Step 3: Test concurrent subagent dispatch**

Use a plan that has `subagent-dispatch.yaml` with multiple entries.
Verify that progress groups render side-by-side in the live area.

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: adjustments from manual verification"
```

---

## Summary

| Task | Module | Tests | Description |
|------|--------|-------|-------------|
| 1 | types.rs | 4 | Project scaffold, enums, structs |
| 2 | config.rs | 4 | YAML config loading |
| 3 | dream.rs | 5 | Dream guard logic |
| 4 | git.rs | 2 | Git commit, baseline, project root |
| 5 | prompt.rs | 3 | Prompt loading + token substitution |
| 6 | format.rs | 8 | Tool call formatting, result text parsing |
| 7 | ui.rs | 4 | UI handle, AppState, message routing |
| 8 | agent/ | 4 | Agent trait + Claude Code agent |
| 9 | agent/pi.rs | — | Pi agent + setup |
| 10 | phase_loop.rs | — | Phase state machine |
| 11 | subagent.rs | 2 | Concurrent subagent dispatch |
| 12 | ui.rs (TUI) | — | Ratatui three-zone rendering |
| 13 | init.rs | 2 | Embedded defaults + trampoline |
| 14 | main.rs | — | CLI wiring |
| 15 | tests/ | 2 | Integration tests |
| 16 | — | — | Manual verification |

---

## Continuation Prompt

Copy and paste this to resume work in a new session:

> I'm implementing the Raveloop Rust rewrite. The implementation plan is at `docs/superpowers/plans/2026-04-17-raveloop-rust-rewrite.md` and the design spec is at `docs/superpowers/specs/2026-04-16-rust-ratatui-rewrite-design.md`. The existing TypeScript source in `src/` has the exact logic to port — every function has a direct Rust equivalent. The Rust project should be created as a sibling directory at `../raveloop-cli/`. Please read the plan, check which tasks are already completed (look for checked boxes), and continue from the next incomplete task. Use subagent-driven development.
