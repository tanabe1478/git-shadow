use anyhow::Result;
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;

pub fn run() -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    // Check for stash remnants
    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let stash_files: Vec<_> = std::fs::read_dir(&stash_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        if !stash_files.is_empty() {
            println!(
                "{}",
                "  warning: stash has remaining files (a previous commit may have been interrupted)"
                    .yellow()
            );
            println!("{}", "    -> Run `git-shadow restore`".yellow());
            println!();
        }
    }

    // Check for stale lock
    if let LockStatus::Stale(info) = lock::check_lock(&git.shadow_dir)? {
        println!(
            "{}",
            format!(
                "  warning: stale lockfile detected (PID {} no longer exists)",
                info.pid
            )
            .yellow()
        );
        println!("{}", "    -> Run `git-shadow restore`".yellow());
        println!();
    }

    if config.files.is_empty() {
        println!("no managed files");
        return Ok(());
    }

    if config.suspended {
        println!(
            "{}",
            "  status: SUSPENDED (run `git-shadow resume` to restore shadow changes)".yellow()
        );
        println!();
    }

    println!("managed files:");
    println!();

    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                println!("  {} (overlay)", file_path);
                if let Some(ref commit) = entry.baseline_commit {
                    println!("    baseline: {}", &commit[..7.min(commit.len())]);
                }

                // Show diff stats
                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                let worktree_path = git.root.join(file_path);

                if !worktree_path.exists() {
                    println!(
                        "{}",
                        "    warning: file does not exist in working tree".yellow()
                    );
                } else if baseline_path.exists() {
                    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
                    let current = std::fs::read_to_string(&worktree_path).unwrap_or_default();
                    let (added, removed) = diff_stats(&baseline, &current);
                    println!("    shadow changes: +{} lines / -{} lines", added, removed);

                    // Check baseline drift (hash mismatch + content comparison)
                    if let Some(ref commit) = entry.baseline_commit {
                        if let Ok(head) = git.head_commit() {
                            if *commit != head {
                                // Hash differs — check if file content actually changed
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
                                        format!(
                                            "    warning: baseline is outdated ({} -> {})",
                                            &commit[..7.min(commit.len())],
                                            &head[..7.min(head.len())]
                                        )
                                        .yellow()
                                    );
                                    println!(
                                        "{}",
                                        format!("    -> Run `git-shadow rebase {}`", file_path)
                                            .yellow()
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
                    "phantom dir"
                } else {
                    "phantom"
                };
                println!("  {} ({})", file_path, label);
                match entry.exclude_mode {
                    crate::config::ExcludeMode::GitInfoExclude => {
                        println!("    exclude: .git/info/exclude");
                    }
                    crate::config::ExcludeMode::None => {
                        println!("    exclude: none (hook protection only)");
                    }
                }
                let worktree_path = git.root.join(file_path);
                if entry.is_directory {
                    if worktree_path.is_dir() {
                        let count = std::fs::read_dir(&worktree_path)
                            .map(|entries| entries.count())
                            .unwrap_or(0);
                        println!("    contents: {} entries", count);
                    } else {
                        println!("{}", "    warning: directory does not exist".yellow());
                    }
                } else if worktree_path.exists() {
                    let metadata = std::fs::metadata(&worktree_path)?;
                    println!("    file size: {}", format_size(metadata.len()));
                } else {
                    println!("{}", "    warning: file does not exist".yellow());
                }
                println!();
            }
        }
    }

    Ok(())
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
}
