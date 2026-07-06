use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::config::{ExcludeMode, FileType, ShadowConfig};
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;
use crate::ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayGitState {
    Clean,
    Modified,
    Staged,
    PartiallyStaged,
}

/// Machine-readable status report (stable English keys/values, not localized).
#[derive(Serialize)]
struct StatusReport {
    suspended: bool,
    /// Stable warning tokens, e.g. "stash_remaining", "stale_lock".
    warnings: Vec<String>,
    files: Vec<FileReport>,
}

#[derive(Serialize)]
struct FileReport {
    path: String,
    #[serde(rename = "type")]
    file_type: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_directory: bool,
    /// Whether the target exists in the working tree.
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shadow_added: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shadow_removed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_outdated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_count: Option<usize>,
}

fn git_state_json(state: OverlayGitState) -> &'static str {
    match state {
        OverlayGitState::Clean => "clean",
        OverlayGitState::Modified => "modified",
        OverlayGitState::Staged => "staged",
        OverlayGitState::PartiallyStaged => "partially_staged",
    }
}

fn exclude_mode_json(mode: &ExcludeMode) -> &'static str {
    match mode {
        ExcludeMode::GitInfoExclude => "git_info_exclude",
        ExcludeMode::None => "none",
    }
}

/// Build the machine-readable report from config + working-tree state.
fn build_report(git: &GitRepo, config: &ShadowConfig) -> Result<StatusReport> {
    let mut warnings = Vec::new();

    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let has_files = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false));
        if has_files {
            warnings.push("stash_remaining".to_string());
        }
    }

    if let LockStatus::Stale(_) = lock::check_lock(&git.shadow_dir)? {
        warnings.push("stale_lock".to_string());
    }

    let mut files = Vec::new();
    for (file_path, entry) in &config.files {
        let worktree_path = git.root.join(file_path);
        match entry.file_type {
            FileType::Overlay => {
                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                let exists = worktree_path.exists();

                let mut shadow_added = None;
                let mut shadow_removed = None;
                let mut git_state = None;
                let mut baseline_outdated = None;

                if exists && baseline_path.exists() {
                    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
                    let current = std::fs::read_to_string(&worktree_path).unwrap_or_default();
                    let (added, removed) = diff_stats(&baseline, &current);
                    shadow_added = Some(added);
                    shadow_removed = Some(removed);

                    let state = overlay_git_state(git.staging_status(file_path)?);
                    git_state = Some(git_state_json(state).to_string());

                    if let Some(ref commit) = entry.baseline_commit {
                        if let Ok(head) = git.head_commit() {
                            let outdated = *commit != head
                                && git
                                    .show_file("HEAD", file_path)
                                    .ok()
                                    .map(|head_content| {
                                        std::fs::read(&baseline_path).unwrap_or_default()
                                            != head_content
                                    })
                                    .unwrap_or(false);
                            baseline_outdated = Some(outdated);
                        }
                    }
                }

                files.push(FileReport {
                    path: file_path.clone(),
                    file_type: "overlay".to_string(),
                    is_directory: false,
                    exists,
                    baseline_commit: entry.baseline_commit.clone(),
                    shadow_added,
                    shadow_removed,
                    git_state,
                    baseline_outdated,
                    exclude_mode: None,
                    size_bytes: None,
                    entry_count: None,
                });
            }
            FileType::Phantom => {
                let (exists, size_bytes, entry_count) = if entry.is_directory {
                    if worktree_path.is_dir() {
                        let count = std::fs::read_dir(&worktree_path)
                            .map(|entries| entries.count())
                            .unwrap_or(0);
                        (true, None, Some(count))
                    } else {
                        (false, None, None)
                    }
                } else if worktree_path.exists() {
                    let size = std::fs::metadata(&worktree_path).map(|m| m.len()).ok();
                    (true, size, None)
                } else {
                    (false, None, None)
                };

                files.push(FileReport {
                    path: file_path.clone(),
                    file_type: "phantom".to_string(),
                    is_directory: entry.is_directory,
                    exists,
                    baseline_commit: None,
                    shadow_added: None,
                    shadow_removed: None,
                    git_state: None,
                    baseline_outdated: None,
                    exclude_mode: Some(exclude_mode_json(&entry.exclude_mode).to_string()),
                    size_bytes,
                    entry_count,
                });
            }
        }
    }

    Ok(StatusReport {
        suspended: config.suspended,
        warnings,
        files,
    })
}

pub fn run(show_git_status: bool, json: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    if json {
        let report = build_report(&git, &config)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if show_git_status {
        print!("{}", git.status_short()?);
        println!();
    }

    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let stash_files: Vec<_> = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        if !stash_files.is_empty() {
            println!("{}", ui::status_warning_stash_remaining(locale).yellow());
            println!("{}", ui::status_action_run_restore(locale).yellow());
            println!();
        }
    }

    if let LockStatus::Stale(info) = lock::check_lock(&git.shadow_dir)? {
        println!(
            "{}",
            ui::status_warning_stale_lock(locale, info.pid).yellow()
        );
        println!("{}", ui::status_action_run_restore(locale).yellow());
        println!();
    }

    if config.files.is_empty() {
        println!("{}", ui::no_managed_files(locale));
        return Ok(());
    }

    if config.suspended {
        println!("{}", ui::status_suspended(locale).yellow());
        println!();
    }

    println!("{}", ui::status_heading_managed_files(locale));
    println!();

    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                println!("  {} ({})", file_path, ui::label_overlay(locale));
                println!("{}", ui::status_overlay_local_only(locale));
                if let Some(ref commit) = entry.baseline_commit {
                    println!(
                        "{}",
                        ui::status_baseline(locale, &commit[..7.min(commit.len())])
                    );
                }

                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                let worktree_path = git.root.join(file_path);

                if !worktree_path.exists() {
                    println!(
                        "{}",
                        ui::status_warning_file_missing_worktree(locale).yellow()
                    );
                } else if baseline_path.exists() {
                    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
                    let current = std::fs::read_to_string(&worktree_path).unwrap_or_default();
                    let (added, removed) = diff_stats(&baseline, &current);
                    println!("{}", ui::status_shadow_changes(locale, added, removed));

                    let git_state = overlay_git_state(git.staging_status(file_path)?);
                    println!(
                        "{}",
                        ui::status_overlay_git_state(locale, git_state_label(git_state))
                    );
                    if matches!(
                        git_state,
                        OverlayGitState::Staged | OverlayGitState::PartiallyStaged
                    ) {
                        println!("{}", ui::status_overlay_staged_warning(locale).yellow());
                    }

                    if let Some(ref commit) = entry.baseline_commit {
                        if let Ok(head) = git.head_commit() {
                            if *commit != head {
                                let content_changed = git
                                    .show_file("HEAD", file_path)
                                    .ok()
                                    .map(|head_content| {
                                        let baseline_bytes =
                                            std::fs::read(&baseline_path).unwrap_or_default();
                                        baseline_bytes != head_content
                                    })
                                    .unwrap_or(false);

                                if content_changed {
                                    println!(
                                        "{}",
                                        ui::status_warning_baseline_outdated(
                                            locale,
                                            &commit[..7.min(commit.len())],
                                            &head[..7.min(head.len())]
                                        )
                                        .yellow()
                                    );
                                    println!(
                                        "{}",
                                        ui::status_action_run_rebase(locale, file_path).yellow()
                                    );
                                }
                            }
                        }
                    }
                }
                println!();
            }
            FileType::Phantom => {
                let label = if entry.is_directory {
                    ui::label_phantom_dir(locale)
                } else {
                    ui::label_phantom(locale)
                };
                println!("  {} ({})", file_path, label);
                if entry.is_directory {
                    println!("{}", ui::status_phantom_dir_explainer(locale));
                }
                match entry.exclude_mode {
                    crate::config::ExcludeMode::GitInfoExclude => {
                        println!("{}", ui::status_exclude_git_info(locale));
                    }
                    crate::config::ExcludeMode::None => {
                        println!("{}", ui::status_exclude_none(locale));
                    }
                }
                let worktree_path = git.root.join(file_path);
                if entry.is_directory {
                    if worktree_path.is_dir() {
                        let count = std::fs::read_dir(&worktree_path)
                            .map(|entries| entries.count())
                            .unwrap_or(0);
                        println!("{}", ui::status_contents(locale, count));
                    } else {
                        println!("{}", ui::status_warning_directory_missing(locale).yellow());
                    }
                } else if worktree_path.exists() {
                    let metadata = std::fs::metadata(&worktree_path)?;
                    println!(
                        "{}",
                        ui::status_file_size(locale, &format_size(metadata.len()))
                    );
                } else {
                    println!("{}", ui::status_warning_file_missing(locale).yellow());
                }
                println!();
            }
        }
    }

    if show_git_status {
        println!("{}", ui::status_git_wrapper_hint(locale));
    }

    Ok(())
}

fn overlay_git_state(status: (bool, bool)) -> OverlayGitState {
    match status {
        (true, true) => OverlayGitState::PartiallyStaged,
        (true, false) => OverlayGitState::Staged,
        (false, true) => OverlayGitState::Modified,
        (false, false) => OverlayGitState::Clean,
    }
}

fn git_state_label(state: OverlayGitState) -> &'static str {
    match state {
        OverlayGitState::Clean => "clean",
        OverlayGitState::Modified => "modified",
        OverlayGitState::Staged => "staged",
        OverlayGitState::PartiallyStaged => "partially staged",
    }
}

fn diff_stats(old: &str, new: &str) -> (usize, usize) {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut added = 0;
    let mut removed = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            _ => {}
        }
    }

    (added, removed)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_git_state_clean() {
        assert_eq!(overlay_git_state((false, false)), OverlayGitState::Clean);
    }

    #[test]
    fn test_overlay_git_state_modified() {
        assert_eq!(overlay_git_state((false, true)), OverlayGitState::Modified);
    }

    #[test]
    fn test_overlay_git_state_staged() {
        assert_eq!(overlay_git_state((true, false)), OverlayGitState::Staged);
    }

    #[test]
    fn test_overlay_git_state_partially_staged() {
        assert_eq!(
            overlay_git_state((true, true)),
            OverlayGitState::PartiallyStaged
        );
    }

    #[test]
    fn test_diff_stats_no_change() {
        let (added, removed) = diff_stats("hello\n", "hello\n");
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_diff_stats_added_lines() {
        let (added, removed) = diff_stats("line1\n", "line1\nline2\nline3\n");
        assert_eq!(added, 2);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_diff_stats_removed_lines() {
        let (added, removed) = diff_stats("line1\nline2\n", "line1\n");
        assert_eq!(added, 0);
        assert_eq!(removed, 1);
    }

    #[test]
    fn test_diff_stats_mixed() {
        let (added, removed) = diff_stats("old\n", "new\n");
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(500), "500 B");
    }

    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1_572_864), "1.5 MB");
    }

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

    #[test]
    fn test_build_report_overlay_json_keys() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        let baseline = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = crate::path::encode_path("CLAUDE.md");
        crate::fs_util::atomic_write(&git.shadow_dir.join("baselines").join(&encoded), &baseline)
            .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# shadow\n").unwrap();

        let report = build_report(&git, &config).unwrap();
        let value = serde_json::to_value(&report).unwrap();

        assert_eq!(value["suspended"], false);
        assert!(value["warnings"].as_array().unwrap().is_empty());
        let file = &value["files"][0];
        assert_eq!(file["path"], "CLAUDE.md");
        assert_eq!(file["type"], "overlay");
        assert_eq!(file["exists"], true);
        assert_eq!(file["shadow_added"], 1);
        assert_eq!(file["git_state"], "modified");
    }

    #[test]
    fn test_build_report_phantom_json_keys() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        std::fs::write(git.root.join("local.md"), "hello").unwrap();
        config
            .add_phantom(
                "local.md".to_string(),
                crate::config::ExcludeMode::GitInfoExclude,
                false,
            )
            .unwrap();

        let report = build_report(&git, &config).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        let file = &value["files"][0];
        assert_eq!(file["type"], "phantom");
        assert_eq!(file["exists"], true);
        assert_eq!(file["exclude_mode"], "git_info_exclude");
        assert_eq!(file["size_bytes"], 5);
    }

    #[test]
    fn test_build_report_warns_on_stash_remnant() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "x").unwrap();

        let report = build_report(&git, &config).unwrap();
        assert!(report.warnings.contains(&"stash_remaining".to_string()));
    }
}
