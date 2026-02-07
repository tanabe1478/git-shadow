use std::path::{Path, PathBuf};

use crate::fs_util;

const SECTION_START: &str = "# >>> git-shadow managed (DO NOT EDIT) >>>";
const SECTION_END: &str = "# <<< git-shadow managed <<<";

pub struct ExcludeManager {
    path: PathBuf,
}

impl ExcludeManager {
    pub fn new(git_dir: &Path) -> Self {
        Self {
            path: git_dir.join("info").join("exclude"),
        }
    }

    /// Add a path to the managed section (idempotent)
    pub fn add_entry(&self, entry_path: &str) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut entries = self.parse_section(&content);

        if entries.contains(&entry_path.to_string()) {
            return Ok(());
        }
        entries.push(entry_path.to_string());

        let new_content = self.rebuild_content(&content, &entries);
        fs_util::atomic_write(&self.path, new_content.as_bytes())?;
        Ok(())
    }

    /// Remove a path from the managed section
    pub fn remove_entry(&self, entry_path: &str) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut entries = self.parse_section(&content);

        entries.retain(|e| e != entry_path);

        let new_content = self.rebuild_content(&content, &entries);
        fs_util::atomic_write(&self.path, new_content.as_bytes())?;
        Ok(())
    }

    /// List all entries in the managed section
    pub fn list_entries(&self) -> anyhow::Result<Vec<String>> {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        Ok(self.parse_section(&content))
    }

    /// Parse entries from the managed section
    fn parse_section(&self, content: &str) -> Vec<String> {
        let mut in_section = false;
        let mut entries = Vec::new();

        for line in content.lines() {
            if line == SECTION_START {
                in_section = true;
                continue;
            }
            if line == SECTION_END {
                in_section = false;
                continue;
            }
            if in_section {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    entries.push(trimmed.to_string());
                }
            }
        }
        entries
    }

    /// Rebuild file content: preserve everything outside the section, replace section
    fn rebuild_content(&self, original: &str, entries: &[String]) -> String {
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut in_section = false;
        let mut past_section = false;

        for line in original.lines() {
            if line == SECTION_START {
                in_section = true;
                continue;
            }
            if line == SECTION_END {
                in_section = false;
                past_section = true;
                continue;
            }
            if in_section {
                continue;
            }
            if past_section {
                after.push(line.to_string());
            } else {
                before.push(line.to_string());
            }
        }

        let mut result = before.join("\n");

        if entries.is_empty() {
            // No entries: don't add section at all
            if !after.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&after.join("\n"));
            }
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            return result;
        }

        // Add section with entries
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(SECTION_START);
        result.push('\n');
        for entry in entries {
            result.push_str(entry);
            result.push('\n');
        }
        result.push_str(SECTION_END);
        result.push('\n');

        if !after.is_empty() {
            result.push_str(&after.join("\n"));
            if !result.ends_with('\n') {
                result.push('\n');
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, ExcludeManager) {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        let info_dir = git_dir.join("info");
        std::fs::create_dir_all(&info_dir).unwrap();
        let manager = ExcludeManager::new(&git_dir);
        (dir, manager)
    }

    #[test]
    fn test_add_entry_creates_section() {
        let (_dir, manager) = setup();
        manager.add_entry("src/components/CLAUDE.md").unwrap();

        let content = std::fs::read_to_string(&manager.path).unwrap();
        assert!(content.contains(SECTION_START));
        assert!(content.contains("src/components/CLAUDE.md"));
        assert!(content.contains(SECTION_END));
    }

    #[test]
    fn test_add_entry_idempotent() {
        let (_dir, manager) = setup();
        manager.add_entry("CLAUDE.md").unwrap();
        manager.add_entry("CLAUDE.md").unwrap();

        let entries = manager.list_entries().unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_add_multiple_entries() {
        let (_dir, manager) = setup();
        manager.add_entry("a.md").unwrap();
        manager.add_entry("b.md").unwrap();

        let entries = manager.list_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&"a.md".to_string()));
        assert!(entries.contains(&"b.md".to_string()));
    }

    #[test]
    fn test_remove_entry() {
        let (_dir, manager) = setup();
        manager.add_entry("a.md").unwrap();
        manager.add_entry("b.md").unwrap();
        manager.remove_entry("a.md").unwrap();

        let entries = manager.list_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains(&"b.md".to_string()));
    }

    #[test]
    fn test_remove_last_entry_removes_section() {
        let (_dir, manager) = setup();
        manager.add_entry("a.md").unwrap();
        manager.remove_entry("a.md").unwrap();

        let content = std::fs::read_to_string(&manager.path).unwrap_or_default();
        assert!(!content.contains(SECTION_START));
        assert!(!content.contains(SECTION_END));
    }

    #[test]
    fn test_preserves_existing_content() {
        let (_dir, manager) = setup();
        std::fs::write(&manager.path, "*.log\ntmp/\n").unwrap();

        manager.add_entry("CLAUDE.md").unwrap();

        let content = std::fs::read_to_string(&manager.path).unwrap();
        assert!(content.contains("*.log"));
        assert!(content.contains("tmp/"));
        assert!(content.contains("CLAUDE.md"));
    }

    #[test]
    fn test_list_entries_empty_file() {
        let (_dir, manager) = setup();
        let entries = manager.list_entries().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_list_entries_no_section() {
        let (_dir, manager) = setup();
        std::fs::write(&manager.path, "*.log\n").unwrap();
        let entries = manager.list_entries().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_entry_is_ok() {
        let (_dir, manager) = setup();
        manager.add_entry("a.md").unwrap();
        assert!(manager.remove_entry("nonexistent.md").is_ok());
    }
}
