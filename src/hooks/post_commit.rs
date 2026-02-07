use anyhow::Result;
use colored::Colorize;

use crate::config::ShadowConfig;
use crate::git::GitRepo;
use crate::lock;
use crate::path;

pub fn handle(git: &GitRepo) -> Result<()> {
    let _config = ShadowConfig::load(&git.shadow_dir)?;
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

        // Best-effort restore
        match std::fs::read(&stash_path) {
            Ok(content) => match std::fs::write(&worktree_path, &content) {
                Ok(_) => {
                    // Successfully restored, remove stash entry
                    let _ = std::fs::remove_file(&stash_path);
                }
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!("⚠ {} の復元に失敗しました: {}", normalized, e).yellow()
                    );
                    failed.push(normalized.clone());
                }
            },
            Err(e) => {
                eprintln!(
                    "{}",
                    format!("⚠ {} の stash 読み込みに失敗しました: {}", normalized, e).yellow()
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
            "⚠ 一部のファイルの復元に失敗しました。git-shadow restore を実行してください".yellow()
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
