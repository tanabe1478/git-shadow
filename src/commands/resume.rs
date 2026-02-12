use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::fs_util;
use crate::git::GitRepo;
use crate::merge;
use crate::path;

pub fn run() -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let mut config = ShadowConfig::load(&git.shadow_dir)?;

    // Guard: not suspended
    if !config.suspended {
        return Err(ShadowError::NotSuspended.into());
    }

    let suspended_dir = git.shadow_dir.join("suspended");
    let head = git.head_commit()?;
    let mut count = 0;

    let file_paths: Vec<(String, FileType, bool)> = config
        .files
        .iter()
        .map(|(p, e)| (p.clone(), e.file_type.clone(), e.is_directory))
        .collect();

    for (file_path, file_type, is_directory) in &file_paths {
        match file_type {
            FileType::Overlay => {
                resume_overlay(&git, &mut config, &suspended_dir, file_path, &head)?;
                count += 1;
            }
            FileType::Phantom => {
                if !is_directory {
                    resume_phantom(&git, &suspended_dir, file_path)?;
                    count += 1;
                }
            }
        }
    }

    // Clean up suspended directory
    if suspended_dir.exists() {
        std::fs::remove_dir_all(&suspended_dir)
            .context("failed to clean up suspended directory")?;
    }

    config.suspended = false;
    config.save(&git.shadow_dir)?;

    println!(
        "{}",
        format!("shadow changes resumed for {} file(s)", count).green()
    );

    Ok(())
}

fn resume_overlay(
    git: &GitRepo,
    config: &mut ShadowConfig,
    suspended_dir: &std::path::Path,
    file_path: &str,
    new_head: &str,
) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let suspend_path = suspended_dir.join(&encoded);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    // Ensure parent directory exists (may be missing after branch switch)
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory for {}", file_path))?;
    }

    if !suspend_path.exists() {
        eprintln!(
            "{}",
            format!("warning: no suspended content for {}", file_path).yellow()
        );
        return Ok(());
    }

    let suspended_content = std::fs::read_to_string(&suspend_path)
        .with_context(|| format!("failed to read suspended content for {}", file_path))?;
    let old_baseline = std::fs::read_to_string(&baseline_path)
        .with_context(|| format!("failed to read baseline for {}", file_path))?;

    // Get current HEAD content for this file
    let new_baseline = match git.show_file("HEAD", file_path) {
        Ok(content) => String::from_utf8_lossy(&content).to_string(),
        Err(_) => {
            // File deleted in new branch — just restore the suspended content
            std::fs::write(&worktree_path, suspended_content.as_bytes())
                .with_context(|| format!("failed to restore {}", file_path))?;
            println!(
                "{}: shadow changes restored (file absent from HEAD)",
                file_path
            );
            return Ok(());
        }
    };

    if old_baseline == new_baseline {
        // Baseline unchanged — restore suspended content directly
        std::fs::write(&worktree_path, suspended_content.as_bytes())
            .with_context(|| format!("failed to restore {}", file_path))?;
        println!("{}: shadow changes restored", file_path);
    } else {
        // Baseline changed — 3-way merge
        let merge_result = merge::three_way_merge(
            &old_baseline,
            &suspended_content,
            &new_baseline,
            &git.shadow_dir,
        )?;

        std::fs::write(&worktree_path, merge_result.content.as_bytes())
            .with_context(|| format!("failed to write merged content for {}", file_path))?;

        // Update baseline
        fs_util::atomic_write(&baseline_path, new_baseline.as_bytes())
            .with_context(|| format!("failed to update baseline for {}", file_path))?;

        if let Some(entry) = config.files.get_mut(file_path) {
            entry.baseline_commit = Some(new_head.to_string());
        }

        if merge_result.has_conflicts {
            eprintln!(
                "{}",
                format!(
                    "warning: conflicts detected in {}. Please resolve manually",
                    file_path
                )
                .yellow()
            );
        } else {
            println!("{}: baseline updated and shadow changes merged", file_path);
        }
    }

    Ok(())
}

fn resume_phantom(git: &GitRepo, suspended_dir: &std::path::Path, file_path: &str) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let suspend_path = suspended_dir.join(&encoded);
    let worktree_path = git.root.join(file_path);

    if !suspend_path.exists() {
        eprintln!(
            "{}",
            format!("warning: no suspended content for {}", file_path).yellow()
        );
        return Ok(());
    }

    let content = std::fs::read(&suspend_path)
        .with_context(|| format!("failed to read suspended content for {}", file_path))?;

    // Ensure parent directory exists
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory for {}", file_path))?;
    }

    std::fs::write(&worktree_path, &content)
        .with_context(|| format!("failed to restore {}", file_path))?;

    println!("{}: phantom file restored", file_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::ShadowConfig;
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
    fn test_resume_overlay_same_baseline() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        // Setup overlay
        let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            &baseline_content,
        )
        .unwrap();
        config
            .add_overlay("CLAUDE.md".to_string(), commit.clone())
            .unwrap();

        // Simulate suspend: save shadow content to suspended/
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        fs_util::atomic_write(&suspended_dir.join(&encoded), b"# Team\n# My shadow\n").unwrap();

        // Working tree has baseline content (as after suspend)
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n").unwrap();

        // Resume
        super::resume_overlay(&git, &mut config, &suspended_dir, "CLAUDE.md", &commit).unwrap();

        // Working tree should have shadow content
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(wt, "# Team\n# My shadow\n");
    }

    #[test]
    fn test_resume_overlay_different_baseline_merges() {
        let (_dir, git) = make_test_repo();
        let old_commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        // Setup overlay with old baseline
        let old_baseline = "line1\nline2\nline3\n";
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            old_baseline.as_bytes(),
        )
        .unwrap();
        config
            .add_overlay("CLAUDE.md".to_string(), old_commit)
            .unwrap();

        // Write old content and commit
        std::fs::write(git.root.join("CLAUDE.md"), old_baseline).unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "set baseline"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        let mid_commit = git.head_commit().unwrap();

        // Update config to match
        if let Some(entry) = config.files.get_mut("CLAUDE.md") {
            entry.baseline_commit = Some(mid_commit);
        }
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            old_baseline.as_bytes(),
        )
        .unwrap();

        // Simulate suspend: shadow content was "line1\nline2\nline3\nmy addition\n"
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        let shadow_content = "line1\nline2\nline3\nmy addition\n";
        fs_util::atomic_write(&suspended_dir.join(&encoded), shadow_content.as_bytes()).unwrap();

        // Now simulate upstream change (new commit changes line2)
        let new_baseline = "line1\nline2 updated\nline3\n";
        std::fs::write(git.root.join("CLAUDE.md"), new_baseline).unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "upstream update"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        let new_head = git.head_commit().unwrap();

        // Resume — should 3-way merge
        super::resume_overlay(&git, &mut config, &suspended_dir, "CLAUDE.md", &new_head).unwrap();

        // Working tree should have merged content
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert!(wt.contains("line2 updated"), "should have upstream change");
        assert!(wt.contains("my addition"), "should preserve shadow change");

        // Baseline should be updated
        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        assert_eq!(baseline, new_baseline);

        // baseline_commit should be updated
        let entry = config.get("CLAUDE.md").unwrap();
        assert_eq!(entry.baseline_commit.as_ref().unwrap(), &new_head);
    }

    #[test]
    fn test_resume_phantom_restores_file() {
        let (_dir, git) = make_test_repo();

        // Setup phantom (file doesn't exist in working tree during suspend)
        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();
        let encoded = path::encode_path("local.md");
        fs_util::atomic_write(&suspended_dir.join(&encoded), b"# Local\n").unwrap();

        // Resume
        super::resume_phantom(&git, &suspended_dir, "local.md").unwrap();

        // Phantom should be restored to working tree
        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert_eq!(content, "# Local\n");
    }

    #[test]
    fn test_resume_clears_suspended_flag() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        config.suspended = true;
        config.save(&git.shadow_dir).unwrap();

        // Simulate resume
        config.suspended = false;
        config.save(&git.shadow_dir).unwrap();

        let loaded = ShadowConfig::load(&git.shadow_dir).unwrap();
        assert!(!loaded.suspended);
    }

    #[test]
    fn test_resume_not_suspended_is_error() {
        let config = ShadowConfig::new();
        assert!(!config.suspended);
    }

    #[test]
    fn test_resume_overlay_missing_suspended_file() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), commit.clone())
            .unwrap();

        let suspended_dir = git.shadow_dir.join("suspended");
        std::fs::create_dir_all(&suspended_dir).unwrap();

        // Resume with no suspended file — should warn but not error
        super::resume_overlay(&git, &mut config, &suspended_dir, "CLAUDE.md", &commit).unwrap();
    }
}
