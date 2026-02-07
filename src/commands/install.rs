use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};

use crate::git::GitRepo;

const HOOK_NAMES: &[&str] = &["pre-commit", "post-commit", "post-merge"];

fn generate_hook_script(hook_name: &str) -> String {
    format!(
        r#"#!/bin/sh
# git-shadow managed hook
git-shadow hook {hook_name}
SHADOW_EXIT=$?
if [ $SHADOW_EXIT -ne 0 ]; then
  exit $SHADOW_EXIT
fi

# 既存 hook のチェーン実行
if [ -x .git/hooks/{hook_name}.pre-shadow ]; then
  .git/hooks/{hook_name}.pre-shadow "$@"
fi
"#,
        hook_name = hook_name
    )
}

pub fn run() -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;

    // Create shadow directory structure
    let shadow_dir = &git.shadow_dir;
    std::fs::create_dir_all(shadow_dir.join("baselines"))
        .context(".git/shadow/baselines/ の作成に失敗")?;
    std::fs::create_dir_all(shadow_dir.join("stash")).context(".git/shadow/stash/ の作成に失敗")?;

    let hooks_dir = git.git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir).context("hooks ディレクトリの作成に失敗")?;

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
                .with_context(|| format!("{} のバックアップに失敗", hook_name))?;
        }

        let script = generate_hook_script(hook_name);
        std::fs::write(&hook_path, &script)
            .with_context(|| format!("{} の書き込みに失敗", hook_name))?;

        // Set executable permission
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    println!("git-shadow hooks をインストールしました");
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

        let hooks_dir = git.git_dir.join("hooks");
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
            let hook = git.git_dir.join("hooks").join(name);
            assert!(hook.exists(), "{} should exist", name);
        }
    }

    #[test]
    fn test_hook_content_calls_git_shadow() {
        let (_dir, git) = make_test_repo();
        install_hooks(&git);

        for name in HOOK_NAMES {
            let hook = git.git_dir.join("hooks").join(name);
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
            let hook = git.git_dir.join("hooks").join(name);
            let perms = std::fs::metadata(&hook).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0, "{} should be executable", name);
        }
    }

    #[test]
    fn test_preserves_existing_hooks() {
        let (_dir, git) = make_test_repo();
        let hooks_dir = git.git_dir.join("hooks");
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
            let hook = git.git_dir.join("hooks").join(name);
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
}
