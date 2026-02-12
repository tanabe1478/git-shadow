use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::fs_util;
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;

pub fn run() -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let mut config = ShadowConfig::load(&git.shadow_dir)?;

    // Guard: already suspended
    if config.suspended {
        return Err(ShadowError::AlreadySuspended.into());
    }

    // Guard: lock exists (commit in progress)
    if !matches!(lock::check_lock(&git.shadow_dir)?, LockStatus::Free) {
        anyhow::bail!("cannot suspend while a commit is in progress");
    }

    // Guard: stash has remaining files
    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let has_files = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false));
        if has_files {
            return Err(ShadowError::StashRemaining.into());
        }
    }

    if config.files.is_empty() {
        println!("no managed files to suspend");
        return Ok(());
    }

    // Create suspended directory
    let suspended_dir = git.shadow_dir.join("suspended");
    std::fs::create_dir_all(&suspended_dir).context("failed to create suspended directory")?;

    let mut count = 0;

    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                suspend_overlay(&git, &suspended_dir, file_path)?;
                count += 1;
            }
            FileType::Phantom => {
                if !entry.is_directory {
                    suspend_phantom(&git, &suspended_dir, file_path)?;
                    count += 1;
                }
            }
        }
    }

    config.suspended = true;
    config.save(&git.shadow_dir)?;

    println!(
        "{}",
        format!("shadow changes suspended for {} file(s)", count).green()
    );
    println!("working tree is now clean — you can switch branches");

    Ok(())
}

fn suspend_overlay(git: &GitRepo, suspended_dir: &std::path::Path, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let suspend_path = suspended_dir.join(&encoded);

    // Save current working tree content (with shadow changes) to suspended/
    let content =
        std::fs::read(&worktree_path).with_context(|| format!("failed to read {}", file_path))?;
    fs_util::atomic_write(&suspend_path, &content)
        .with_context(|| format!("failed to save suspended content for {}", file_path))?;

    // Restore baseline content to working tree
    let baseline = std::fs::read(&baseline_path)
        .with_context(|| format!("failed to read baseline for {}", file_path))?;
    std::fs::write(&worktree_path, &baseline)
        .with_context(|| format!("failed to restore baseline for {}", file_path))?;

    Ok(())
}

fn suspend_phantom(git: &GitRepo, suspended_dir: &std::path::Path, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let suspend_path = suspended_dir.join(&encoded);

    if !worktree_path.exists() {
        return Ok(());
    }

    // Save phantom content to suspended/
    let content =
        std::fs::read(&worktree_path).with_context(|| format!("failed to read {}", file_path))?;
    fs_util::atomic_write(&suspend_path, &content)
        .with_context(|| format!("failed to save suspended content for {}", file_path))?;

    // Remove phantom from working tree
    std::fs::remove_file(&worktree_path)
        .with_context(|| format!("failed to remove {} from working tree", file_path))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{ExcludeMode, ShadowConfig};
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
    fn test_suspend_overlay_saves_and_restores_baseline() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

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

        // Suspend
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        super::suspend_overlay(&git, &suspended_dir, "CLAUDE.md").unwrap();

        // Working tree should have baseline content
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(wt, "# Team\n");

        // Suspended should have shadow content
        let suspended = std::fs::read_to_string(suspended_dir.join(&encoded)).unwrap();
        assert_eq!(suspended, "# Team\n# My shadow\n");
    }

    #[test]
    fn test_suspend_phantom_saves_and_removes() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create phantom
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Suspend
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        super::suspend_phantom(&git, &suspended_dir, "local.md").unwrap();

        // Phantom should be removed from working tree
        assert!(!git.root.join("local.md").exists());

        // Suspended should have content
        let encoded = path::encode_path("local.md");
        let suspended = std::fs::read_to_string(suspended_dir.join(&encoded)).unwrap();
        assert_eq!(suspended, "# Local\n");
    }

    #[test]
    fn test_suspend_sets_suspended_flag() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        assert!(!config.suspended);

        // Simulate suspend logic
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        super::suspend_overlay(&git, &suspended_dir, "CLAUDE.md").unwrap();
        config.suspended = true;
        config.save(&git.shadow_dir).unwrap();

        // Reload and verify
        let loaded = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(loaded.suspended);
    }

    #[test]
    fn test_suspend_blocks_when_already_suspended() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        config.suspended = true;
        config.save(&git.shadow_dir).unwrap();

        // Should detect already suspended via config
        let loaded = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(loaded.suspended);
    }

    #[test]
    fn test_suspend_blocks_when_stash_has_files() {
        let (_dir, git) = make_test_repo();

        // Create stash remnant
        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "remnant").unwrap();

        let stash_dir = git.shadow_dir.join("stash");
        let has_files = std::fs::read_dir(&stash_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false));
        assert!(has_files);
    }
}
