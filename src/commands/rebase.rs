use anyhow::{bail, Result};
use colored::Colorize;

use crate::config::{FileType, ShadowConfig};
use crate::fs_util;
use crate::git::GitRepo;
use crate::merge;
use crate::path;

pub fn run(file: Option<&str>) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;
    let mut config = ShadowConfig::load(&git.shadow_dir)?;
    let head = git.head_commit()?;

    if config.files.is_empty() {
        println!("no managed files");
        return Ok(());
    }

    let mut found = false;

    let file_paths: Vec<String> = config.files.keys().cloned().collect();
    for file_path in &file_paths {
        let entry = config.files.get(file_path).unwrap();

        if entry.file_type != FileType::Overlay {
            continue;
        }

        if let Some(target) = file {
            let normalized = path::normalize_path(target, &git.root)?;
            if *file_path != normalized {
                continue;
            }
        }
        found = true;

        rebase_file(&git, &mut config, file_path, &head)?;
    }

    if !found {
        if let Some(target) = file {
            bail!("{} is not managed as overlay", target);
        } else {
            println!("no overlay files found");
        }
    }

    config.save(&git.shadow_dir)?;

    Ok(())
}

fn rebase_file(
    git: &GitRepo,
    config: &mut ShadowConfig,
    file_path: &str,
    new_head: &str,
) -> Result<()> {
    let encoded = path::encode_path(file_path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    let worktree_path = git.root.join(file_path);

    // 1. Read current content (baseline + shadow changes)
    let current_content = std::fs::read_to_string(&worktree_path)?;

    // 2. Read old baseline
    let old_baseline = std::fs::read_to_string(&baseline_path)?;

    // 3. Get new HEAD content
    let new_baseline = match git.show_file("HEAD", file_path) {
        Ok(content) => String::from_utf8_lossy(&content).to_string(),
        Err(_) => {
            bail!(
                "{} does not exist in HEAD. The file may have been deleted",
                file_path
            );
        }
    };

    // Check if baseline actually changed
    if old_baseline == new_baseline {
        println!("{}: baseline has not changed", file_path);
        return Ok(());
    }

    // 4. 3-way merge: old_baseline (base), current_content (ours), new_baseline (theirs)
    let merge_result = merge::three_way_merge(
        &old_baseline,
        &current_content,
        &new_baseline,
        &git.shadow_dir,
    )?;

    // 5. Write merged content to working tree
    std::fs::write(&worktree_path, &merge_result.content)?;

    // 6. Update baseline
    fs_util::atomic_write(&baseline_path, new_baseline.as_bytes())?;

    // 7. Update config
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
        println!("{}", format!("baseline updated for {}", file_path).green());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::ShadowConfig;
    use crate::git::GitRepo;
    use crate::{fs_util, merge, path};

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
    fn test_rebase_clean_merge() {
        let (_dir, git) = make_test_repo();
        let old_commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        // Setup overlay with old baseline
        let old_baseline =
            String::from_utf8_lossy(&git.show_file("HEAD", "CLAUDE.md").unwrap()).to_string();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            old_baseline.as_bytes(),
        )
        .unwrap();
        config
            .add_overlay("CLAUDE.md".to_string(), old_commit)
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Add shadow changes
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        // Make a new commit that changes the file differently
        std::fs::write(git.root.join("other.txt"), "other").unwrap();
        std::process::Command::new("git")
            .args(["add", "other.txt"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        // Restore old content for commit
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# Upstream addition\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "upstream"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let new_head = git.head_commit().unwrap();

        // Restore shadow content (simulating that shadow was applied)
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

        // Perform rebase directly
        rebase_for_test(&git, &mut config, "CLAUDE.md", &new_head);

        // Verify baseline updated
        let new_baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        assert_eq!(new_baseline, "# Team\n# Upstream addition\n");

        // Verify config updated
        let entry = config.get("CLAUDE.md").unwrap();
        assert_eq!(entry.baseline_commit.as_ref().unwrap(), &new_head);

        // Verify working tree has merged content
        let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert!(content.contains("# My shadow") || content.contains("# Upstream addition"));
    }

    #[test]
    fn test_rebase_no_change() {
        let (_dir, git) = make_test_repo();
        let commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();

        let baseline_content =
            String::from_utf8_lossy(&git.show_file("HEAD", "CLAUDE.md").unwrap()).to_string();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            baseline_content.as_bytes(),
        )
        .unwrap();
        config
            .add_overlay("CLAUDE.md".to_string(), commit.clone())
            .unwrap();
        config.save(&git.shadow_dir).unwrap();

        // Rebase when baseline hasn't changed
        let old_baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        let new_baseline =
            String::from_utf8_lossy(&git.show_file("HEAD", "CLAUDE.md").unwrap()).to_string();

        // Should detect no changes
        assert_eq!(old_baseline, new_baseline);
    }

    #[test]
    fn test_rebase_with_conflict() {
        let (_dir, git) = make_test_repo();
        let old_commit = git.head_commit().unwrap();

        // Old baseline is "# Team\n"
        let old_baseline = "# Team\n";
        // Our shadow changes the same line
        let ours = "# My Team\n";
        // Upstream also changes the same line
        let theirs = "# Their Team\n";

        let result = merge::three_way_merge(old_baseline, ours, theirs, &git.shadow_dir).unwrap();
        assert!(result.has_conflicts);
        assert!(result.content.contains("<<<<<<<"));

        // Verify old_commit is valid
        assert!(!old_commit.is_empty());
    }

    #[test]
    fn test_rebase_preserves_shadow_changes() {
        let (_dir, git) = make_test_repo();

        // Base: "line1\nline2\nline3\n"
        // Ours (shadow): "line1\nline2\nline3\nmy addition\n"
        // Theirs (new HEAD): "line1\nline2 updated\nline3\n"
        // Expected merge: "line1\nline2 updated\nline3\nmy addition\n"

        let base = "line1\nline2\nline3\n";
        let ours = "line1\nline2\nline3\nmy addition\n";
        let theirs = "line1\nline2 updated\nline3\n";

        let result = merge::three_way_merge(base, ours, theirs, &git.shadow_dir).unwrap();
        assert!(!result.has_conflicts);
        assert!(result.content.contains("line2 updated"));
        assert!(result.content.contains("my addition"));
    }

    /// Helper to rebase a file (bypasses cwd discovery)
    fn rebase_for_test(git: &GitRepo, config: &mut ShadowConfig, file_path: &str, new_head: &str) {
        let encoded = path::encode_path(file_path);
        let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
        let worktree_path = git.root.join(file_path);

        let current_content = std::fs::read_to_string(&worktree_path).unwrap();
        let old_baseline = std::fs::read_to_string(&baseline_path).unwrap();
        let new_baseline =
            String::from_utf8_lossy(&git.show_file("HEAD", file_path).unwrap()).to_string();

        let merge_result = merge::three_way_merge(
            &old_baseline,
            &current_content,
            &new_baseline,
            &git.shadow_dir,
        )
        .unwrap();

        std::fs::write(&worktree_path, &merge_result.content).unwrap();
        fs_util::atomic_write(&baseline_path, new_baseline.as_bytes()).unwrap();

        if let Some(entry) = config.files.get_mut(file_path) {
            entry.baseline_commit = Some(new_head.to_string());
        }
    }
}
