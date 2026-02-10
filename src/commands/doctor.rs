use anyhow::Result;
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;

const HOOK_NAMES: &[&str] = &["pre-commit", "post-commit", "post-merge"];
const COMPETING_HOOKS: &[&str] = &[".husky", ".pre-commit-config.yaml", "lefthook.yml"];

pub fn run() -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    let mut issues = Vec::new();
    let mut warnings = Vec::new();

    // 1. Check hook files
    check_hooks(&git, &mut issues, &mut warnings);

    // 2. Check competing hook managers
    check_competing_hooks(&git, &mut warnings);

    // 3. Check config integrity
    check_config_integrity(&git, &config, &mut issues);

    // 4. Check stash remnants
    check_stash(&git, &mut warnings);

    // 5. Check lock
    check_lock(&git, &mut warnings);

    // Print results
    if issues.is_empty() && warnings.is_empty() {
        println!("{}", "all checks passed".green());
    } else {
        if !issues.is_empty() {
            println!("{}", "issues:".red());
            for issue in &issues {
                println!("  {} {}", "✗".red(), issue);
            }
        }
        if !warnings.is_empty() {
            println!("{}", "warnings:".yellow());
            for warning in &warnings {
                println!("  {} {}", "⚠".yellow(), warning);
            }
        }
    }

    Ok(())
}

fn check_hooks(git: &GitRepo, issues: &mut Vec<String>, warnings: &mut Vec<String>) {
    for hook_name in HOOK_NAMES {
        let hook_path = git.git_dir.join("hooks").join(hook_name);

        if !hook_path.exists() {
            issues.push(format!("{} hook does not exist", hook_name));
            continue;
        }

        // Check executable permission
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&hook_path) {
                if metadata.permissions().mode() & 0o111 == 0 {
                    issues.push(format!("{} hook is not executable", hook_name));
                }
            }
        }

        // Check content calls git-shadow
        if let Ok(content) = std::fs::read_to_string(&hook_path) {
            if !content.contains("git-shadow hook") && !content.contains("git shadow hook") {
                warnings.push(format!("{} hook does not call git-shadow", hook_name));
            }
        }
    }
}

fn check_competing_hooks(git: &GitRepo, warnings: &mut Vec<String>) {
    for marker in COMPETING_HOOKS {
        if git.root.join(marker).exists() {
            warnings.push(format!("competing hook manager detected: {}", marker));
        }
    }
}

fn check_config_integrity(git: &GitRepo, config: &ShadowConfig, issues: &mut Vec<String>) {
    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                let worktree_path = git.root.join(file_path);
                if !worktree_path.exists() {
                    issues.push(format!("{} does not exist in working tree", file_path));
                }

                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                if !baseline_path.exists() {
                    issues.push(format!("baseline file for {} does not exist", file_path));
                }
            }
            FileType::Phantom => {
                let worktree_path = git.root.join(file_path);
                if entry.is_directory {
                    if !worktree_path.is_dir() {
                        issues.push(format!(
                            "{} (phantom dir) does not exist in working tree",
                            file_path
                        ));
                    }
                } else if !worktree_path.exists() {
                    issues.push(format!(
                        "{} (phantom) does not exist in working tree",
                        file_path
                    ));
                }
            }
        }
    }
}

fn check_stash(git: &GitRepo, warnings: &mut Vec<String>) {
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
            warnings.push("stash has remaining files. Run `git-shadow restore`".to_string());
        }
    }
}

fn check_lock(git: &GitRepo, warnings: &mut Vec<String>) {
    if let Ok(status) = lock::check_lock(&git.shadow_dir) {
        match status {
            LockStatus::Stale(info) => {
                warnings.push(format!(
                    "stale lockfile detected (PID {}). Run `git-shadow restore`",
                    info.pid
                ));
            }
            LockStatus::HeldByOther(info) => {
                warnings.push(format!(
                    "lockfile is held by another process (PID {})",
                    info.pid
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ShadowConfig;
    use crate::fs_util;
    use crate::git::GitRepo;
    use crate::path;

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
    fn test_hook_missing_detected() {
        let (_dir, git) = make_test_repo();
        let mut issues = Vec::new();
        let mut warnings = Vec::new();

        super::check_hooks(&git, &mut issues, &mut warnings);

        // Hooks not installed yet
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.contains("pre-commit")));
    }

    #[test]
    fn test_hook_present_and_valid() {
        let (_dir, git) = make_test_repo();

        // Install hooks
        let hooks_dir = git.git_dir.join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        for name in super::HOOK_NAMES {
            let content = format!("#!/bin/sh\ngit-shadow hook {}\n", name);
            std::fs::write(hooks_dir.join(name), &content).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    hooks_dir.join(name),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }

        let mut issues = Vec::new();
        let mut warnings = Vec::new();
        super::check_hooks(&git, &mut issues, &mut warnings);

        assert!(issues.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_competing_hooks_detected() {
        let (_dir, git) = make_test_repo();

        // Create competing hook marker
        std::fs::write(git.root.join(".pre-commit-config.yaml"), "repos: []\n").unwrap();

        let mut warnings = Vec::new();
        super::check_competing_hooks(&git, &mut warnings);

        assert!(!warnings.is_empty());
        assert!(warnings
            .iter()
            .any(|w| w.contains("competing hook manager")));
    }

    #[test]
    fn test_config_integrity_missing_file() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();

        // Add overlay but delete the file
        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        config.save(&git.shadow_dir).unwrap();

        std::fs::remove_file(git.root.join("CLAUDE.md")).unwrap();

        let mut issues = Vec::new();
        super::check_config_integrity(&git, &config, &mut issues);

        assert!(issues
            .iter()
            .any(|i| i.contains("does not exist in working tree")));
    }

    #[test]
    fn test_config_integrity_missing_baseline() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();

        // Add overlay without creating baseline
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        config.save(&git.shadow_dir).unwrap();

        let mut issues = Vec::new();
        super::check_config_integrity(&git, &config, &mut issues);

        assert!(issues.iter().any(|i| i.contains("baseline file for")));
    }

    #[test]
    fn test_stash_remnant_detected() {
        let (_dir, git) = make_test_repo();

        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "remnant").unwrap();

        let mut warnings = Vec::new();
        super::check_stash(&git, &mut warnings);

        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("stash")));
    }

    #[test]
    fn test_stale_lock_detected() {
        let (_dir, git) = make_test_repo();

        // Create stale lock with non-existent PID
        std::fs::write(
            git.shadow_dir.join("lock"),
            "pid=999999\ntimestamp=2026-01-01T00:00:00+00:00",
        )
        .unwrap();

        let mut warnings = Vec::new();
        super::check_lock(&git, &mut warnings);

        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("stale lockfile")));
    }

    #[test]
    fn test_config_integrity_phantom_dir_missing() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        // Register phantom directory but don't create the directory
        config
            .add_phantom(
                ".claude".to_string(),
                crate::config::ExcludeMode::None,
                true,
            )
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        let mut issues = Vec::new();
        super::check_config_integrity(&git, &config, &mut issues);

        assert!(
            issues.iter().any(|i| i.contains("phantom dir")),
            "Should report missing phantom directory, got: {:?}",
            issues
        );
    }

    #[test]
    fn test_config_integrity_phantom_dir_present() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        std::fs::create_dir_all(git.root.join(".claude")).unwrap();
        config
            .add_phantom(
                ".claude".to_string(),
                crate::config::ExcludeMode::None,
                true,
            )
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        let mut issues = Vec::new();
        super::check_config_integrity(&git, &config, &mut issues);

        assert!(
            issues.is_empty(),
            "Should have no issues when directory exists, got: {:?}",
            issues
        );
    }

    #[test]
    fn test_all_healthy() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        // Install hooks
        let hooks_dir = git.git_dir.join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        for name in super::HOOK_NAMES {
            let content = format!("#!/bin/sh\ngit-shadow hook {}\n", name);
            std::fs::write(hooks_dir.join(name), &content).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    hooks_dir.join(name),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }

        let mut issues = Vec::new();
        let mut warnings = Vec::new();
        super::check_hooks(&git, &mut issues, &mut warnings);
        super::check_competing_hooks(&git, &mut warnings);
        super::check_config_integrity(&git, &config, &mut issues);
        super::check_stash(&git, &mut warnings);
        super::check_lock(&git, &mut warnings);

        assert!(issues.is_empty());
        assert!(warnings.is_empty());
    }
}
