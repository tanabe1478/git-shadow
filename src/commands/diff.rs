use anyhow::Result;

use crate::config::{FileEntry, FileType, ShadowConfig};
use crate::diff_util;
use crate::error::ShadowError;
use crate::git::GitRepo;
use crate::path;
use crate::ui;

pub fn run(file: Option<&str>) -> Result<()> {
    let locale = ui::detect_locale();
    let cwd = std::env::current_dir()?;
    let git = GitRepo::discover(&cwd)?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    if config.suspended {
        return Err(ShadowError::Suspended.into());
    }

    if config.files.is_empty() {
        println!("{}", ui::no_managed_files(locale));
        return Ok(());
    }

    let mut found = false;

    for (file_path, entry) in &config.files {
        if let Some(target) = file {
            let normalized = path::normalize_path(target, &cwd, &git.root)?;
            if *file_path != normalized {
                continue;
            }
        }
        found = true;

        match entry.file_type {
            FileType::Overlay => {
                show_overlay_diff(&git, file_path, locale)?;
            }
            FileType::Phantom => {
                show_phantom_diff(&git, file_path, entry, locale)?;
            }
        }
    }

    if !found {
        if let Some(target) = file {
            println!("{}", ui::diff_not_managed(locale, target));
        }
    }

    Ok(())
}

fn show_overlay_diff(git: &GitRepo, file_path: &str, locale: ui::UiLocale) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
    let current = std::fs::read_to_string(&worktree_path).unwrap_or_default();

    if baseline == current {
        println!("{}", ui::diff_no_shadow_changes(locale, file_path));
        return Ok(());
    }

    diff_util::print_colored_diff(
        &baseline,
        &current,
        &ui::diff_baseline_label(locale, file_path),
        &ui::diff_shadow_label(locale, file_path),
    );

    Ok(())
}

fn show_phantom_diff(
    git: &GitRepo,
    file_path: &str,
    entry: &FileEntry,
    locale: ui::UiLocale,
) -> Result<()> {
    let worktree_path = git.root.join(file_path);

    if entry.is_directory {
        if worktree_path.is_dir() {
            let count = std::fs::read_dir(&worktree_path)
                .map(|entries| entries.count())
                .unwrap_or(0);
            println!("{}", ui::diff_phantom_directory(locale, file_path, count));
        } else {
            println!("{}", ui::diff_phantom_directory_missing(locale, file_path));
        }
        return Ok(());
    }

    if !worktree_path.exists() {
        println!("{}", ui::diff_file_missing(locale, file_path));
        return Ok(());
    }

    let content = std::fs::read_to_string(&worktree_path).unwrap_or_default();
    diff_util::print_new_file_diff(&content, file_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{ExcludeMode, ShadowConfig};
    use crate::diff_util;
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
    fn test_overlay_diff_shows_changes() {
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

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        // Generate diff
        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        let current = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        let diff = diff_util::unified_diff(
            &baseline,
            &current,
            "a/CLAUDE.md (baseline)",
            "b/CLAUDE.md (shadow)",
        );

        assert!(diff.contains("+# My shadow"));
        assert!(diff.contains("--- a/CLAUDE.md (baseline)"));
        assert!(diff.contains("+++ b/CLAUDE.md (shadow)"));
    }

    #[test]
    fn test_overlay_no_changes() {
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

        // No shadow changes - content matches baseline
        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        let current = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();

        assert_eq!(baseline, current);
    }

    #[test]
    fn test_phantom_shows_full_content() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();

        std::fs::write(git.root.join("local.md"), "# Local\nline2\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // For phantom, we show all content as new
        let content = std::fs::read_to_string(git.root.join("local.md")).unwrap();
        assert!(content.contains("# Local"));
        assert!(content.contains("line2"));
    }

    #[test]
    fn test_diff_specific_file() {
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

        std::fs::write(git.root.join("local.md"), "# Local\n").unwrap();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();

        config.save(&git.shadow_dir).unwrap();

        // Verify we can match specific file
        let normalized = path::normalize_path("CLAUDE.md", &git.root, &git.root).unwrap();
        assert_eq!(normalized, "CLAUDE.md");
        assert!(config.get(&normalized).is_some());
    }
}
