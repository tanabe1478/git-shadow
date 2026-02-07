use anyhow::Result;
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::git::GitRepo;
use crate::path;

pub fn handle(git: &GitRepo) -> Result<()> {
    let config = ShadowConfig::load(&git.shadow_dir)?;
    let head = git.head_commit()?;

    for (file_path, entry) in &config.files {
        if entry.file_type != FileType::Overlay {
            continue;
        }

        if let Some(ref baseline_commit) = entry.baseline_commit {
            if *baseline_commit == head {
                continue;
            }

            // Check if file content actually changed
            let encoded = path::encode_path(file_path);
            let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
            if let Ok(baseline_content) = std::fs::read(&baseline_path) {
                if let Ok(head_content) = git.show_file("HEAD", file_path) {
                    if baseline_content != head_content {
                        eprintln!(
                            "{}",
                            format!(
                                "warning: baseline for {} is outdated.\n  Run `git-shadow rebase {}`",
                                file_path, file_path
                            )
                            .yellow()
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShadowConfig;
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
    fn test_no_warning_when_baseline_matches() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();

        // Save baseline
        let content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join("CLAUDE.md"),
            &content,
        )
        .unwrap();

        config.save(&git.shadow_dir).unwrap();

        // Should not error
        handle(&git).unwrap();
    }

    #[test]
    fn test_detects_baseline_drift() {
        let (_dir, git) = make_test_repo();
        let old_commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), old_commit)
            .unwrap();

        // Save old baseline
        let content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join("CLAUDE.md"),
            &content,
        )
        .unwrap();

        config.save(&git.shadow_dir).unwrap();

        // Make a new commit that changes the file
        std::fs::write(git.root.join("CLAUDE.md"), "# Updated Team\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "update"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        // Should not error (warnings go to stderr)
        handle(&git).unwrap();
    }
}
