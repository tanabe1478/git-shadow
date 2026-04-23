use anyhow::Result;

use crate::commands::rebase;
use crate::git::GitRepo;

pub fn handle(git: &GitRepo) -> Result<()> {
    rebase::auto_rebase_all(git, "post-merge")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShadowConfig;
    use crate::fs_util;
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
    fn test_post_merge_auto_rebases_clean_changes() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("CLAUDE.md"), "line1\nline2\nline3\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "set base"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let old_commit = git.head_commit().unwrap();
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), old_commit)
            .unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        fs_util::atomic_write(
            &git.shadow_dir.join("baselines").join(&encoded),
            b"line1\nline2\nline3\n",
        )
        .unwrap();
        config.save(&git.shadow_dir).unwrap();

        std::fs::write(git.root.join("CLAUDE.md"), "line1\nline2 modified\nline3\n").unwrap();
        std::fs::write(git.root.join("CLAUDE.md"), "line1\nline2\nline3\nline4\n").unwrap();
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
        std::fs::write(git.root.join("CLAUDE.md"), "line1\nline2 modified\nline3\n").unwrap();

        handle(&git).unwrap();

        let baseline =
            std::fs::read_to_string(git.shadow_dir.join("baselines").join(&encoded)).unwrap();
        assert_eq!(baseline, "line1\nline2\nline3\nline4\n");
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert!(wt.contains("line2 modified"));
        assert!(wt.contains("line4"));
    }
}
