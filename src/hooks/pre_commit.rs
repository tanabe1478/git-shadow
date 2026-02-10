use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{FileEntry, FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::git::GitRepo;
use crate::lock;
use crate::{fs_util, path};

/// Tracks stashed files for rollback capability
struct PreCommitTransaction {
    stashed_overlays: Vec<String>, // normalized paths of overlay files stashed
    stashed_phantoms: Vec<String>, // normalized paths of phantom files stashed
    overwritten: Vec<String>,      // overlay files where baseline was restored
}

impl PreCommitTransaction {
    fn new() -> Self {
        Self {
            stashed_overlays: Vec::new(),
            stashed_phantoms: Vec::new(),
            overwritten: Vec::new(),
        }
    }

    /// Best-effort rollback: restore stashed files to working tree
    fn rollback(&self, git: &GitRepo) {
        for file_path in self
            .stashed_overlays
            .iter()
            .chain(self.stashed_phantoms.iter())
        {
            let encoded = path::encode_path(file_path);
            let stash_path = git.shadow_dir.join("stash").join(&encoded);
            let worktree_path = git.root.join(file_path);

            if stash_path.exists() {
                if let Ok(content) = std::fs::read(&stash_path) {
                    let _ = std::fs::write(&worktree_path, &content);
                    let _ = std::fs::remove_file(&stash_path);
                }
            }
        }

        // Re-stage overlay files that were overwritten with baseline
        for file_path in &self.overwritten {
            let _ = git.add(file_path);
        }
    }
}

pub fn handle(git: &GitRepo) -> Result<()> {
    // 0. Acquire lock
    lock::acquire_lock(&git.shadow_dir).map_err(|e| {
        // Convert StaleLock to anyhow with context
        anyhow::anyhow!("{}", e)
    })?;

    let config = ShadowConfig::load(&git.shadow_dir)?;

    if config.files.is_empty() {
        lock::release_lock(&git.shadow_dir)?;
        return Ok(());
    }

    // 1. Integrity checks
    if let Err(e) = run_hard_checks(git, &config) {
        lock::release_lock(&git.shadow_dir).ok();
        return Err(e);
    }
    run_soft_checks(git, &config);

    // 2. Partial staging detection
    if let Err(e) = detect_partial_staging(git, &config) {
        lock::release_lock(&git.shadow_dir).ok();
        return Err(e);
    }

    // 3-4. Process files with rollback
    let mut tx = PreCommitTransaction::new();
    if let Err(e) = process_files(git, &config, &mut tx) {
        tx.rollback(git);
        lock::release_lock(&git.shadow_dir).ok();
        return Err(e);
    }

    // Success - lock stays for post-commit to release
    Ok(())
}

fn run_hard_checks(git: &GitRepo, config: &ShadowConfig) -> Result<()> {
    // Check stash remnants
    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let has_files = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false));
        if has_files {
            return Err(ShadowError::StashRemaining.into());
        }
    }

    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                // Check file exists
                if !git.root.join(file_path).exists() {
                    return Err(ShadowError::FileMissing(file_path.clone()).into());
                }
                // Check baseline exists
                let encoded = path::encode_path(file_path);
                let baseline = git.shadow_dir.join("baselines").join(&encoded);
                if !baseline.exists() {
                    return Err(ShadowError::BaselineMissing(file_path.clone()).into());
                }
            }
            FileType::Phantom => {}
        }
    }

    Ok(())
}

fn run_soft_checks(git: &GitRepo, config: &ShadowConfig) {
    let head = git.head_commit().ok();

    for (file_path, entry) in &config.files {
        if entry.file_type == FileType::Overlay {
            if let (Some(ref baseline_commit), Some(ref current_head)) =
                (&entry.baseline_commit, &head)
            {
                if baseline_commit != current_head {
                    eprintln!(
                        "{}",
                        format!(
                            "warning: baseline for {} is outdated. Run `git-shadow rebase {}`",
                            file_path, file_path
                        )
                        .yellow()
                    );
                }
            }
        }
    }
}

fn detect_partial_staging(git: &GitRepo, config: &ShadowConfig) -> Result<()> {
    for (file_path, entry) in &config.files {
        if entry.file_type == FileType::Overlay {
            let (index_changed, worktree_changed) = git.staging_status(file_path)?;
            if index_changed && worktree_changed {
                return Err(ShadowError::PartialStage(file_path.clone()).into());
            }
        }
    }
    Ok(())
}

fn process_files(
    git: &GitRepo,
    config: &ShadowConfig,
    tx: &mut PreCommitTransaction,
) -> Result<()> {
    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                process_overlay(git, file_path, tx)?;
            }
            FileType::Phantom => {
                process_phantom(git, file_path, entry, tx)?;
            }
        }
    }
    Ok(())
}

fn process_overlay(git: &GitRepo, file_path: &str, tx: &mut PreCommitTransaction) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let stash_path = git.shadow_dir.join("stash").join(&encoded);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);

    // a. Stash current content
    let content =
        std::fs::read(&worktree_path).with_context(|| format!("failed to read {}", file_path))?;
    fs_util::atomic_write(&stash_path, &content)
        .with_context(|| format!("failed to stash {}", file_path))?;
    tx.stashed_overlays.push(file_path.to_string());

    // b. Restore baseline
    let baseline = std::fs::read(&baseline_path)
        .with_context(|| format!("failed to read baseline for {}", file_path))?;
    std::fs::write(&worktree_path, &baseline)
        .with_context(|| format!("failed to restore baseline for {}", file_path))?;
    tx.overwritten.push(file_path.to_string());

    // c. Stage the baseline content
    git.add(file_path)
        .map_err(|e| anyhow::anyhow!("{}", e))
        .with_context(|| format!("failed to stage {}", file_path))?;

    Ok(())
}

fn process_phantom(
    git: &GitRepo,
    file_path: &str,
    entry: &FileEntry,
    tx: &mut PreCommitTransaction,
) -> Result<()> {
    if entry.is_directory {
        // Directory phantoms: no stash needed, just unstage
        git.unstage_phantom(file_path)?;
        return Ok(());
    }

    let encoded = path::encode_path(file_path);
    let worktree_path = git.root.join(file_path);
    let stash_path = git.shadow_dir.join("stash").join(&encoded);

    // a. Stash current content (if file exists)
    if worktree_path.exists() {
        let content = std::fs::read(&worktree_path)
            .with_context(|| format!("failed to read {}", file_path))?;
        fs_util::atomic_write(&stash_path, &content)
            .with_context(|| format!("failed to stash {}", file_path))?;
        tx.stashed_phantoms.push(file_path.to_string());
    }

    // b. Unstage from index
    git.unstage_phantom(file_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExcludeMode, ShadowConfig};
    use crate::lock::LockStatus;

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

        // Create and commit a file
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

        // Initialize shadow
        std::fs::create_dir_all(repo.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("stash")).unwrap();

        (dir, repo)
    }

    fn setup_overlay(git: &GitRepo) -> ShadowConfig {
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();

        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My additions\n").unwrap();

        config.save(&git.shadow_dir).unwrap();
        config
    }

    #[test]
    fn test_overlay_stashes_and_restores_baseline() {
        let (_dir, git) = make_test_repo();
        let _config = setup_overlay(&git);

        handle(&git).unwrap();

        // Working tree should have baseline content
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(wt, "# Team\n");

        // Stash should have shadow content
        let stash =
            std::fs::read_to_string(git.shadow_dir.join("stash").join("CLAUDE.md")).unwrap();
        assert_eq!(stash, "# Team\n# My additions\n");

        // Cleanup for test
        lock::release_lock(&git.shadow_dir).unwrap();
    }

    #[test]
    fn test_phantom_stashes_and_unstages() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create phantom file
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Stage it (simulating accidental git add)
        std::process::Command::new("git")
            .args(["add", "local.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        handle(&git).unwrap();

        // Stash should have phantom content
        let stash = std::fs::read_to_string(git.shadow_dir.join("stash").join("local.md")).unwrap();
        assert_eq!(stash, "# Local\n");

        lock::release_lock(&git.shadow_dir).unwrap();
    }

    #[test]
    fn test_partial_staging_blocks_commit() {
        let (_dir, git) = make_test_repo();
        let _config = setup_overlay(&git);

        // Create partial staging: stage one change, then modify again
        std::fs::write(git.root.join("CLAUDE.md"), "# Staged\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::fs::write(git.root.join("CLAUDE.md"), "# Partial\n").unwrap();

        let result = handle(&git);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("partial staging"));
    }

    #[test]
    fn test_stash_remnants_blocks_commit() {
        let (_dir, git) = make_test_repo();
        let _config = setup_overlay(&git);

        // Manually create stash remnant
        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "remnant").unwrap();

        let result = handle(&git);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("stash"));
    }

    #[test]
    fn test_missing_file_blocks_commit() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();

        // Save baseline but delete the working file
        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config.save(&git.shadow_dir).unwrap();

        std::fs::remove_file(git.root.join("CLAUDE.md")).unwrap();

        let result = handle(&git);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("does not exist in the working tree"));
    }

    #[test]
    fn test_missing_baseline_blocks_commit() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        // Don't create baseline file
        config.save(&git.shadow_dir).unwrap();

        let result = handle(&git);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("baseline missing"));
    }

    #[test]
    fn test_phantom_directory_skips_stash() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create phantom directory with files inside
        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        std::fs::write(git.root.join(".claude/settings.json"), r#"{"key": "val"}"#).unwrap();
        std::fs::write(git.root.join(".claude/notes.md"), "# Notes\n").unwrap();

        config
            .add_phantom(".claude".to_string(), ExcludeMode::None, true)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Stage the directory files
        std::process::Command::new("git")
            .args(["add", ".claude/"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        handle(&git).unwrap();

        // Directory should still exist in worktree
        assert!(git.root.join(".claude").is_dir());
        assert!(git.root.join(".claude/settings.json").exists());
        assert!(git.root.join(".claude/notes.md").exists());

        // No stash entry for the directory
        let stash_dir = git.shadow_dir.join("stash");
        let stash_files: Vec<_> = std::fs::read_dir(&stash_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        assert!(
            stash_files.is_empty(),
            "No stash entries should exist for directory phantoms"
        );

        lock::release_lock(&git.shadow_dir).unwrap();
    }

    #[test]
    fn test_empty_config_releases_lock() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        // Lock should be released
        let status = lock::check_lock(&git.shadow_dir).unwrap();
        assert!(matches!(status, LockStatus::Free));
    }
}
