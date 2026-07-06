use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};

use crate::config::{FileType, ShadowConfig};
use crate::git::GitRepo;
use crate::ui;
use crate::{fs_util, path};

const HOOK_NAMES: &[&str] = &["pre-commit", "post-commit", "post-merge", "post-rewrite"];

fn generate_hook_script(hook_name: &str) -> String {
    format!(
        r#"#!/bin/sh
# git-shadow managed hook
HOOKS_DIR="$(cd "$(dirname "$0")" && pwd)"

git-shadow hook {hook_name}
SHADOW_EXIT=$?
if [ $SHADOW_EXIT -ne 0 ]; then
  exit $SHADOW_EXIT
fi

# Chain to existing hook
if [ -x "$HOOKS_DIR/{hook_name}.pre-shadow" ]; then
  "$HOOKS_DIR/{hook_name}.pre-shadow" "$@"
fi
"#,
        hook_name = hook_name
    )
}

pub fn run() -> Result<()> {
    let locale = ui::detect_locale();
    let git = GitRepo::discover(&std::env::current_dir()?)?;

    // Create shadow directory structure
    let shadow_dir = &git.shadow_dir;
    std::fs::create_dir_all(shadow_dir.join("baselines"))
        .context("failed to create .git/shadow/baselines/")?;
    std::fs::create_dir_all(shadow_dir.join("stash"))
        .context("failed to create .git/shadow/stash/")?;

    // In a worktree, inherit config from main repo if available
    inherit_from_main_worktree(&git)?;

    // Honor core.hooksPath: if set (husky, lefthook, dev-hooks, ...), hooks in the
    // default common_dir/hooks would never run, so install into the effective directory.
    let hooks_dir = git.effective_hooks_dir();
    let custom_hooks_path = git.hooks_path_config();
    std::fs::create_dir_all(&hooks_dir).context("failed to create hooks directory")?;

    for hook_name in HOOK_NAMES {
        let hook_path = hooks_dir.join(hook_name);

        // Check if already installed by us
        if hook_path.exists() {
            let content = std::fs::read_to_string(&hook_path)?;
            if content.contains("git-shadow hook") {
                // Already installed, skip
                continue;
            }
            // Existing hook from another tool - back it up
            let backup = hooks_dir.join(format!("{}.pre-shadow", hook_name));
            std::fs::rename(&hook_path, &backup)
                .with_context(|| format!("failed to back up {}", hook_name))?;
        }

        let script = generate_hook_script(hook_name);
        std::fs::write(&hook_path, &script)
            .with_context(|| format!("failed to write {}", hook_name))?;

        // Set executable permission
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    if let Some(hooks_path) = custom_hooks_path {
        println!(
            "{}",
            ui::install_custom_hooks_path(locale, &hooks_path, &hooks_dir.display().to_string())
        );
    }

    println!("{}", ui::install_success(locale));
    Ok(())
}

/// In a worktree, check if the main repo has a shadow config and inherit it.
/// Overlays get their baselines regenerated from the worktree's HEAD.
/// Phantoms are copied as-is (exclude is already shared via common_dir).
pub fn inherit_from_main_worktree(git: &GitRepo) -> Result<()> {
    // Only run in worktree environments
    if git.git_dir == git.common_dir {
        return Ok(());
    }

    // Check if this worktree already has a config
    let local_config_path = git.shadow_dir.join("config.json");
    if local_config_path.exists() {
        return Ok(());
    }

    // Look for config in main repo's shadow dir
    let main_shadow_dir = git.common_dir.join("shadow");
    let main_config_path = main_shadow_dir.join("config.json");
    if !main_config_path.exists() {
        return Ok(());
    }

    let main_config = ShadowConfig::load(&main_shadow_dir)?;
    if main_config.files.is_empty() {
        return Ok(());
    }

    let mut local_config = ShadowConfig::new();
    let mut inherited_count = 0;

    for (file_path, entry) in &main_config.files {
        match entry.file_type {
            FileType::Overlay => {
                // Check if the file exists in this worktree's HEAD
                if !git.is_tracked(file_path)? {
                    continue;
                }

                // Regenerate baseline from this worktree's HEAD
                let commit = git.head_commit()?;
                let baseline_content = match git.show_file("HEAD", file_path) {
                    Ok(content) => content,
                    Err(_) => continue,
                };

                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                fs_util::atomic_write(&baseline_path, &baseline_content)
                    .with_context(|| format!("failed to save baseline for {}", file_path))?;

                local_config.add_overlay(file_path.clone(), commit)?;
                inherited_count += 1;
            }
            FileType::Phantom => {
                // Copy phantom entry as-is (exclude is shared via common_dir)
                local_config.add_phantom(
                    file_path.clone(),
                    entry.exclude_mode.clone(),
                    entry.is_directory,
                )?;
                inherited_count += 1;
            }
        }
    }

    if inherited_count > 0 {
        local_config.save(&git.shadow_dir)?;
        println!(
            "{}",
            ui::inherited_from_main_worktree(ui::detect_locale(), inherited_count)
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
        let repo = GitRepo::discover(&root).unwrap();
        (dir, repo)
    }

    fn install_hooks(git: &GitRepo) {
        let shadow_dir = &git.shadow_dir;
        std::fs::create_dir_all(shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(shadow_dir.join("stash")).unwrap();

        let hooks_dir = git.effective_hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();

        for hook_name in HOOK_NAMES {
            let hook_path = hooks_dir.join(hook_name);
            if hook_path.exists() {
                let content = std::fs::read_to_string(&hook_path).unwrap();
                if content.contains("git-shadow hook") {
                    continue;
                }
                let backup = hooks_dir.join(format!("{}.pre-shadow", hook_name));
                std::fs::rename(&hook_path, &backup).unwrap();
            }
            let script = generate_hook_script(hook_name);
            std::fs::write(&hook_path, &script).unwrap();
            let mut perms = std::fs::metadata(&hook_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook_path, perms).unwrap();
        }
    }

    #[test]
    fn test_creates_hook_files() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);

        for name in HOOK_NAMES {
            let hook = git.hooks_dir().join(name);
            assert!(hook.exists(), "{} should exist", name);
        }
    }

    #[test]
    fn test_hook_content_calls_git_shadow() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);

        for name in HOOK_NAMES {
            let hook = git.hooks_dir().join(name);
            let content = std::fs::read_to_string(&hook).unwrap();
            assert!(
                content.contains(&format!("git-shadow hook {}", name)),
                "{} should call git-shadow hook",
                name
            );
        }
    }

    #[test]
    fn test_hook_has_executable_permission() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);

        for name in HOOK_NAMES {
            let hook = git.hooks_dir().join(name);
            let perms = std::fs::metadata(&hook).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0, "{} should be executable", name);
        }
    }

    #[test]
    fn test_preserves_existing_hooks() {
        let (_dir, git) = make_test_repo();
        let hooks_dir = git.hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();

        // Create an existing pre-commit hook
        let existing = hooks_dir.join("pre-commit");
        std::fs::write(&existing, "#!/bin/sh\necho existing\n").unwrap();

        install_hooks(&git);

        // Original should be backed up
        let backup = hooks_dir.join("pre-commit.pre-shadow");
        assert!(backup.exists());
        let backup_content = std::fs::read_to_string(&backup).unwrap();
        assert!(backup_content.contains("echo existing"));

        // New hook should call git-shadow
        let new_content = std::fs::read_to_string(&existing).unwrap();
        assert!(new_content.contains("git-shadow hook pre-commit"));
        assert!(new_content.contains("pre-commit.pre-shadow"));
    }

    #[test]
    fn test_install_respects_hooks_path() {
        let (_dir, git) = make_test_repo();

        // Simulate husky/lefthook/dev-hooks: core.hooksPath points elsewhere.
        std::process::Command::new("git")
            .args(["config", "core.hooksPath", "dev-hooks"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        install_hooks(&git);

        // Hooks must land in the custom directory, not common_dir/hooks.
        let custom_dir = git.root.join("dev-hooks");
        for name in HOOK_NAMES {
            let hook = custom_dir.join(name);
            assert!(hook.exists(), "{} should exist in custom hooks dir", name);
            let content = std::fs::read_to_string(&hook).unwrap();
            assert!(content.contains(&format!("git-shadow hook {}", name)));
        }

        // Nothing should have been written to the default hooks dir.
        for name in HOOK_NAMES {
            assert!(
                !git.hooks_dir().join(name).exists(),
                "{} should NOT exist in default hooks dir",
                name
            );
        }

        assert!(git.hooks_installed());
    }

    #[test]
    fn test_creates_shadow_directories() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);

        assert!(git.shadow_dir.join("baselines").exists());
        assert!(git.shadow_dir.join("stash").exists());
    }

    #[test]
    fn test_idempotent_install() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);
        install_hooks(&git); // Second install should not error

        for name in HOOK_NAMES {
            let hook = git.hooks_dir().join(name);
            let content = std::fs::read_to_string(&hook).unwrap();
            // Should not be double-wrapped
            let count = content.matches("git-shadow hook").count();
            assert_eq!(count, 1, "{} should only have one git-shadow call", name);
        }
    }

    #[test]
    fn test_hooks_installed_returns_true_after_install() {
        let (_dir, git) = make_test_repo();
        assert!(!git.hooks_installed());
        install_hooks(&git);
        assert!(git.hooks_installed());
    }

    fn install_for_test(git: &GitRepo) {
        let shadow_dir = &git.shadow_dir;
        std::fs::create_dir_all(shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(shadow_dir.join("stash")).unwrap();

        super::inherit_from_main_worktree(git).unwrap();

        let hooks_dir = git.effective_hooks_dir();
        std::fs::create_dir_all(&hooks_dir).unwrap();

        for hook_name in HOOK_NAMES {
            let hook_path = hooks_dir.join(hook_name);
            if hook_path.exists() {
                let content = std::fs::read_to_string(&hook_path).unwrap();
                if content.contains("git-shadow hook") {
                    continue;
                }
                let backup = hooks_dir.join(format!("{}.pre-shadow", hook_name));
                std::fs::rename(&hook_path, &backup).unwrap();
            }
            let script = generate_hook_script(hook_name);
            std::fs::write(&hook_path, &script).unwrap();
            let mut perms = std::fs::metadata(&hook_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook_path, perms).unwrap();
        }
    }

    #[test]
    fn test_worktree_inherits_config_from_main() {
        use crate::config::{FileType, ShadowConfig};
        use crate::path;
        use std::process::Command;

        // Set up main repo with a tracked file and commit
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

        // Install and configure shadow in main repo
        install_hooks(&main_git);
        let commit = main_git.head_commit().unwrap();
        let baseline_content = main_git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        crate::fs_util::atomic_write(
            &main_git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        let mut config = ShadowConfig::new();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        config.save(&main_git.shadow_dir).unwrap();

        // Create worktree
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

        // Install in worktree — should inherit config
        install_for_test(&wt_git);

        // Verify: config was inherited
        let wt_config = ShadowConfig::load(&wt_git.shadow_dir).unwrap();
        assert_eq!(wt_config.files.len(), 1, "should inherit 1 file");
        let entry = wt_config.get("CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, FileType::Overlay);

        // Verify: baseline was regenerated from worktree's HEAD
        let baseline_path = wt_git.shadow_dir.join("baselines").join(&encoded);
        assert!(baseline_path.exists(), "baseline should be created");
        let baseline = std::fs::read(&baseline_path).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&baseline),
            "# Team\n",
            "baseline should match HEAD content"
        );
    }
}
