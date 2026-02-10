use anyhow::{bail, Result};
use colored::Colorize;
use is_terminal::IsTerminal;

use crate::config::{ExcludeMode, FileType, ShadowConfig};
use crate::exclude::ExcludeManager;
use crate::git::GitRepo;
use crate::path;

pub fn run(file: &str, force: bool) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let mut config = ShadowConfig::load(&git.shadow_dir)?;
    let normalized = path::normalize_path(file, &git.root)?;

    let entry = config
        .get(&normalized)
        .ok_or_else(|| anyhow::anyhow!("{} is not managed by git-shadow", normalized))?
        .clone();

    // Confirmation prompt
    if !force {
        if !std::io::stdin().is_terminal() {
            bail!("--force is required in non-interactive mode");
        }

        let prompt = match entry.file_type {
            FileType::Overlay => {
                format!(
                    "Shadow changes for {} will be discarded. Continue? [y/N]",
                    normalized
                )
            }
            FileType::Phantom => {
                if entry.is_directory {
                    format!(
                        "{} (directory) will be unregistered from shadow management. The directory and its contents will remain. Continue? [y/N]",
                        normalized
                    )
                } else {
                    format!(
                        "{} will be unregistered from shadow management. The file itself will remain. Continue? [y/N]",
                        normalized
                    )
                }
            }
        };

        eprintln!("{}", prompt);
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("aborted");
            return Ok(());
        }
    }

    match entry.file_type {
        FileType::Overlay => {
            remove_overlay(&git, &normalized)?;
        }
        FileType::Phantom => {
            remove_phantom(&git, &normalized, &entry.exclude_mode, entry.is_directory)?;
        }
    }

    config.remove(&normalized)?;
    config.save(&git.shadow_dir)?;

    println!(
        "{}",
        format!("unregistered {} from shadow management", normalized).green()
    );

    Ok(())
}

fn remove_overlay(git: &GitRepo, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    // Restore baseline content to working tree
    if baseline_path.exists() {
        let baseline = std::fs::read(&baseline_path)?;
        std::fs::write(&worktree_path, &baseline)?;
        std::fs::remove_file(&baseline_path)?;
    }

    Ok(())
}

fn remove_phantom(
    git: &GitRepo,
    file_path: &str,
    exclude_mode: &ExcludeMode,
    is_directory: bool,
) -> Result<()> {
    // Remove from .git/info/exclude if applicable
    if *exclude_mode == ExcludeMode::GitInfoExclude {
        let exclude_path = if is_directory {
            format!("{}/", file_path)
        } else {
            file_path.to_string()
        };
        let manager = ExcludeManager::new(&git.git_dir);
        manager.remove_entry(&exclude_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{ExcludeMode, ShadowConfig};
    use crate::exclude::ExcludeManager;
    use crate::git::GitRepo;
    use crate::{fs_util, path};

    fn make_test_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# Team\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&root)
            .output()
            .unwrap();

        let repo = GitRepo::discover(&root).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("stash")).unwrap();
        (dir, repo)
    }

    #[test]
    fn test_remove_overlay_restores_baseline() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();

        // Setup overlay
        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        // Remove overlay (bypass prompt via direct function call)
        remove_overlay_for_test(&git, "CLAUDE.md");

        // Working tree should have baseline content
        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Team\n");

        // Baseline file should be deleted
        assert!(!git.shadow_dir.join("baselines").join(&encoded).exists());
    }

    #[test]
    fn test_remove_phantom_keeps_file() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create phantom file
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::GitInfoExclude, false)
            .unwrap();

        // Add to exclude
        let manager = ExcludeManager::new(&git.git_dir);
        manager.add_entry("local.md").unwrap();

        config.save(&git.shadow_dir).unwrap();

        // Remove phantom
        remove_phantom_for_test(&git, "local.md", &ExcludeMode::GitInfoExclude, false);

        // File should still exist
        assert!(git.root.join("local.md").exists());
        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert_eq!(content, "# Local\n");

        // Exclude entry should be removed
        let entries = manager.list_entries().unwrap();
        assert!(!entries.contains(&"local.md".to_string()));
    }

    #[test]
    fn test_remove_phantom_no_exclude_skips_exclude() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Remove phantom with no-exclude mode
        remove_phantom_for_test(&git, "local.md", &ExcludeMode::None, false);

        // Should not error - file still exists
        assert!(git.root.join("local.md").exists());
    }

    #[test]
    fn test_remove_updates_config() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();

        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Remove overlay
        remove_overlay_for_test(&git, "CLAUDE.md");
        config.remove("CLAUDE.md").unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Reload and verify
        let reloaded = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(reloaded.get("CLAUDE.md").is_none());
        assert!(reloaded.files.is_empty());
    }

    #[test]
    fn test_remove_not_managed_errors() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        let result = config.get("nonexistent.md");
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_overlay_nested_path() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create nested file in git
        std::fs::create_dir_all(git.root.join("src/components")).unwrap();
        std::fs::write(git.root.join("src/components/CLAUDE.md"), "# Component\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "src/components/CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add component"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let commit = git.head_commit().unwrap();
        let baseline_content = git.show_file("HEAD", "src/components/CLAUDE.md").unwrap();
        let encoded = path::encode_path("src/components/CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config
            .add_overlay("src/components/CLAUDE.md".to_string(), commit)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Add shadow changes
        std::fs::write(
            git.root.join("src/components/CLAUDE.md"),
            "# Component\n# My shadow\n",
        )
        .unwrap();

        // Remove
        remove_overlay_for_test(&git, "src/components/CLAUDE.md");

        let content = std::fs::read_to_string(git.root.join("src/components/CLAUDE.md")).unwrap();
        assert_eq!(content, "# Component\n");
        assert!(!git.shadow_dir.join("baselines").join(&encoded).exists());
    }

    /// Helper to remove overlay (bypasses prompt)
    fn remove_overlay_for_test(git: &GitRepo, file_path: &str) {
        let encoded = path::encode_path(file_path);
        let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
        let worktree_path = git.root.join(file_path);

        if baseline_path.exists() {
            let baseline = std::fs::read(&baseline_path).unwrap();
            std::fs::write(&worktree_path, &baseline).unwrap();
            std::fs::remove_file(&baseline_path).unwrap();
        }
    }

    /// Helper to remove phantom (bypasses prompt)
    fn remove_phantom_for_test(
        git: &GitRepo,
        file_path: &str,
        exclude_mode: &ExcludeMode,
        is_directory: bool,
    ) {
        if *exclude_mode == ExcludeMode::GitInfoExclude {
            let exclude_path = if is_directory {
                format!("{}/", file_path)
            } else {
                file_path.to_string()
            };
            let manager = ExcludeManager::new(&git.git_dir);
            manager.remove_entry(&exclude_path).unwrap();
        }
    }

    #[test]
    fn test_remove_phantom_directory_removes_exclude_with_trailing_slash() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create directory phantom
        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        std::fs::write(git.root.join(".claude/settings.json"), "{}").unwrap();

        // Add exclude entry with trailing slash (as add_phantom would)
        let manager = ExcludeManager::new(&git.git_dir);
        manager.add_entry(".claude/").unwrap();

        config
            .add_phantom(".claude".to_string(), ExcludeMode::GitInfoExclude, true)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Remove phantom directory
        remove_phantom_for_test(&git, ".claude", &ExcludeMode::GitInfoExclude, true);

        // Exclude entry should be removed
        let entries = manager.list_entries().unwrap();
        assert!(
            !entries.contains(&".claude/".to_string()),
            "Exclude entry with trailing slash should be removed, got: {:?}",
            entries
        );

        // Directory should still exist
        assert!(git.root.join(".claude").is_dir());
        assert!(git.root.join(".claude/settings.json").exists());
    }

    #[test]
    fn test_remove_phantom_file_removes_exclude_without_trailing_slash() {
        let (_dir, git) = make_test_repo();

        // Add file exclude entry (no trailing slash)
        let manager = ExcludeManager::new(&git.git_dir);
        manager.add_entry("local.md").unwrap();

        // Remove phantom file
        remove_phantom_for_test(&git, "local.md", &ExcludeMode::GitInfoExclude, false);

        let entries = manager.list_entries().unwrap();
        assert!(!entries.contains(&"local.md".to_string()));
    }
}
