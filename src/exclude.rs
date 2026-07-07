use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::{ExcludeMode, FileType, ShadowConfig};
use crate::fs_util;
use crate::git::GitRepo;

const SECTION_START: &str = "# >>> git-shadow managed (DO NOT EDIT) >>>";
const SECTION_END: &str = "# <<< git-shadow managed <<<";

/// Build an anchored, escaped `.git/info/exclude` pattern for a repo-root-relative path.
///
/// - Leading `/` anchors the pattern to the repository root so a top-level `local.md`
///   does not accidentally exclude files named `local.md` in subdirectories.
/// - gitignore metacharacters are backslash-escaped so filenames containing them are
///   matched literally.
/// - A trailing `/` marks directory entries.
pub fn to_exclude_pattern(relative_path: &str, is_directory: bool) -> String {
    let mut pattern = String::with_capacity(relative_path.len() + 4);
    pattern.push('/');
    pattern.push_str(&escape_gitignore(relative_path));
    if is_directory {
        pattern.push('/');
    }
    pattern
}

/// Escape gitignore special characters. `/` is left intact (it is structural).
fn escape_gitignore(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for (i, ch) in path.chars().enumerate() {
        match ch {
            // Glob metacharacters and the escape character itself: always escape.
            '\\' | '*' | '?' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            // `#` (comment) and `!` (negation) only matter at the start of a pattern;
            // escape them there. Leading whitespace would otherwise be significant/trimmed.
            '#' | '!' | ' ' | '\t' if i == 0 => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Compute the union of anchored exclude patterns required across all worktrees.
///
/// The `.git/info/exclude` file is shared via `common_dir`, but each worktree keeps its
/// own shadow config under `git_dir`. To avoid dropping an entry another worktree still
/// relies on, we union the `GitInfoExclude` phantom patterns from every worktree's config.
/// The current worktree's in-memory `current` config is used instead of its on-disk copy
/// (which may not be saved yet).
pub fn union_patterns(git: &GitRepo, current: &ShadowConfig) -> Vec<String> {
    let mut patterns = BTreeSet::new();
    collect_patterns(&mut patterns, current);

    let current_shadow = git.shadow_dir.canonicalize().ok();

    for wt in git.list_worktree_paths() {
        let Ok(wt_git) = GitRepo::discover(&wt) else {
            continue;
        };
        // Skip the current worktree; its state is provided via `current`.
        if wt_git.shadow_dir.canonicalize().ok() == current_shadow {
            continue;
        }
        if let Ok(cfg) = ShadowConfig::load(&wt_git.shadow_dir) {
            collect_patterns(&mut patterns, &cfg);
        }
    }

    patterns.into_iter().collect()
}

fn collect_patterns(patterns: &mut BTreeSet<String>, config: &ShadowConfig) {
    for (path, entry) in &config.files {
        if entry.file_type == FileType::Phantom && entry.exclude_mode == ExcludeMode::GitInfoExclude
        {
            patterns.insert(to_exclude_pattern(path, entry.is_directory));
        }
    }
}

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

    /// Replace the entire managed section with exactly `entries`.
    ///
    /// This regenerates the section from the caller's source of truth (the shadow
    /// config(s)), so any stale/unanchored entries left by older versions are upgraded.
    pub fn set_entries(&self, entries: &[String]) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let new_content = self.rebuild_content(&content, entries);
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

    #[test]
    fn test_to_exclude_pattern_anchors_file() {
        assert_eq!(to_exclude_pattern("local.md", false), "/local.md");
        assert_eq!(
            to_exclude_pattern("src/components/notes.md", false),
            "/src/components/notes.md"
        );
    }

    #[test]
    fn test_to_exclude_pattern_anchors_directory() {
        assert_eq!(to_exclude_pattern(".claude", true), "/.claude/");
    }

    #[test]
    fn test_to_exclude_pattern_escapes_metacharacters() {
        assert_eq!(to_exclude_pattern("a[b]*.md", false), "/a\\[b\\]\\*.md");
        assert_eq!(to_exclude_pattern("weird?.log", false), "/weird\\?.log");
    }

    #[test]
    fn test_to_exclude_pattern_escapes_leading_special_chars() {
        assert_eq!(to_exclude_pattern("#hash.md", false), "/\\#hash.md");
        assert_eq!(to_exclude_pattern("!bang.md", false), "/\\!bang.md");
        assert_eq!(to_exclude_pattern(" spaced.md", false), "/\\ spaced.md");
    }

    #[test]
    fn test_set_entries_replaces_stale_unanchored_entries() {
        let (_dir, manager) = setup();
        // Stale unanchored entry from an older version.
        manager.add_entry("local.md").unwrap();

        // Regenerate the section with anchored patterns.
        manager
            .set_entries(&["/local.md".to_string(), "/.claude/".to_string()])
            .unwrap();

        let entries = manager.list_entries().unwrap();
        assert!(entries.contains(&"/local.md".to_string()));
        assert!(entries.contains(&"/.claude/".to_string()));
        assert!(!entries.contains(&"local.md".to_string()));
    }

    #[test]
    fn test_set_entries_empty_removes_section() {
        let (_dir, manager) = setup();
        manager.add_entry("a.md").unwrap();
        manager.set_entries(&[]).unwrap();

        let content = std::fs::read_to_string(&manager.path).unwrap_or_default();
        assert!(!content.contains(SECTION_START));
    }
}
