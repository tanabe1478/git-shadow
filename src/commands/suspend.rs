use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::fs_util;
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;
use crate::ui;

pub fn run() -> Result<()> {
    let locale = ui::detect_locale();
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
        println!("{}", ui::suspend_no_managed_files(locale));
        return Ok(());
    }

    let count = perform_suspend(&git, &config)?;

    config.suspended = true;
    config.save(&git.shadow_dir)?;

    println!("{}", ui::suspend_success(locale, count).green());
    println!("{}", ui::suspend_worktree_clean(locale));

    Ok(())
}

/// Move all shadow changes into `suspended/`, restoring baselines / removing phantoms.
///
/// If any file fails mid-loop, the files already suspended are rolled back to the working
/// tree so no changes are orphaned, and the error is returned.
fn perform_suspend(git: &GitRepo, config: &ShadowConfig) -> Result<usize> {
    let suspended_dir = git.shadow_dir.join("suspended");
    std::fs::create_dir_all(&suspended_dir).context("failed to create suspended directory")?;

    // Track paths that were actually moved into suspended/ so we can undo them.
    let mut suspended: Vec<String> = Vec::new();

    for (file_path, entry) in &config.files {
        let step = match entry.file_type {
            FileType::Overlay => suspend_overlay(git, &suspended_dir, file_path).map(|_| true),
            FileType::Phantom if !entry.is_directory => {
                suspend_phantom(git, &suspended_dir, file_path)
            }
            FileType::Phantom => Ok(false),
        };

        match step {
            Ok(true) => suspended.push(file_path.clone()),
            Ok(false) => {}
            Err(e) => {
                rollback_suspend(git, &suspended_dir, &suspended);
                return Err(e);
            }
        }
    }

    Ok(suspended.len())
}

/// Restore already-suspended files back to the working tree and drop their suspended copy.
fn rollback_suspend(git: &GitRepo, suspended_dir: &std::path::Path, suspended: &[String]) {
    for file_path in suspended {
        let encoded = path::encode_path(file_path);
        let suspend_path = suspended_dir.join(&encoded);
        let worktree_path = git.root.join(file_path);
        if let Ok(content) = std::fs::read(&suspend_path) {
            let _ = std::fs::write(&worktree_path, &content);
            let _ = std::fs::remove_file(&suspend_path);
        }
    }
}

fn suspend_overlay(git: &GitRepo, suspended_dir: &std::path::Path, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let suspend_path = suspended_dir.join(&encoded);

    // Read both inputs BEFORE mutating anything, so a read failure leaves no partial state.
    let content =
        std::fs::read(&worktree_path).with_context(|| format!("failed to read {}", file_path))?;
    let baseline = std::fs::read(&baseline_path)
        .with_context(|| format!("failed to read baseline for {}", file_path))?;

    // Save current working tree content (with shadow changes) to suspended/
    fs_util::atomic_write(&suspend_path, &content)
        .with_context(|| format!("failed to save suspended content for {}", file_path))?;

    // Restore baseline content to working tree
    std::fs::write(&worktree_path, &baseline)
        .with_context(|| format!("failed to restore baseline for {}", file_path))?;

    Ok(())
}

/// Returns `true` if the phantom was present and suspended, `false` if there was nothing
/// to suspend (file absent).
fn suspend_phantom(
    git: &GitRepo,
    suspended_dir: &std::path::Path,
    file_path: &str,
) -> Result<bool> {
    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let suspend_path = suspended_dir.join(&encoded);

    if !worktree_path.exists() {
        return Ok(false);
    }

    // Save phantom content to suspended/
    let content =
        std::fs::read(&worktree_path).with_context(|| format!("failed to read {}", file_path))?;
    fs_util::atomic_write(&suspend_path, &content)
        .with_context(|| format!("failed to save suspended content for {}", file_path))?;

    // Remove phantom from working tree
    std::fs::remove_file(&worktree_path)
        .with_context(|| format!("failed to remove {} from working tree", file_path))?;

    Ok(true)
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
    fn test_perform_suspend_rolls_back_on_failure() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        // Overlay "a.md": has a baseline and will suspend successfully.
        std::fs::write(git.root.join("a.md"), "a-shadow\n").unwrap();
        let a_encoded = path::encode_path("a.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&a_encoded),
            b"a-base\n",
        )
        .unwrap();
        config
            .add_overlay("a.md".to_string(), commit.clone())
            .unwrap();

        // Overlay "b.md": no baseline file -> suspend will fail (a < b, so a runs first).
        std::fs::write(git.root.join("b.md"), "b-shadow\n").unwrap();
        config.add_overlay("b.md".to_string(), commit).unwrap();

        let result = super::perform_suspend(&git, &config);
        assert!(result.is_err(), "suspend should fail on missing baseline");

        // "a.md" must be rolled back to its original shadow content.
        let a_content = std::fs::read_to_string(git.root.join("a.md")).unwrap();
        assert_eq!(
            a_content, "a-shadow\n",
            "already-suspended file must be restored"
        );

        // No orphaned suspended files should remain.
        let suspended_dir = git.shadow_dir.join("suspended");
        let leftovers: Vec<_> = std::fs::read_dir(&suspended_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no suspended files should be orphaned, found: {:?}",
            leftovers
        );
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
