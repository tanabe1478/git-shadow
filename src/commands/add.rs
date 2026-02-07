use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{ExcludeMode, ShadowConfig};
use crate::error::ShadowError;
use crate::exclude::ExcludeManager;
use crate::git::GitRepo;
use crate::{fs_util, path};

pub fn run(file: &str, phantom: bool, no_exclude: bool, force: bool) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let normalized = path::normalize_path(file, &git.root)?;

    // Warn if hooks not installed
    if !git.hooks_installed() {
        eprintln!(
            "{}",
            "⚠ hooks がインストールされていません。`git-shadow install` を実行してください"
                .yellow()
        );
    }

    let mut config = ShadowConfig::load(&git.shadow_dir)?;

    if phantom {
        add_phantom(&git, &mut config, &normalized, no_exclude)?;
    } else {
        add_overlay(&git, &mut config, &normalized, force)?;
    }

    config.save(&git.shadow_dir)?;
    Ok(())
}

fn add_overlay(
    git: &GitRepo,
    config: &mut ShadowConfig,
    normalized: &str,
    force: bool,
) -> Result<()> {
    // Check file is tracked
    if !git.is_tracked(normalized)? {
        return Err(ShadowError::FileNotTracked(normalized.to_string()).into());
    }

    let file_path = git.root.join(normalized);

    // Binary check
    if fs_util::is_binary(&file_path)? {
        return Err(ShadowError::BinaryFile(normalized.to_string()).into());
    }

    // Size check
    fs_util::check_size(&file_path, force)?;

    // Get HEAD content as baseline
    let commit = git.head_commit()?;
    let baseline_content = git.show_file("HEAD", normalized)?;

    // Save baseline
    let encoded = path::encode_path(normalized);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    fs_util::atomic_write(&baseline_path, &baseline_content).context("ベースラインの保存に失敗")?;

    // Add to config
    config.add_overlay(normalized.to_string(), commit)?;

    println!(
        "{} を overlay として登録しました (ベースライン: {})",
        normalized,
        &config
            .get(normalized)
            .unwrap()
            .baseline_commit
            .as_deref()
            .unwrap_or("?")[..7]
    );
    Ok(())
}

fn add_phantom(
    git: &GitRepo,
    config: &mut ShadowConfig,
    normalized: &str,
    no_exclude: bool,
) -> Result<()> {
    // Phantom files should NOT be tracked
    if git.is_tracked(normalized)? {
        return Err(anyhow::anyhow!(
            "ファイル '{}' は既に Git で追跡されています。overlay として登録するには --phantom を外してください",
            normalized
        ));
    }

    let exclude_mode = if no_exclude {
        ExcludeMode::None
    } else {
        // Add to .git/info/exclude
        let manager = ExcludeManager::new(&git.git_dir);
        manager
            .add_entry(normalized)
            .context(".git/info/exclude への追加に失敗")?;
        ExcludeMode::GitInfoExclude
    };

    config.add_phantom(normalized.to_string(), exclude_mode)?;

    println!("{} を phantom として登録しました", normalized);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        std::fs::write(root.join("CLAUDE.md"), "# Team CLAUDE\n").unwrap();
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

        // Initialize shadow directory
        std::fs::create_dir_all(repo.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("stash")).unwrap();

        (dir, repo)
    }

    #[test]
    fn test_add_overlay_creates_config_entry() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        add_overlay(&git, &mut config, "CLAUDE.md", false).unwrap();

        let entry = config.get("CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, crate::config::FileType::Overlay);
        assert!(entry.baseline_commit.is_some());
    }

    #[test]
    fn test_add_overlay_saves_baseline() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        add_overlay(&git, &mut config, "CLAUDE.md", false).unwrap();

        let baseline = git.shadow_dir.join("baselines").join("CLAUDE.md");
        assert!(baseline.exists());
        let content = std::fs::read_to_string(&baseline).unwrap();
        assert_eq!(content, "# Team CLAUDE\n");
    }

    #[test]
    fn test_add_overlay_rejects_untracked() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("new.md"), "new").unwrap();
        let mut config = ShadowConfig::new();
        let result = add_overlay(&git, &mut config, "new.md", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_overlay_rejects_binary() {
        let (_dir, git) = make_test_repo();
        // Create and commit a binary file
        let mut content = b"hello".to_vec();
        content.push(0x00);
        std::fs::write(git.root.join("bin.dat"), &content).unwrap();
        std::process::Command::new("git")
            .args(["add", "bin.dat"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add binary"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let mut config = ShadowConfig::new();
        let result = add_overlay(&git, &mut config, "bin.dat", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_overlay_rejects_duplicate() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        add_overlay(&git, &mut config, "CLAUDE.md", false).unwrap();
        let result = add_overlay(&git, &mut config, "CLAUDE.md", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_phantom_creates_config_entry() {
        let (_dir, git) = make_test_repo();
        // Create a phantom file (not tracked)
        let phantom_dir = git.root.join("src").join("components");
        std::fs::create_dir_all(&phantom_dir).unwrap();
        std::fs::write(phantom_dir.join("CLAUDE.md"), "# Local\n").unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "src/components/CLAUDE.md", false).unwrap();

        let entry = config.get("src/components/CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, crate::config::FileType::Phantom);
        assert_eq!(entry.exclude_mode, ExcludeMode::GitInfoExclude);
    }

    #[test]
    fn test_add_phantom_adds_to_exclude() {
        let (_dir, git) = make_test_repo();
        std::fs::create_dir_all(git.root.join("src")).unwrap();
        std::fs::write(git.root.join("src/CLAUDE.md"), "# Local\n").unwrap();
        // Ensure info dir exists
        std::fs::create_dir_all(git.git_dir.join("info")).unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "src/CLAUDE.md", false).unwrap();

        let manager = ExcludeManager::new(&git.git_dir);
        let entries = manager.list_entries().unwrap();
        assert!(entries.contains(&"src/CLAUDE.md".to_string()));
    }

    #[test]
    fn test_add_phantom_no_exclude() {
        let (_dir, git) = make_test_repo();
        std::fs::create_dir_all(git.root.join("src")).unwrap();
        std::fs::write(git.root.join("src/CLAUDE.md"), "# Local\n").unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "src/CLAUDE.md", true).unwrap();

        let entry = config.get("src/CLAUDE.md").unwrap();
        assert_eq!(entry.exclude_mode, ExcludeMode::None);
    }

    #[test]
    fn test_add_phantom_rejects_tracked() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let result = add_phantom(&git, &mut config, "CLAUDE.md", false);
        assert!(result.is_err());
    }
}
