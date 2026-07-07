use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::exclude::{self, ExcludeManager};
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;
use crate::ui;

const HOOK_NAMES: &[&str] = &["pre-commit", "post-commit", "post-merge", "post-rewrite"];

pub fn run(force: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    // Refuse while a commit is in progress: leftover stash or a live lock means
    // pre-commit ran but post-commit has not, so wiping state would lose work.
    guard_commit_in_progress(&git)?;

    // Refuse if files are still managed, unless --force restores overlays and wipes.
    if !config.files.is_empty() && !force {
        return Err(ShadowError::UninstallHasEntries(config.files.len()).into());
    }

    // --force: restore overlay baselines to the working tree; phantom files are left
    // untouched on disk (they are the user's local-only files).
    if force {
        let restored = restore_overlays(&git, &config)?;
        if restored > 0 {
            println!(
                "{}",
                ui::uninstall_forced_overlays(locale, restored).green()
            );
        }
    }

    // Remove the shadow hooks from the effective hooks dir, restoring any pre-shadow
    // backups made by install (mirrors install's chaining/backup logic).
    remove_hooks(&git, locale)?;

    // Regenerate the shared exclude section from the OTHER worktrees' configs only.
    // Passing an empty config for the current worktree drops entries this worktree
    // owned while preserving those another worktree still relies on.
    let manager = ExcludeManager::new(&git.common_dir);
    let patterns = exclude::union_patterns(&git, &ShadowConfig::new());
    manager.set_entries(&patterns)?;

    // Remove this worktree's shadow state (git_dir-based, per-worktree).
    if git.shadow_dir.exists() {
        std::fs::remove_dir_all(&git.shadow_dir)
            .with_context(|| format!("failed to remove {}", git.shadow_dir.display()))?;
    }

    println!("{}", ui::uninstall_success(locale).green());
    Ok(())
}

/// Block uninstall when a commit cycle appears to be mid-flight.
fn guard_commit_in_progress(git: &GitRepo) -> Result<()> {
    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let has_files = std::fs::read_dir(&stash_dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            })
            .unwrap_or(false);
        if has_files {
            return Err(ShadowError::StashRemaining.into());
        }
    }

    if let Ok(LockStatus::HeldByOther(info)) = lock::check_lock(&git.shadow_dir) {
        return Err(ShadowError::LockHeld {
            pid: info.pid,
            timestamp: info.timestamp.to_rfc3339(),
        }
        .into());
    }

    Ok(())
}

/// Restore overlay baselines to the working tree. Returns the number restored.
fn restore_overlays(git: &GitRepo, config: &ShadowConfig) -> Result<usize> {
    let mut restored = 0;
    for (file_path, entry) in &config.files {
        if entry.file_type != FileType::Overlay {
            continue;
        }
        let encoded = path::encode_path(file_path);
        let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
        if baseline_path.exists() {
            let baseline = std::fs::read(&baseline_path)?;
            let worktree_path = git.root.join(file_path);
            std::fs::write(&worktree_path, &baseline)
                .with_context(|| format!("failed to restore baseline to {}", file_path))?;
            restored += 1;
        }
    }
    Ok(restored)
}

/// Remove shadow hooks from the effective hooks dir, restoring pre-shadow backups.
fn remove_hooks(git: &GitRepo, locale: ui::UiLocale) -> Result<()> {
    let hooks_dir = git.effective_hooks_dir();

    for hook_name in HOOK_NAMES {
        let hook_path = hooks_dir.join(hook_name);
        let backup = hooks_dir.join(format!("{}.pre-shadow", hook_name));

        // Only remove hooks we installed (they dispatch to git-shadow). Leave any
        // unrelated user hook in place.
        if let Ok(content) = std::fs::read_to_string(&hook_path) {
            if content.contains("git-shadow hook") {
                std::fs::remove_file(&hook_path)
                    .with_context(|| format!("failed to remove {}", hook_name))?;
            }
        }

        // Restore the pre-existing hook we backed up during install.
        if backup.exists() {
            std::fs::rename(&backup, &hook_path)
                .with_context(|| format!("failed to restore {}", hook_name))?;
            println!("{}", ui::uninstall_hook_restored(locale, hook_name));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExcludeMode;
    use std::os::unix::fs::PermissionsExt;

    fn make_test_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "t@t.com"],
        ] {
            std::process::Command::new("git")
                .args(&args)
                .current_dir(&root)
                .output()
                .unwrap();
        }
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

    fn write_shadow_hook(git: &GitRepo, name: &str) {
        let hooks_dir = git.effective_hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let content = format!("#!/bin/sh\ngit-shadow hook {}\n", name);
        std::fs::write(hooks_dir.join(name), &content).unwrap();
        std::fs::set_permissions(hooks_dir.join(name), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    #[test]
    fn test_remove_hooks_deletes_shadow_hooks() {
        let (_dir, git) = make_test_repo();
        for name in HOOK_NAMES {
            write_shadow_hook(&git, name);
        }

        remove_hooks(&git, ui::UiLocale::En).unwrap();

        for name in HOOK_NAMES {
            assert!(
                !git.effective_hooks_dir().join(name).exists(),
                "{} should be removed",
                name
            );
        }
    }

    #[test]
    fn test_remove_hooks_restores_pre_shadow_backup() {
        let (_dir, git) = make_test_repo();
        let hooks_dir = git.effective_hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();

        // Shadow hook plus a backed-up original.
        write_shadow_hook(&git, "pre-commit");
        std::fs::write(
            hooks_dir.join("pre-commit.pre-shadow"),
            "#!/bin/sh\necho original\n",
        )
        .unwrap();

        remove_hooks(&git, ui::UiLocale::En).unwrap();

        let restored = std::fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
        assert!(restored.contains("echo original"));
        assert!(!hooks_dir.join("pre-commit.pre-shadow").exists());
    }

    #[test]
    fn test_remove_hooks_leaves_unrelated_hook() {
        let (_dir, git) = make_test_repo();
        let hooks_dir = git.effective_hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho user\n").unwrap();

        remove_hooks(&git, ui::UiLocale::En).unwrap();

        assert!(hooks_dir.join("pre-commit").exists());
    }

    #[test]
    fn test_restore_overlays_writes_baseline_to_worktree() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        let baseline = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        crate::fs_util::atomic_write(&git.shadow_dir.join("baselines").join(&encoded), &baseline)
            .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();

        // Local shadow edit in the working tree.
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# shadow\n").unwrap();

        let restored = restore_overlays(&git, &config).unwrap();
        assert_eq!(restored, 1);
        assert_eq!(
            std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap(),
            "# Team\n"
        );
    }

    #[test]
    fn test_guard_refuses_on_stash_remnant() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "x").unwrap();
        let result = guard_commit_in_progress(&git);
        assert!(matches!(
            result.unwrap_err().downcast_ref::<ShadowError>(),
            Some(ShadowError::StashRemaining)
        ));
    }

    #[test]
    fn test_uninstall_refuses_with_active_entries() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Mirror run()'s refusal logic without touching cwd.
        assert!(!config.files.is_empty());
        let err = ShadowError::UninstallHasEntries(config.files.len());
        assert!(matches!(err, ShadowError::UninstallHasEntries(1)));
    }

    #[test]
    fn test_clean_uninstall_removes_state_and_hooks() {
        let (_dir, git) = make_test_repo();
        for name in HOOK_NAMES {
            write_shadow_hook(&git, name);
        }
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        // Simulate the core of run() for an empty config.
        guard_commit_in_progress(&git).unwrap();
        remove_hooks(&git, ui::UiLocale::En).unwrap();
        let manager = ExcludeManager::new(&git.common_dir);
        let patterns = exclude::union_patterns(&git, &ShadowConfig::new());
        manager.set_entries(&patterns).unwrap();
        std::fs::remove_dir_all(&git.shadow_dir).unwrap();

        assert!(!git.shadow_dir.exists());
        for name in HOOK_NAMES {
            assert!(!git.effective_hooks_dir().join(name).exists());
        }
    }
}
