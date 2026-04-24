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
    EmbeddedFile { path: "survey-incremental.md", content: include_str!("../defaults/survey-incremental.md") },
    EmbeddedFile { path: "create-plan.md", content: include_str!("../defaults/create-plan.md") },
    EmbeddedFile { path: "discover-stage1.md", content: include_str!("../defaults/discover-stage1.md") },
    EmbeddedFile { path: "discover-stage2.md", content: include_str!("../defaults/discover-stage2.md") },
    EmbeddedFile { path: "ontology.yaml", content: include_str!("../defaults/ontology.yaml") },
];

/// Paths that used to ship via `EMBEDDED_FILES` but have been removed
/// or renamed. `init --force` deletes these from the target dir so
/// existing configs catch up to the current layout without manual
/// cleanup. Keep the list narrow: only add an entry when we are sure
/// the path was once ours and a user could not legitimately be keeping
/// it for their own purposes.
const RETIRED_PATHS: &[&str] = &[
    // Former location of pi subagent prompts; moved to
    // `agents/pi/subagents/` as part of the drift-guard cleanup.
    "skills",
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

    let mut pruned = 0;
    if force {
        for retired in RETIRED_PATHS {
            let path = target_dir.join(retired);
            if !path.exists() {
                continue;
            }
            if path.is_dir() {
                fs::remove_dir_all(&path).with_context(|| {
                    format!("Failed to prune retired dir {}", path.display())
                })?;
            } else {
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to prune retired file {}", path.display())
                })?;
            }
            pruned += 1;
            println!("  ✗ Pruned retired path: {retired}");
        }
    }

    if force {
        println!("  ✓ Init --force complete: {created} created, {overwritten} overwritten, {skipped} unchanged, {pruned} pruned");
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
    fn every_file_under_defaults_is_registered_in_embedded_files() {
        // Drift guard: every file shipped under `defaults/` must have a
        // matching `EmbeddedFile` entry, otherwise `init` and
        // `init --force` silently fail to scaffold or refresh it — the
        // file ships in the source tree but never reaches the user's
        // config dir. This generalises an older coding-style-specific
        // guard so any addition anywhere in `defaults/` is covered. A
        // missing registration for `discover-stage2.md` is exactly the
        // bug this replaces.
        let defaults_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults");
        let mut on_disk: Vec<String> = Vec::new();
        collect_files_recursively(&defaults_root, &defaults_root, &mut on_disk);
        on_disk.sort();
        assert!(!on_disk.is_empty(), "expected at least one file under defaults/");

        let embedded: std::collections::HashSet<&str> =
            EMBEDDED_FILES.iter().map(|f| f.path).collect();
        let missing: Vec<&String> = on_disk
            .iter()
            .filter(|p| !embedded.contains(p.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "defaults/ file(s) missing from EMBEDDED_FILES: {missing:?}"
        );
    }

    fn collect_files_recursively(root: &Path, current: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(current).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursively(root, &path, out);
            } else if path.is_file() {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    #[test]
    fn init_force_prunes_retired_paths() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        // Simulate the pre-rename layout: a stale skills/ directory
        // holding pi subagent prompts that have since moved to
        // agents/pi/subagents/.
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(target.join("skills/brainstorming.md"), "stale\n").unwrap();

        run_init(&target, true).unwrap();

        assert!(
            !target.join("skills").exists(),
            "init --force should prune the retired skills/ directory"
        );
        assert!(
            target.join("agents/pi/subagents/brainstorming.md").exists(),
            "replacement location must still be scaffolded after prune"
        );
    }

    #[test]
    fn init_without_force_does_not_prune_retired_paths() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(target.join("skills/brainstorming.md"), "stale\n").unwrap();

        run_init(&target, false).unwrap();

        assert!(
            target.join("skills").exists(),
            "non-force init must not prune — pruning is opt-in via --force"
        );
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
