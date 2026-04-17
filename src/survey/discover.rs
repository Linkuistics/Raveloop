// src/survey/discover.rs
//
// Plan discovery: walk a root directory, find plan directories
// (identified by a `phase.md` file), and classify each by project
// (the basename of the nearest ancestor containing `.git`).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::git::find_project_root;

/// A single plan's state, bundled for inclusion in the survey prompt.
#[derive(Debug)]
pub struct PlanSnapshot {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub backlog: Option<String>,
    pub memory: Option<String>,
}

/// Derive the project name for a plan by walking up from the plan's
/// own directory to the nearest ancestor containing `.git`, then
/// taking that ancestor's basename. Hard errors if no `.git` is found
/// above the plan — plans outside a git repo are unsupported.
fn project_name_for_plan(plan_path: &Path) -> Result<String> {
    let git_root = find_project_root(plan_path)?;
    Path::new(&git_root)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .with_context(|| format!("Could not derive project name from git root {git_root}"))
}

/// Walk `root` looking for plan directories. A directory is a plan iff
/// it contains a `phase.md` file; this matches the convention used
/// everywhere else in Raveloop. For each plan, the project name is the
/// basename of the nearest ancestor containing `.git` — not the root
/// basename — so plans from different repos under the same `--root`
/// are labelled correctly. Returned plans are sorted by plan name for
/// deterministic output.
pub fn discover_plans(root: &Path) -> Result<Vec<PlanSnapshot>> {
    let mut plans = Vec::new();

    let entries = fs::read_dir(root)
        .with_context(|| format!("Failed to read plan root {}", root.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let phase_file = path.join("phase.md");
        if !phase_file.exists() {
            continue;
        }

        let plan = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let project = project_name_for_plan(&path)?;
        let phase = fs::read_to_string(&phase_file)
            .with_context(|| format!("Failed to read {}", phase_file.display()))?
            .trim()
            .to_string();
        let backlog = fs::read_to_string(path.join("backlog.md")).ok();
        let memory = fs::read_to_string(path.join("memory.md")).ok();

        plans.push(PlanSnapshot {
            project,
            plan,
            phase,
            backlog,
            memory,
        });
    }

    plans.sort_by(|a, b| a.plan.cmp(&b.plan));
    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_plan(
        root: &Path,
        name: &str,
        phase: &str,
        backlog: Option<&str>,
        memory: Option<&str>,
    ) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("phase.md"), phase).unwrap();
        if let Some(b) = backlog {
            fs::write(dir.join("backlog.md"), b).unwrap();
        }
        if let Some(m) = memory {
            fs::write(dir.join("memory.md"), m).unwrap();
        }
    }

    /// Create a fake git project at `project_dir` with an empty `.git`
    /// directory — `find_project_root` only checks for `.git`'s
    /// existence, not its validity, so this is sufficient for tests.
    fn mark_as_git_project(project_dir: &Path) {
        fs::create_dir_all(project_dir.join(".git")).unwrap();
    }

    #[test]
    fn discover_plans_finds_directories_with_phase_md() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("my-project");
        let root = project.join("LLM_STATE");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "work\n", Some("# backlog a\n"), Some("# memory a\n"));
        write_plan(&root, "plan-b", "triage\n", Some("# backlog b\n"), None);
        // A directory WITHOUT phase.md is ignored.
        fs::create_dir_all(root.join("not-a-plan")).unwrap();
        fs::write(root.join("not-a-plan").join("backlog.md"), "noise").unwrap();

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].plan, "plan-a");
        assert_eq!(plans[1].plan, "plan-b");
    }

    #[test]
    fn discover_plans_derives_project_from_ancestor_git_dir() {
        // Project layout:
        //   tmp/my-project/.git          <- project marker
        //   tmp/my-project/LLM_STATE/    <- the --root argument
        //   tmp/my-project/LLM_STATE/plan-x/phase.md
        // The project name should be "my-project", NOT "LLM_STATE".
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("my-project");
        let root = project.join("LLM_STATE");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-x", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].project, "my-project");
    }

    #[test]
    fn discover_plans_errors_when_no_git_above_plan() {
        // Tempdir has no `.git` anywhere above the plan → hard error.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("rogue-state");
        fs::create_dir_all(&root).unwrap();
        write_plan(&root, "plan-x", "work\n", None, None);

        let err = discover_plans(&root).unwrap_err();
        assert!(format!("{err:#}").contains("No .git found"));
    }

    #[test]
    fn discover_plans_trims_phase_whitespace() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "  \n work \n\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans[0].phase, "work");
    }

    #[test]
    fn discover_plans_records_missing_backlog_and_memory_as_none() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "plan-a", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert!(plans[0].backlog.is_none());
        assert!(plans[0].memory.is_none());
    }

    #[test]
    fn discover_plans_returns_sorted_by_plan_name() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("p");
        let root = project.join("state");
        fs::create_dir_all(&root).unwrap();
        mark_as_git_project(&project);
        write_plan(&root, "zeta", "work\n", None, None);
        write_plan(&root, "alpha", "work\n", None, None);
        write_plan(&root, "mu", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        let names: Vec<_> = plans.iter().map(|p| p.plan.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn discover_plans_errors_when_root_unreadable() {
        let missing = PathBuf::from("/definitely/not/a/path/for/survey/test");
        assert!(discover_plans(&missing).is_err());
    }
}
