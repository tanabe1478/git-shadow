use anyhow::Result;
use colored::Colorize;
use is_terminal::IsTerminal;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::exclude::ExcludeManager;
use crate::git::GitRepo;
use crate::path;
use crate::ui;

pub fn run(file: &str, force: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let cwd = std::env::current_dir()?;
    let git = GitRepo::discover(&cwd)?;
    let mut config = ShadowConfig::load(&git.shadow_dir)?;
    let normalized = path::normalize_path(file, &cwd, &git.root)?;

    let entry = config
        .get(&normalized)
        .ok_or_else(|| ShadowError::NotManaged(normalized.clone()))?
        .clone();

    // Confirmation prompt
    if !force {
        if !std::io::stdin().is_terminal() {
            return Err(ShadowError::NonInteractiveWithoutForce.into());
        }

        let prompt = match entry.file_type {
            FileType::Overlay => ui::remove_prompt_overlay(locale, &normalized),
            FileType::Phantom => {
                if entry.is_directory {
                    ui::remove_prompt_phantom_directory(locale, &normalized)
                } else {
                    ui::remove_prompt_phantom(locale, &normalized)
                }
            }
        };

        eprintln!("{}", prompt);
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("{}", ui::aborted(locale));
            return Ok(());
        }
    }

    if entry.file_type == FileType::Overlay {
        remove_overlay(&git, &normalized)?;
    }

    config.remove(&normalized)?;

    regenerate_exclude(&git, &config)?;

    config.save(&git.shadow_dir)?;

    println!("{}", ui::unregistered(locale, &normalized).green());

    Ok(())
}

/// Regenerate the shared `.git/info/exclude` managed section from the union of all
/// worktrees' configs.
///
/// This keeps entries that other worktrees still rely on (config is per-worktree but
/// `.git/info/exclude` is shared via `common_dir`), and only drops the removed entry if
/// no worktree needs it anymore.
fn regenerate_exclude(git: &GitRepo, config: &ShadowConfig) -> Result<()> {
    let manager = ExcludeManager::new(&git.common_dir);
    let patterns = crate::exclude::union_patterns(git, config);
    manager.set_entries(&patterns)?;
    Ok(())
}

fn remove_overlay(git: &GitRepo, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    // Restore baseline content to working tree
    if baseline_path.exists() {
        let baseline = std::fs::read(&baseline_path)?;
        std::fs::write(&worktree_path, &baseline)?;
        std::fs::remove_file(&baseline_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{ExcludeMode, ShadowConfig};
    use crate::exclude::ExcludeManager;
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
    fn test_remove_overlay_restores_baseline() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();

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

        // Remove overlay (bypass prompt via direct function call)
        remove_overlay_for_test(&git, "CLAUDE.md");

        // Working tree should have baseline content
        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(content, "# Team\n");

        // Baseline file should be deleted
        assert!(!git.shadow_dir.join("baselines").join(&encoded).exists());
    }

    #[test]
    fn test_remove_phantom_keeps_file() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create phantom file and register it, seeding the exclude section via the real
        // anchored path (mirrors add.rs).
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::GitInfoExclude, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();
        super::regenerate_exclude(&git, &config).unwrap();

        let manager = ExcludeManager::new(&git.common_dir);
        assert!(manager
            .list_entries()
            .unwrap()
            .contains(&"/local.md".to_string()));

        // Remove phantom via the real path.
        remove_phantom_for_test(&git, &mut config, "local.md");

        // File should still exist
        assert!(git.root.join("local.md").exists());
        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert_eq!(content, "# Local\n");

        // Anchored exclude entry should be removed
        let entries = manager.list_entries().unwrap();
        assert!(!entries.contains(&"/local.md".to_string()));
    }

    #[test]
    fn test_remove_phantom_no_exclude_skips_exclude() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Remove phantom (ExcludeMode::None -> nothing to add/drop in exclude)
        remove_phantom_for_test(&git, &mut config, "local.md");

        // Should not error - file still exists
        assert!(git.root.join("local.md").exists());
    }

    #[test]
    fn test_remove_updates_config() {
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

        // Remove overlay
        remove_overlay_for_test(&git, "CLAUDE.md");
        config.remove("CLAUDE.md").unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Reload and verify
        let reloaded = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(reloaded.get("CLAUDE.md").is_none());
        assert!(reloaded.files.is_empty());
    }

    #[test]
    fn test_remove_not_managed_errors() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        let result = config.get("nonexistent.md");
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_overlay_nested_path() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create nested file in git
        std::fs::create_dir_all(git.root.join("src/components")).unwrap();
        std::fs::write(git.root.join("src/components/CLAUDE.md"), "# Component\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "src/components/CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add component"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let commit = git.head_commit().unwrap();
        let baseline_content = git.show_file("HEAD", "src/components/CLAUDE.md").unwrap();
        let encoded = path::encode_path("src/components/CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config
            .add_overlay("src/components/CLAUDE.md".to_string(), commit)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Add shadow changes
        std::fs::write(
            git.root.join("src/components/CLAUDE.md"),
            "# Component\n# My shadow\n",
        )
        .unwrap();

        // Remove
        remove_overlay_for_test(&git, "src/components/CLAUDE.md");

        let content = std::fs::read_to_string(git.root.join("src/components/CLAUDE.md")).unwrap();
        assert_eq!(content, "# Component\n");
        assert!(!git.shadow_dir.join("baselines").join(&encoded).exists());
    }

    /// Helper to remove overlay (bypasses prompt)
    fn remove_overlay_for_test(git: &GitRepo, file_path: &str) {
        let encoded = path::encode_path(file_path);
        let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
        let worktree_path = git.root.join(file_path);

        if baseline_path.exists() {
            let baseline = std::fs::read(&baseline_path).unwrap();
            std::fs::write(&worktree_path, &baseline).unwrap();
            std::fs::remove_file(&baseline_path).unwrap();
        }
    }

    /// Helper to remove a phantom via the SAME path as `run()`: drop it from the config
    /// and regenerate the shared exclude section from the union of worktrees' configs
    /// (`union_patterns` + `set_entries`). Bypasses only the TTY prompt.
    fn remove_phantom_for_test(git: &GitRepo, config: &mut ShadowConfig, file_path: &str) {
        config.remove(file_path).unwrap();
        super::regenerate_exclude(git, config).unwrap();
    }

    #[test]
    fn test_remove_phantom_directory_removes_exclude_with_trailing_slash() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Create directory phantom and register it, seeding the exclude via the real path.
        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        std::fs::write(git.root.join(".claude/settings.json"), "{}").unwrap();

        config
            .add_phantom(".claude".to_string(), ExcludeMode::GitInfoExclude, true)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();
        super::regenerate_exclude(&git, &config).unwrap();

        let manager = ExcludeManager::new(&git.common_dir);
        assert!(manager
            .list_entries()
            .unwrap()
            .contains(&"/.claude/".to_string()));

        // Remove phantom directory via the real path
        remove_phantom_for_test(&git, &mut config, ".claude");

        // Anchored directory exclude entry should be removed
        let entries = manager.list_entries().unwrap();
        assert!(
            !entries.contains(&"/.claude/".to_string()),
            "Anchored directory exclude entry should be removed, got: {:?}",
            entries
        );

        // Directory should still exist
        assert!(git.root.join(".claude").is_dir());
        assert!(git.root.join(".claude/settings.json").exists());
    }

    #[test]
    fn test_remove_drops_anchored_entry_when_last_worktree_unregisters() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Register a phantom file and seed the exclude section via the real anchored path.
        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::GitInfoExclude, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();
        super::regenerate_exclude(&git, &config).unwrap();

        let manager = ExcludeManager::new(&git.common_dir);
        assert!(manager
            .list_entries()
            .unwrap()
            .contains(&"/local.md".to_string()));

        // This is the only worktree, so unregistering drops the shared anchored entry.
        remove_phantom_for_test(&git, &mut config, "local.md");

        let entries = manager.list_entries().unwrap();
        assert!(
            !entries.contains(&"/local.md".to_string()),
            "anchored entry should be dropped when the last worktree unregisters it, got: {:?}",
            entries
        );
    }

    #[test]
    fn test_remove_keeps_exclude_entry_used_by_another_worktree() {
        use std::process::Command;

        // Main repo with an installed shadow config.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Command::new("git")
            .args(["init"])
            .current_dir(&root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# Team\n").unwrap();
        Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&root)
            .output()
            .unwrap();

        let main_git = GitRepo::discover(&root).unwrap();
        std::fs::create_dir_all(main_git.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(main_git.shadow_dir.join("stash")).unwrap();

        // Both worktrees manage the same phantom `local.md`.
        let mut main_config = ShadowConfig::new();
        main_config
            .add_phantom("local.md".to_string(), ExcludeMode::GitInfoExclude, false)
            .unwrap();
        main_config.save(&main_git.shadow_dir).unwrap();

        // Create a worktree and give it its own config referencing the same phantom.
        let wt_path = dir.path().join("worktree");
        Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "wt-branch",
                wt_path.to_str().unwrap(),
            ])
            .current_dir(&root)
            .output()
            .unwrap();
        let wt_git = GitRepo::discover(&wt_path).unwrap();
        std::fs::create_dir_all(wt_git.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(wt_git.shadow_dir.join("stash")).unwrap();
        let mut wt_config = ShadowConfig::new();
        wt_config
            .add_phantom("local.md".to_string(), ExcludeMode::GitInfoExclude, false)
            .unwrap();
        wt_config.save(&wt_git.shadow_dir).unwrap();

        // Simulate removing `local.md` from the main worktree only.
        main_config.remove("local.md").unwrap();
        let patterns = crate::exclude::union_patterns(&main_git, &main_config);

        // The worktree still references it, so the shared entry must remain.
        assert!(
            patterns.contains(&"/local.md".to_string()),
            "shared exclude entry must be kept, got: {:?}",
            patterns
        );
    }
}
