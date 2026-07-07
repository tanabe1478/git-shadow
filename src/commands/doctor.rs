use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;
use crate::ui::{self, UiLocale};

const HOOK_NAMES: &[&str] = &["pre-commit", "post-commit", "post-merge", "post-rewrite"];
const COMPETING_HOOKS: &[&str] = &[".husky", ".pre-commit-config.yaml", "lefthook.yml"];

/// Machine-readable doctor report (stable English keys/values, not localized).
#[derive(Serialize)]
struct DoctorReport<'a> {
    ok: bool,
    issues: &'a [String],
    warnings: &'a [String],
}

/// Run all diagnostic checks and collect issues (broken) and warnings (attention).
fn collect(git: &GitRepo, config: &ShadowConfig, locale: UiLocale) -> (Vec<String>, Vec<String>) {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();

    check_hooks(git, &mut issues, &mut warnings, locale);
    check_competing_hooks(git, &mut warnings, locale);
    check_config_integrity(git, config, &mut issues, locale);
    check_stash(git, &mut warnings, locale);
    check_lock(git, &mut warnings, locale);
    check_suspended(config, git, &mut warnings, locale);
    check_worktree(git, &mut warnings, locale);

    (issues, warnings)
}

pub fn run(json: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    // JSON output must be stable and English regardless of locale.
    let report_locale = if json { UiLocale::En } else { locale };
    let (issues, warnings) = collect(&git, &config, report_locale);
    let issue_count = issues.len();

    if json {
        let report = DoctorReport {
            ok: issues.is_empty(),
            issues: &issues,
            warnings: &warnings,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if issues.is_empty() && warnings.is_empty() {
        println!("{}", ui::doctor_all_checks_passed(locale).green());
    } else {
        if !issues.is_empty() {
            println!("{}", ui::doctor_issues_heading(locale).red());
            for issue in &issues {
                println!("  {} {}", "✗".red(), issue);
            }
        }
        if !warnings.is_empty() {
            println!("{}", ui::doctor_warnings_heading(locale).yellow());
            for warning in &warnings {
                println!("  {} {}", "⚠".yellow(), warning);
            }
        }
    }

    // Issues (red) make doctor exit non-zero so scripts/CI can gate on it.
    // Warnings alone keep a zero exit code. The full report is printed first.
    if issue_count > 0 {
        return Err(ShadowError::DoctorFoundIssues(issue_count).into());
    }

    Ok(())
}

fn check_hooks(
    git: &GitRepo,
    issues: &mut Vec<String>,
    warnings: &mut Vec<String>,
    locale: ui::UiLocale,
) {
    let effective_dir = git.effective_hooks_dir();

    // Detect inert hooks: installed in the default dir, but core.hooksPath points
    // elsewhere so Git never runs them (silent data-leak risk).
    if let Some(hooks_path) = git.hooks_path_config() {
        let default_dir = git.hooks_dir();
        if default_dir != effective_dir {
            let default_installed = HOOK_NAMES
                .iter()
                .all(|name| hook_calls_shadow(&default_dir.join(name)));
            let effective_installed = HOOK_NAMES
                .iter()
                .all(|name| hook_calls_shadow(&effective_dir.join(name)));
            if default_installed && !effective_installed {
                issues.push(ui::doctor_hooks_inert(locale, &hooks_path));
                return;
            }
        }
    }

    for hook_name in HOOK_NAMES {
        let hook_path = effective_dir.join(hook_name);

        if !hook_path.exists() {
            issues.push(ui::doctor_hook_missing(locale, hook_name));
            continue;
        }

        // Check executable permission
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&hook_path) {
                if metadata.permissions().mode() & 0o111 == 0 {
                    issues.push(ui::doctor_hook_not_executable(locale, hook_name));
                }
            }
        }

        // Check content calls git-shadow
        if let Ok(content) = std::fs::read_to_string(&hook_path) {
            if !content.contains("git-shadow hook") && !content.contains("git shadow hook") {
                warnings.push(ui::doctor_hook_not_calling_shadow(locale, hook_name));
            }
        }
    }
}

/// Whether the hook file at `path` exists and dispatches to git-shadow.
fn hook_calls_shadow(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| content.contains("git-shadow hook") || content.contains("git shadow hook"))
        .unwrap_or(false)
}

fn check_competing_hooks(git: &GitRepo, warnings: &mut Vec<String>, locale: ui::UiLocale) {
    for marker in COMPETING_HOOKS {
        if git.root.join(marker).exists() {
            warnings.push(ui::doctor_competing_hook_manager(locale, marker));
        }
    }
}

fn check_config_integrity(
    git: &GitRepo,
    config: &ShadowConfig,
    issues: &mut Vec<String>,
    locale: ui::UiLocale,
) {
    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                let worktree_path = git.root.join(file_path);
                if !worktree_path.exists() {
                    issues.push(ui::doctor_overlay_missing_worktree(locale, file_path));
                }

                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                if !baseline_path.exists() {
                    issues.push(ui::doctor_baseline_missing(locale, file_path));
                }
            }
            FileType::Phantom => {
                let worktree_path = git.root.join(file_path);
                if entry.is_directory {
                    if !worktree_path.is_dir() {
                        issues.push(ui::doctor_phantom_dir_missing(locale, file_path));
                    }
                } else if !worktree_path.exists() {
                    issues.push(ui::doctor_phantom_missing(locale, file_path));
                }
            }
        }
    }
}

fn check_stash(git: &GitRepo, warnings: &mut Vec<String>, locale: ui::UiLocale) {
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
            warnings.push(ui::doctor_stash_remaining(locale).to_string());
        }
    }
}

fn check_suspended(
    config: &ShadowConfig,
    git: &GitRepo,
    warnings: &mut Vec<String>,
    locale: ui::UiLocale,
) {
    if config.suspended {
        warnings.push(ui::doctor_suspended(locale).to_string());

        // Check if suspended directory exists and has files
        let suspended_dir = git.shadow_dir.join("suspended");
        if !suspended_dir.exists() {
            warnings.push(ui::doctor_suspended_dir_missing(locale).to_string());
        }
    }
}

fn check_worktree(git: &GitRepo, warnings: &mut Vec<String>, locale: ui::UiLocale) {
    if git.git_dir != git.common_dir {
        // We are in a worktree
        if !git.shadow_dir.exists() {
            warnings.push(ui::doctor_worktree_not_initialized(locale).to_string());
        } else {
            let config_path = git.shadow_dir.join("config.json");
            if !config_path.exists() {
                warnings.push(ui::doctor_worktree_no_config(locale).to_string());
            }
        }
    }
}

fn check_lock(git: &GitRepo, warnings: &mut Vec<String>, locale: ui::UiLocale) {
    if let Ok(status) = lock::check_lock(&git.shadow_dir) {
        match status {
            LockStatus::Stale(info) => {
                warnings.push(ui::doctor_stale_lock(locale, info.pid));
            }
            LockStatus::HeldByOther(info) => {
                warnings.push(ui::doctor_lock_held(locale, info.pid));
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
    use crate::ui::UiLocale;

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

        super::check_hooks(&git, &mut issues, &mut warnings, UiLocale::En);

        // Hooks not installed yet
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.contains("pre-commit")));
    }

    #[test]
    fn test_hook_present_and_valid() {
        let (_dir, git) = make_test_repo();

        // Install hooks
        let hooks_dir = git.hooks_dir();
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
        super::check_hooks(&git, &mut issues, &mut warnings, UiLocale::En);

        assert!(issues.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_competing_hooks_detected() {
        let (_dir, git) = make_test_repo();

        // Create competing hook marker
        std::fs::write(git.root.join(".pre-commit-config.yaml"), "repos: []\n").unwrap();

        let mut warnings = Vec::new();
        super::check_competing_hooks(&git, &mut warnings, UiLocale::En);

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
        super::check_config_integrity(&git, &config, &mut issues, UiLocale::En);

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
        super::check_config_integrity(&git, &config, &mut issues, UiLocale::En);

        assert!(issues.iter().any(|i| i.contains("baseline file for")));
    }

    #[test]
    fn test_stash_remnant_detected() {
        let (_dir, git) = make_test_repo();

        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "remnant").unwrap();

        let mut warnings = Vec::new();
        super::check_stash(&git, &mut warnings, UiLocale::En);

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
        super::check_lock(&git, &mut warnings, UiLocale::En);

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
        super::check_config_integrity(&git, &config, &mut issues, UiLocale::En);

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
        super::check_config_integrity(&git, &config, &mut issues, UiLocale::En);

        assert!(
            issues.is_empty(),
            "Should have no issues when directory exists, got: {:?}",
            issues
        );
    }

    fn install_default_hooks(git: &GitRepo) {
        let hooks_dir = git.hooks_dir();
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
    }

    #[test]
    fn test_inert_hooks_detected_when_hooks_path_set() {
        let (_dir, git) = make_test_repo();
        // Hooks installed in the default dir...
        install_default_hooks(&git);
        // ...but core.hooksPath points elsewhere, so they never run.
        std::process::Command::new("git")
            .args(["config", "core.hooksPath", "dev-hooks"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let mut issues = Vec::new();
        let mut warnings = Vec::new();
        super::check_hooks(&git, &mut issues, &mut warnings, UiLocale::En);

        assert!(
            issues.iter().any(|i| i.contains("core.hooksPath")),
            "should report inert hooks, got: {:?}",
            issues
        );
    }

    #[test]
    fn test_collect_reports_issues_when_hooks_missing() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        let (issues, _warnings) = super::collect(&git, &config, UiLocale::En);
        assert!(!issues.is_empty(), "missing hooks should surface as issues");
    }

    #[test]
    fn test_collect_no_issues_when_healthy() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();
        install_default_hooks(&git);

        let (issues, _warnings) = super::collect(&git, &config, UiLocale::En);
        assert!(issues.is_empty(), "healthy repo should have no issues");
    }

    #[test]
    fn test_all_healthy() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        config.save(&git.shadow_dir).unwrap();

        // Install hooks
        let hooks_dir = git.hooks_dir();
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
        super::check_hooks(&git, &mut issues, &mut warnings, UiLocale::En);
        super::check_competing_hooks(&git, &mut warnings, UiLocale::En);
        super::check_config_integrity(&git, &config, &mut issues, UiLocale::En);
        super::check_stash(&git, &mut warnings, UiLocale::En);
        super::check_lock(&git, &mut warnings, UiLocale::En);

        assert!(issues.is_empty());
        assert!(warnings.is_empty());
    }
}
