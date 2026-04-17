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
