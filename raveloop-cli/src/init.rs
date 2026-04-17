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

        let perms = fs::metadata(target.join("raveloop")).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0);
    }

    #[test]
    fn init_skips_existing_files() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target).unwrap();

        fs::write(target.join("config.yaml"), "custom: true\n").unwrap();
        run_init(&target).unwrap();

        let content = fs::read_to_string(target.join("config.yaml")).unwrap();
        assert_eq!(content, "custom: true\n");
    }
}
