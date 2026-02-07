use anyhow::Result;

use crate::config::{FileType, ShadowConfig};
use crate::diff_util;
use crate::git::GitRepo;
use crate::path;

pub fn run(file: Option<&str>) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    if config.files.is_empty() {
        println!("管理対象ファイルはありません");
        return Ok(());
    }

    let mut found = false;

    for (file_path, entry) in &config.files {
        if let Some(target) = file {
            let normalized = path::normalize_path(target, &git.root)?;
            if *file_path != normalized {
                continue;
            }
        }
        found = true;

        match entry.file_type {
            FileType::Overlay => {
                show_overlay_diff(&git, file_path)?;
            }
            FileType::Phantom => {
                show_phantom_diff(&git, file_path)?;
            }
        }
    }

    if !found {
        if let Some(target) = file {
            println!("{} は shadow 管理対象ではありません", target);
        }
    }

    Ok(())
}

fn show_overlay_diff(git: &GitRepo, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
    let current = std::fs::read_to_string(&worktree_path).unwrap_or_default();

    if baseline == current {
        println!("{}: shadow 変更なし", file_path);
        return Ok(());
    }

    diff_util::print_colored_diff(
        &baseline,
        &current,
        &format!("a/{} (baseline)", file_path),
        &format!("b/{} (shadow)", file_path),
    );

    Ok(())
}

fn show_phantom_diff(git: &GitRepo, file_path: &str) -> Result<()> {
    let worktree_path = git.root.join(file_path);

    if !worktree_path.exists() {
        println!("{}: ファイルが存在しません", file_path);
        return Ok(());
    }

    let content = std::fs::read_to_string(&worktree_path).unwrap_or_default();
    diff_util::print_new_file_diff(&content, file_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{ExcludeMode, ShadowConfig};
    use crate::diff_util;
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
    fn test_overlay_diff_shows_changes() {
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

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        // Generate diff
        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        let current = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        let diff = diff_util::unified_diff(
            &baseline,
            &current,
            "a/CLAUDE.md (baseline)",
            "b/CLAUDE.md (shadow)",
        );

        assert!(diff.contains("+# My shadow"));
        assert!(diff.contains("--- a/CLAUDE.md (baseline)"));
        assert!(diff.contains("+++ b/CLAUDE.md (shadow)"));
    }

    #[test]
    fn test_overlay_no_changes() {
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

        // No shadow changes - content matches baseline
        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        let current = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();

        assert_eq!(baseline, current);
    }

    #[test]
    fn test_phantom_shows_full_content() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        std::fs::write(git.root.join("local.md"), "# Local\nline2\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // For phantom, we show all content as new
        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert!(content.contains("# Local"));
        assert!(content.contains("line2"));
    }

    #[test]
    fn test_diff_specific_file() {
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

        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None)
            .unwrap();

        config.save(&git.shadow_dir).unwrap();

        // Verify we can match specific file
        let normalized = path::normalize_path("CLAUDE.md", &git.root).unwrap();
        assert_eq!(normalized, "CLAUDE.md");
        assert!(config.get(&normalized).is_some());
    }
}
