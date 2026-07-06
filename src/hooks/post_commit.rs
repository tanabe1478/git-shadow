use anyhow::Result;
use colored::Colorize;

use crate::git::GitRepo;
use crate::lock;
use crate::path;
use crate::ui;

/// Decide whether restoring `stash_content` to `worktree_path` would overwrite edits
/// the user made after the commit object was written.
///
/// Safe to overwrite when the current worktree content is missing, already equal to the
/// stash content, or still equal to the baseline that pre-commit restored (overlays).
/// Otherwise the worktree diverged from both and must be preserved.
fn would_overwrite_edits(
    git: &GitRepo,
    normalized: &str,
    worktree_path: &std::path::Path,
    stash_content: &[u8],
) -> bool {
    let Ok(current) = std::fs::read(worktree_path) else {
        // Nothing on disk to overwrite.
        return false;
    };

    if current == stash_content {
        // Already matches what we would write.
        return false;
    }

    // For overlays, pre-commit leaves the baseline in the worktree; that is the expected
    // state and is safe to overwrite with the shadow content.
    let baseline_path = git
        .shadow_dir
        .join("baselines")
        .join(path::encode_path(normalized));
    if let Ok(baseline) = std::fs::read(&baseline_path) {
        if current == baseline {
            return false;
        }
    }

    // Current content differs from both the stash and the expected baseline: user edited it.
    true
}

pub fn handle(git: &GitRepo) -> Result<()> {
    let stash_dir = git.shadow_dir.join("stash");

    // If no stash directory or no files, nothing to do (e.g. --no-verify)
    if !stash_dir.exists() {
        return Ok(());
    }

    let stash_files: Vec<_> = std::fs::read_dir(&stash_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect();

    if stash_files.is_empty() {
        lock::release_lock(&git.shadow_dir)?;
        return Ok(());
    }

    let mut failed = Vec::new();

    for entry in &stash_files {
        let filename = entry.file_name();
        let encoded = filename.to_string_lossy();
        let normalized = path::decode_path(&encoded);

        let worktree_path = git.root.join(&normalized);
        let stash_path = entry.path();

        // Read the stashed shadow content first.
        let content = match std::fs::read(&stash_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!(
                    "{}",
                    ui::post_commit_read_stash_failed(
                        ui::detect_locale(),
                        &normalized,
                        &e.to_string(),
                    )
                    .yellow()
                );
                failed.push(normalized.clone());
                continue;
            }
        };

        // Overwrite safety: if the worktree file was edited since pre-commit ran
        // (e.g. `git commit --no-verify` bypassed the StashRemaining guard), do not
        // clobber the user's edits. Leave the stash entry in place and warn.
        if would_overwrite_edits(git, &normalized, &worktree_path, &content) {
            eprintln!(
                "{}",
                ui::post_commit_restore_conflict(ui::detect_locale(), &normalized).yellow()
            );
            failed.push(normalized.clone());
            continue;
        }

        // Ensure parent directories exist before writing (nested paths).
        if let Some(parent) = worktree_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "{}",
                    ui::post_commit_restore_failed(
                        ui::detect_locale(),
                        &normalized,
                        &e.to_string()
                    )
                    .yellow()
                );
                failed.push(normalized.clone());
                continue;
            }
        }

        // Best-effort restore
        match std::fs::write(&worktree_path, &content) {
            Ok(_) => {
                // Successfully restored, remove stash entry
                let _ = std::fs::remove_file(&stash_path);
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    ui::post_commit_restore_failed(
                        ui::detect_locale(),
                        &normalized,
                        &e.to_string()
                    )
                    .yellow()
                );
                failed.push(normalized.clone());
            }
        }
    }

    if failed.is_empty() {
        // All restored successfully
        lock::release_lock(&git.shadow_dir)?;
    } else {
        // Partial failure - keep lock
        eprintln!(
            "{}",
            ui::post_commit_partial_failure(ui::detect_locale()).yellow()
        );
        for f in &failed {
            eprintln!("  - {}", f);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fs_util, lock};

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
    fn test_restores_stashed_overlay() {
        let (_dir, git) = make_test_repo();

        // Simulate post pre-commit state: baseline in worktree, shadow in stash
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n").unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join("CLAUDE.md"),
            b"# Team\n",
        )
        .unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join("CLAUDE.md"),
            b"# Team\n# My shadow\n",
        )
        .unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        // Working tree should be restored
        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Team\n# My shadow\n");

        // Stash should be cleaned
        assert!(!git.shadow_dir.join("stash").join("CLAUDE.md").exists());

        // Lock should be released
        assert!(matches!(
            lock::check_lock(&git.shadow_dir).unwrap(),
            lock::LockStatus::Free
        ));
    }

    #[test]
    fn test_restores_stashed_phantom() {
        let (_dir, git) = make_test_repo();

        // Create phantom stash
        fs_util::atomic_write(&git.shadow_dir.join("stash").join("local.md"), b"# Local\n")
            .unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert_eq!(content, "# Local\n");

        assert!(matches!(
            lock::check_lock(&git.shadow_dir).unwrap(),
            lock::LockStatus::Free
        ));
    }

    #[test]
    fn test_no_stash_no_op() {
        let (_dir, git) = make_test_repo();
        // No stash files, no lock
        handle(&git).unwrap();
    }

    #[test]
    fn test_empty_stash_releases_lock() {
        let (_dir, git) = make_test_repo();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        assert!(matches!(
            lock::check_lock(&git.shadow_dir).unwrap(),
            lock::LockStatus::Free
        ));
    }

    #[test]
    fn test_skips_restore_when_worktree_edited_after_commit() {
        let (_dir, git) = make_test_repo();

        // Baseline is what pre-commit would have restored to the worktree.
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join("CLAUDE.md"),
            b"# Team\n",
        )
        .unwrap();
        // Stash holds the original shadow content.
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join("CLAUDE.md"),
            b"# Team\n# My shadow\n",
        )
        .unwrap();
        // User edited the file after committing (differs from baseline AND stash).
        std::fs::write(
            git.root.join("CLAUDE.md"),
            "# Team\n# User edit after commit\n",
        )
        .unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        // User's edit must be preserved.
        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Team\n# User edit after commit\n");

        // Stash entry must be kept for manual recovery.
        assert!(git.shadow_dir.join("stash").join("CLAUDE.md").exists());

        // Lock must be kept because restore did not complete cleanly.
        assert!(!matches!(
            lock::check_lock(&git.shadow_dir).unwrap(),
            lock::LockStatus::Free
        ));
    }

    #[test]
    fn test_restores_when_worktree_matches_baseline() {
        let (_dir, git) = make_test_repo();

        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join("CLAUDE.md"),
            b"# Team\n",
        )
        .unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join("CLAUDE.md"),
            b"# Team\n# My shadow\n",
        )
        .unwrap();
        // Worktree still holds the baseline pre-commit restored.
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n").unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Team\n# My shadow\n");
        assert!(!git.shadow_dir.join("stash").join("CLAUDE.md").exists());
    }

    #[test]
    fn test_creates_parent_dirs_when_restoring_nested_stash() {
        let (_dir, git) = make_test_repo();

        // Nested stash entry whose parent directory does not exist yet.
        let encoded = path::encode_path("src/deep/local.md");
        fs_util::atomic_write(&git.shadow_dir.join("stash").join(&encoded), b"# Nested\n").unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        let content = std::fs::read_to_string(git.root.join("src/deep/local.md")).unwrap();
        assert_eq!(content, "# Nested\n");
    }

    #[test]
    fn test_decodes_url_encoded_stash_path() {
        let (_dir, git) = make_test_repo();

        // Create stash with URL-encoded filename
        let encoded = path::encode_path("src/components/CLAUDE.md");
        std::fs::create_dir_all(git.root.join("src/components")).unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join(&encoded),
            b"# Component\n",
        )
        .unwrap();
        lock::acquire_lock(&git.shadow_dir).unwrap();

        handle(&git).unwrap();

        let content = std::fs::read_to_string(git.root.join("src/components/CLAUDE.md")).unwrap();
        assert_eq!(content, "# Component\n");
    }
}
