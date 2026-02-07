use anyhow::Result;

use crate::git::GitRepo;
use crate::lock;
use crate::path;

pub fn run(file: Option<&str>) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let stash_dir = git.shadow_dir.join("stash");
    let mut restored = Vec::new();

    if stash_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();

        for entry in entries {
            let filename = entry.file_name();
            let encoded = filename.to_string_lossy().to_string();
            let normalized = path::decode_path(&encoded);

            // If a specific file is requested, skip others
            if let Some(target) = file {
                if normalized != target {
                    continue;
                }
            }

            let worktree_path = git.root.join(&normalized);
            let stash_path = entry.path();

            // Ensure parent directory exists
            if let Some(parent) = worktree_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let content = std::fs::read(&stash_path)?;
            std::fs::write(&worktree_path, &content)?;
            std::fs::remove_file(&stash_path)?;
            restored.push(normalized);
        }
    }

    // Remove stale lock
    let lock_removed = if git.shadow_dir.join("lock").exists() {
        lock::release_lock(&git.shadow_dir)?;
        true
    } else {
        false
    };

    // Print summary
    if restored.is_empty() && !lock_removed {
        println!("復旧するものはありません");
    } else {
        if !restored.is_empty() {
            println!("復元されたファイル:");
            for f in &restored {
                println!("  {}", f);
            }
        }
        if lock_removed {
            println!("lockfile を削除しました");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_util;

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
    fn test_restores_stashed_files() {
        let (_dir, git) = make_test_repo();

        // Put file in stash
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join("CLAUDE.md"),
            b"# Shadow content\n",
        )
        .unwrap();

        // Overwrite worktree with baseline
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n").unwrap();

        restore_for_test(&git, None);

        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Shadow content\n");
        assert!(!git.shadow_dir.join("stash").join("CLAUDE.md").exists());
    }

    #[test]
    fn test_restores_specific_file() {
        let (_dir, git) = make_test_repo();

        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join("CLAUDE.md"),
            b"# Shadow\n",
        )
        .unwrap();
        fs_util::atomic_write(&git.shadow_dir.join("stash").join("other.md"), b"# Other\n")
            .unwrap();

        restore_for_test(&git, Some("CLAUDE.md"));

        // CLAUDE.md restored
        assert!(!git.shadow_dir.join("stash").join("CLAUDE.md").exists());
        // other.md still in stash
        assert!(git.shadow_dir.join("stash").join("other.md").exists());
    }

    #[test]
    fn test_removes_stale_lock() {
        let (_dir, git) = make_test_repo();

        // Create stale lock
        std::fs::write(
            git.shadow_dir.join("lock"),
            "pid=999999\ntimestamp=2026-01-01T00:00:00+00:00",
        )
        .unwrap();

        restore_for_test(&git, None);

        assert!(!git.shadow_dir.join("lock").exists());
    }

    #[test]
    fn test_nothing_to_restore() {
        let (_dir, git) = make_test_repo();
        // Should not error
        restore_for_test(&git, None);
    }

    #[test]
    fn test_restores_nested_path() {
        let (_dir, git) = make_test_repo();

        let encoded = path::encode_path("src/components/CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("stash").join(&encoded),
            b"# Component\n",
        )
        .unwrap();

        restore_for_test(&git, None);

        let content = std::fs::read_to_string(git.root.join("src/components/CLAUDE.md")).unwrap();
        assert_eq!(content, "# Component\n");
    }

    /// Helper that runs restore logic directly (bypassing cwd discovery)
    fn restore_for_test(git: &GitRepo, file: Option<&str>) {
        let stash_dir = git.shadow_dir.join("stash");
        if stash_dir.exists() {
            let entries: Vec<_> = std::fs::read_dir(&stash_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .collect();

            for entry in entries {
                let filename = entry.file_name();
                let encoded = filename.to_string_lossy().to_string();
                let normalized = path::decode_path(&encoded);

                if let Some(target) = file {
                    if normalized != target {
                        continue;
                    }
                }

                let worktree_path = git.root.join(&normalized);
                if let Some(parent) = worktree_path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                let content = std::fs::read(entry.path()).unwrap();
                std::fs::write(&worktree_path, &content).unwrap();
                std::fs::remove_file(entry.path()).unwrap();
            }
        }

        if git.shadow_dir.join("lock").exists() {
            lock::release_lock(&git.shadow_dir).unwrap();
        }
    }
}
