use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{ExcludeMode, ShadowConfig};
use crate::error::ShadowError;
use crate::exclude::ExcludeManager;
use crate::git::GitRepo;
use crate::ui;
use crate::{fs_util, path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddMode {
    Auto,
    Overlay,
    Phantom,
}

pub fn run(
    files: &[String],
    overlay: bool,
    phantom: bool,
    no_exclude: bool,
    force: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let git = GitRepo::discover(&cwd)?;

    let mode = if overlay {
        AddMode::Overlay
    } else if phantom {
        AddMode::Phantom
    } else {
        AddMode::Auto
    };

    run_with_repo(&git, &cwd, files, mode, no_exclude, force)
}

fn run_with_repo(
    git: &GitRepo,
    cwd: &Path,
    files: &[String],
    mode: AddMode,
    no_exclude: bool,
    force: bool,
) -> Result<()> {
    let locale = ui::detect_locale();
    git.ensure_initialized()?;

    if !git.hooks_installed() {
        eprintln!("{}", ui::warning_hooks_not_installed(locale).yellow());
    }

    let mut config = ShadowConfig::load(&git.shadow_dir)?;
    let mut failures = 0usize;

    for file in files {
        let normalized = path::normalize_path(file, cwd, &git.root)?;

        let resolved_mode = match resolve_mode(git, &normalized, mode) {
            Ok(resolved_mode) => resolved_mode,
            Err(err) => {
                failures += 1;
                eprintln!(
                    "{}",
                    ui::add_failed(locale, &normalized, &ui::format_error(&err, locale)).red()
                );
                continue;
            }
        };

        let result = match resolved_mode {
            AddMode::Overlay => add_overlay(git, &mut config, &normalized, force),
            AddMode::Phantom => add_phantom(git, &mut config, &normalized, no_exclude),
            AddMode::Auto => unreachable!("auto mode should be resolved before registration"),
        };

        if let Err(err) = result {
            failures += 1;
            eprintln!(
                "{}",
                ui::add_failed(locale, &normalized, &ui::format_error(&err, locale)).red()
            );
        }
    }

    config.save(&git.shadow_dir)?;

    if failures > 0 {
        return Err(ShadowError::AddSomeFailed(failures).into());
    }

    Ok(())
}

fn resolve_mode(git: &GitRepo, normalized: &str, mode: AddMode) -> Result<AddMode> {
    match mode {
        AddMode::Overlay => Ok(AddMode::Overlay),
        AddMode::Phantom => Ok(AddMode::Phantom),
        AddMode::Auto => {
            if git.is_tracked(normalized)? {
                return Ok(AddMode::Overlay);
            }

            if git.root.join(normalized).exists() {
                return Ok(AddMode::Phantom);
            }

            Err(ShadowError::PathDoesNotExist(normalized.to_string()).into())
        }
    }
}

fn add_overlay(
    git: &GitRepo,
    config: &mut ShadowConfig,
    normalized: &str,
    force: bool,
) -> Result<()> {
    let locale = ui::detect_locale();

    if !git.is_tracked(normalized)? {
        return Err(ShadowError::FileNotTracked(normalized.to_string()).into());
    }

    let file_path = git.root.join(normalized);

    if fs_util::is_binary(&file_path)? {
        return Err(ShadowError::BinaryFile(normalized.to_string()).into());
    }

    fs_util::check_size(&file_path, force)?;

    let commit = git.head_commit()?;
    let baseline_content = git.show_file("HEAD", normalized)?;

    let encoded = path::encode_path(normalized);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    fs_util::atomic_write(&baseline_path, &baseline_content).context("failed to save baseline")?;

    config.add_overlay(normalized.to_string(), commit)?;

    println!(
        "{}",
        ui::registered_overlay(
            locale,
            normalized,
            &config
                .get(normalized)
                .unwrap()
                .baseline_commit
                .as_deref()
                .unwrap_or("?")[..7],
        )
    );
    Ok(())
}

fn add_phantom(
    git: &GitRepo,
    config: &mut ShadowConfig,
    normalized: &str,
    no_exclude: bool,
) -> Result<()> {
    if git.is_tracked(normalized)? {
        return Err(ShadowError::TrackedFileNeedsOverlay(normalized.to_string()).into());
    }

    let full_path = git.root.join(normalized);
    let is_dir = full_path.is_dir();

    let exclude_mode = if no_exclude {
        ExcludeMode::None
    } else {
        let exclude_path = if is_dir {
            format!("{}/", normalized)
        } else {
            normalized.to_string()
        };
        let manager = ExcludeManager::new(&git.common_dir);
        manager
            .add_entry(&exclude_path)
            .context("failed to add to .git/info/exclude")?;
        ExcludeMode::GitInfoExclude
    };

    config.add_phantom(normalized.to_string(), exclude_mode, is_dir)?;

    if is_dir {
        println!(
            "{}",
            ui::registered_phantom_directory(ui::detect_locale(), normalized)
        );
    } else {
        println!(
            "{}",
            ui::registered_phantom(ui::detect_locale(), normalized)
        );
    }
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
        std::fs::create_dir_all(repo.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("stash")).unwrap();

        (dir, repo)
    }

    #[test]
    fn test_resolve_mode_auto_overlay_for_tracked() {
        let (_dir, git) = make_test_repo();
        let mode = resolve_mode(&git, "CLAUDE.md", AddMode::Auto).unwrap();
        assert_eq!(mode, AddMode::Overlay);
    }

    #[test]
    fn test_resolve_mode_auto_phantom_for_existing_untracked() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        let mode = resolve_mode(&git, "local.md", AddMode::Auto).unwrap();
        assert_eq!(mode, AddMode::Phantom);
    }

    #[test]
    fn test_resolve_mode_auto_rejects_missing_path() {
        let (_dir, git) = make_test_repo();
        let err = resolve_mode(&git, "missing.md", AddMode::Auto).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
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
        std::fs::create_dir_all(git.common_dir.join("info")).unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "src/CLAUDE.md", false).unwrap();

        let manager = ExcludeManager::new(&git.common_dir);
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
    fn test_add_phantom_directory_creates_config_entry() {
        let (_dir, git) = make_test_repo();
        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        std::fs::write(git.root.join(".claude/settings.json"), "{}").unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, ".claude", false).unwrap();

        let entry = config.get(".claude").unwrap();
        assert_eq!(entry.file_type, crate::config::FileType::Phantom);
        assert!(entry.is_directory);
    }

    #[test]
    fn test_add_phantom_directory_adds_trailing_slash_to_exclude() {
        let (_dir, git) = make_test_repo();
        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        std::fs::write(git.root.join(".claude/notes.md"), "notes").unwrap();
        std::fs::create_dir_all(git.common_dir.join("info")).unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, ".claude", false).unwrap();

        let manager = ExcludeManager::new(&git.common_dir);
        let entries = manager.list_entries().unwrap();
        assert!(entries.contains(&".claude/".to_string()));
    }

    #[test]
    fn test_add_phantom_directory_no_exclude() {
        let (_dir, git) = make_test_repo();
        std::fs::create_dir_all(git.root.join("codemaps")).unwrap();
        std::fs::write(git.root.join("codemaps/map.json"), "{}").unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "codemaps", true).unwrap();

        let entry = config.get("codemaps").unwrap();
        assert!(entry.is_directory);
        assert_eq!(entry.exclude_mode, ExcludeMode::None);
    }

    #[test]
    fn test_add_phantom_file_not_directory() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();

        let mut config = ShadowConfig::new();
        add_phantom(&git, &mut config, "local.md", false).unwrap();

        let entry = config.get("local.md").unwrap();
        assert!(!entry.is_directory);
    }

    #[test]
    fn test_add_phantom_rejects_tracked() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let result = add_phantom(&git, &mut config, "CLAUDE.md", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_requires_install_before_overlay_add() {
        let (_dir, git) = make_test_repo();
        std::fs::remove_dir_all(&git.shadow_dir).unwrap();

        let files = vec!["CLAUDE.md".to_string()];
        let err =
            run_with_repo(&git, &git.root, &files, AddMode::Overlay, false, false).unwrap_err();
        assert!(err.to_string().contains("Run `git-shadow install`"));
    }

    #[test]
    fn test_run_requires_install_before_phantom_add() {
        let (_dir, git) = make_test_repo();
        std::fs::remove_dir_all(&git.shadow_dir).unwrap();
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();

        let files = vec!["local.md".to_string()];
        let err =
            run_with_repo(&git, &git.root, &files, AddMode::Phantom, false, false).unwrap_err();
        assert!(err.to_string().contains("Run `git-shadow install`"));
    }

    #[test]
    fn test_run_rejects_path_outside_repo() {
        let (_dir, git) = make_test_repo();
        let outside_dir = git.root.parent().unwrap().join("outside");
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("local.md"), "# Outside\n").unwrap();

        let cwd = git.root.join("src");
        std::fs::create_dir_all(&cwd).unwrap();

        let files = vec!["../../outside/local.md".to_string()];
        let err = run_with_repo(&git, &cwd, &files, AddMode::Phantom, false, false).unwrap_err();
        assert!(err.to_string().contains("outside repository"));
    }

    #[test]
    fn test_run_auto_detects_mixed_inputs() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();

        let files = vec!["CLAUDE.md".to_string(), "local.md".to_string()];
        run_with_repo(&git, &git.root, &files, AddMode::Auto, false, false).unwrap();

        let config = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert_eq!(
            config.get("CLAUDE.md").unwrap().file_type,
            crate::config::FileType::Overlay
        );
        assert_eq!(
            config.get("local.md").unwrap().file_type,
            crate::config::FileType::Phantom
        );
    }

    #[test]
    fn test_run_returns_error_when_any_input_fails() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();

        let files = vec!["local.md".to_string(), "missing.md".to_string()];
        let err = run_with_repo(&git, &git.root, &files, AddMode::Auto, false, false).unwrap_err();
        assert!(!err.to_string().is_empty());

        let config = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(config.get("local.md").is_some());
        assert!(config.get("missing.md").is_none());
    }
}
