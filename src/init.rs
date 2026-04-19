use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::CONFIG_ENV_VAR;

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
    EmbeddedFile { path: "fixed-memory/coding-style-swift.md", content: include_str!("../defaults/fixed-memory/coding-style-swift.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-typescript.md", content: include_str!("../defaults/fixed-memory/coding-style-typescript.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-python.md", content: include_str!("../defaults/fixed-memory/coding-style-python.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-bash.md", content: include_str!("../defaults/fixed-memory/coding-style-bash.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-elixir.md", content: include_str!("../defaults/fixed-memory/coding-style-elixir.md") },
    EmbeddedFile { path: "fixed-memory/memory-style.md", content: include_str!("../defaults/fixed-memory/memory-style.md") },
    EmbeddedFile { path: "agents/pi/subagents/brainstorming.md", content: include_str!("../defaults/agents/pi/subagents/brainstorming.md") },
    EmbeddedFile { path: "agents/pi/subagents/tdd.md", content: include_str!("../defaults/agents/pi/subagents/tdd.md") },
    EmbeddedFile { path: "agents/pi/subagents/writing-plans.md", content: include_str!("../defaults/agents/pi/subagents/writing-plans.md") },
    EmbeddedFile { path: "survey.md", content: include_str!("../defaults/survey.md") },
    EmbeddedFile { path: "create-plan.md", content: include_str!("../defaults/create-plan.md") },
];

pub fn run_init(target_dir: &Path, force: bool) -> Result<()> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create {}", target_dir.display()))?;

    let mut created = 0;
    let mut overwritten = 0;
    let mut skipped = 0;

    for file in EMBEDDED_FILES {
        let dest = target_dir.join(file.path);
        let exists = dest.exists();
        if exists && !force {
            skipped += 1;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // When forcing, only overwrite if content actually differs — keeps
        // the audit noise proportional to what changed.
        let should_write = if exists {
            fs::read_to_string(&dest).ok().as_deref() != Some(file.content)
        } else {
            true
        };
        if !should_write {
            skipped += 1;
            continue;
        }
        fs::write(&dest, file.content)
            .with_context(|| format!("Failed to write {}", dest.display()))?;
        if exists {
            overwritten += 1;
            println!("  ↻ Overwritten: {}", file.path);
        } else {
            created += 1;
        }
    }

    if force {
        println!("  ✓ Init --force complete: {created} created, {overwritten} overwritten, {skipped} unchanged");
    } else {
        println!("  ✓ Init complete: {created} created, {skipped} skipped (already exist)");
    }

    print_discovery_guidance(target_dir);
    Ok(())
}

/// After scaffolding, tell the user how to make `ravel-lite` find this
/// config. Silent when the target is already the XDG default, since the
/// binary will find it there with no setup.
fn print_discovery_guidance(target_dir: &Path) {
    let xdg_default = dirs::config_dir().map(|p| p.join("ravel-lite"));
    let is_xdg_default = xdg_default.as_deref() == Some(target_dir);

    println!();
    if is_xdg_default {
        println!(
            "  Config is at the default location; ravel-lite will discover it automatically."
        );
    } else {
        println!("  To use this config as the default for ravel-lite, set:");
        println!("    export {CONFIG_ENV_VAR}={}", target_dir.display());
        println!("  Or pass --config {} on each invocation.", target_dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_all_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        assert!(target.join("config.yaml").exists());
        assert!(target.join("phases/reflect.md").exists());
        assert!(target.join("agents/claude-code/config.yaml").exists());
    }

    #[test]
    fn init_does_not_write_a_trampoline() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        assert!(
            !target.join("ravel-lite").exists(),
            "init must not scaffold a ravel-lite trampoline; discovery uses env var + default location"
        );
    }

    #[test]
    fn init_skips_existing_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        fs::write(target.join("config.yaml"), "custom: true\n").unwrap();
        run_init(&target, false).unwrap();

        let content = fs::read_to_string(target.join("config.yaml")).unwrap();
        assert_eq!(content, "custom: true\n");
    }

    #[test]
    fn every_default_coding_style_file_is_embedded() {
        // Drift guard: if a new defaults/fixed-memory/coding-style-*.md is
        // added on disk but no matching EmbeddedFile is registered, the new
        // file silently fails to ship via `init`. This test catches that.
        let defaults_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("defaults")
            .join("fixed-memory");
        let on_disk: Vec<String> = fs::read_dir(&defaults_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.starts_with("coding-style-") && name.ends_with(".md"))
            .collect();
        assert!(!on_disk.is_empty(), "expected at least one coding-style-*.md on disk");

        let embedded: std::collections::HashSet<&str> = EMBEDDED_FILES
            .iter()
            .map(|f| f.path)
            .collect();
        for name in &on_disk {
            let expected = format!("fixed-memory/{name}");
            assert!(
                embedded.contains(expected.as_str()),
                "defaults/fixed-memory/{name} is not registered in EMBEDDED_FILES"
            );
        }
    }

    #[test]
    fn init_force_overwrites_existing_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        // Simulate a user-edited phase prompt.
        let reflect_path = target.join("phases/reflect.md");
        fs::write(&reflect_path, "STALE CONTENT\n").unwrap();

        run_init(&target, true).unwrap();

        // Phase prompt restored from embedded default.
        let content = fs::read_to_string(&reflect_path).unwrap();
        assert_ne!(content, "STALE CONTENT\n");
        assert!(content.contains("[NEW]") || content.contains("[IMPRECISE]"),
            "expected new state-based labels in the refreshed prompt");
    }
}
