use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};

use crate::error::ShadowError;

pub struct GitRepo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub shadow_dir: PathBuf,
}

impl GitRepo {
    /// Discover git repo from current or given directory
    pub fn discover(start: &Path) -> anyhow::Result<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(start)
            .output()
            .context("git コマンドの実行に失敗")?;

        if !output.status.success() {
            return Err(ShadowError::NotAGitRepo.into());
        }

        let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let git_dir = root.join(".git");
        let shadow_dir = git_dir.join("shadow");

        Ok(Self {
            root,
            git_dir,
            shadow_dir,
        })
    }

    /// Get current HEAD commit hash (full)
    pub fn head_commit(&self) -> anyhow::Result<String> {
        let output = self.run_git(&["rev-parse", "HEAD"])?;
        Ok(output.trim().to_string())
    }

    /// Read file content from a specific ref (e.g. "HEAD")
    pub fn show_file(&self, reference: &str, path: &str) -> anyhow::Result<Vec<u8>> {
        let spec = format!("{}:{}", reference, path);
        let output = Command::new("git")
            .args(["show", &spec])
            .current_dir(&self.root)
            .output()
            .context("git show の実行に失敗")?;

        if !output.status.success() {
            bail!(
                "git show {} 失敗: {}",
                spec,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(output.stdout)
    }

    /// Check if a file is tracked by git
    pub fn is_tracked(&self, path: &str) -> anyhow::Result<bool> {
        let output = Command::new("git")
            .args(["ls-files", "--error-unmatch", path])
            .current_dir(&self.root)
            .output()
            .context("git ls-files の実行に失敗")?;

        Ok(output.status.success())
    }

    /// Check staging status for partial staging detection
    /// Returns (index_differs_from_head, worktree_differs_from_index)
    pub fn staging_status(&self, path: &str) -> anyhow::Result<(bool, bool)> {
        let output = Command::new("git")
            .args(["status", "--porcelain=v2", "--", path])
            .current_dir(&self.root)
            .output()
            .context("git status の実行に失敗")?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            if !line.starts_with('1') && !line.starts_with('2') {
                continue;
            }
            // Format: "1 XY sub mH mI mW hH hI path"
            let parts: Vec<&str> = line.splitn(9, ' ').collect();
            if parts.len() < 2 {
                continue;
            }
            let xy = parts[1];
            let x = xy.chars().next().unwrap_or('.');
            let y = xy.chars().nth(1).unwrap_or('.');

            let index_changed = x != '.';
            let worktree_changed = y != '.';

            return Ok((index_changed, worktree_changed));
        }

        // File not in status output = clean
        Ok((false, false))
    }

    /// Stage a file (git add)
    pub fn add(&self, path: &str) -> anyhow::Result<()> {
        self.run_git(&["add", path])?;
        Ok(())
    }

    /// Unstage a phantom file (try multiple strategies)
    pub fn unstage_phantom(&self, path: &str) -> Result<(), ShadowError> {
        // Strategy 1: git rm --cached --ignore-unmatch
        if self
            .run_git(&["rm", "--cached", "--ignore-unmatch", path])
            .is_ok()
        {
            return Ok(());
        }

        // Strategy 2: git restore --staged
        if self.run_git(&["restore", "--staged", path]).is_ok() {
            return Ok(());
        }

        // Strategy 3: git reset -- <file>
        if self.run_git(&["reset", "--", path]).is_ok() {
            return Ok(());
        }

        Err(ShadowError::UnstageFailure(path.to_string()))
    }

    /// Check if hooks are installed
    pub fn hooks_installed(&self) -> bool {
        let hooks_dir = self.git_dir.join("hooks");
        ["pre-commit", "post-commit", "post-merge"]
            .iter()
            .all(|name| {
                let hook = hooks_dir.join(name);
                if let Ok(content) = std::fs::read_to_string(&hook) {
                    content.contains("git-shadow hook")
                } else {
                    false
                }
            })
    }

    /// Run a git command and return stdout
    fn run_git(&self, args: &[&str]) -> Result<String, ShadowError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| ShadowError::GitCommand {
                command: format!("git {}", args.join(" ")),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ShadowError::GitCommand {
                command: format!("git {}", args.join(" ")),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        run_cmd(&root, "git", &["init"]);
        run_cmd(&root, "git", &["config", "user.name", "Test"]);
        run_cmd(&root, "git", &["config", "user.email", "t@t.com"]);

        std::fs::write(root.join("CLAUDE.md"), "# Test\n").unwrap();
        run_cmd(&root, "git", &["add", "CLAUDE.md"]);
        run_cmd(&root, "git", &["commit", "-m", "init"]);

        let repo = GitRepo::discover(&root).unwrap();
        (dir, repo)
    }

    fn run_cmd(cwd: &Path, cmd: &str, args: &[&str]) {
        let output = Command::new(cmd)
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        if !output.status.success() {
            panic!(
                "{} {} failed: {}",
                cmd,
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn test_discover_from_root() {
        let (_dir, repo) = make_test_repo();
        assert!(repo.root.exists());
        assert!(repo.git_dir.exists());
    }

    #[test]
    fn test_discover_from_subdir() {
        let (_dir, repo) = make_test_repo();
        let sub = repo.root.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let found = GitRepo::discover(&sub).unwrap();
        assert_eq!(found.root, repo.root);
    }

    #[test]
    fn test_discover_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = GitRepo::discover(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_head_commit() {
        let (_dir, repo) = make_test_repo();
        let hash = repo.head_commit().unwrap();
        assert_eq!(hash.len(), 40); // Full SHA
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_show_file() {
        let (_dir, repo) = make_test_repo();
        let content = repo.show_file("HEAD", "CLAUDE.md").unwrap();
        assert_eq!(String::from_utf8_lossy(&content), "# Test\n");
    }

    #[test]
    fn test_is_tracked_true() {
        let (_dir, repo) = make_test_repo();
        assert!(repo.is_tracked("CLAUDE.md").unwrap());
    }

    #[test]
    fn test_is_tracked_false() {
        let (_dir, repo) = make_test_repo();
        assert!(!repo.is_tracked("nonexistent.md").unwrap());
    }

    #[test]
    fn test_staging_status_clean() {
        let (_dir, repo) = make_test_repo();
        let (idx, wt) = repo.staging_status("CLAUDE.md").unwrap();
        assert!(!idx);
        assert!(!wt);
    }

    #[test]
    fn test_staging_status_fully_staged() {
        let (_dir, repo) = make_test_repo();
        std::fs::write(repo.root.join("CLAUDE.md"), "# Modified\n").unwrap();
        run_cmd(&repo.root, "git", &["add", "CLAUDE.md"]);

        let (idx, wt) = repo.staging_status("CLAUDE.md").unwrap();
        assert!(idx); // index differs from HEAD
        assert!(!wt); // worktree matches index
    }

    #[test]
    fn test_staging_status_partial() {
        let (_dir, repo) = make_test_repo();
        // Stage a change
        std::fs::write(repo.root.join("CLAUDE.md"), "# Staged\n").unwrap();
        run_cmd(&repo.root, "git", &["add", "CLAUDE.md"]);
        // Make another change in worktree
        std::fs::write(repo.root.join("CLAUDE.md"), "# Partial\n").unwrap();

        let (idx, wt) = repo.staging_status("CLAUDE.md").unwrap();
        assert!(idx); // index differs from HEAD
        assert!(wt); // worktree differs from index
    }

    #[test]
    fn test_add_stages_file() {
        let (_dir, repo) = make_test_repo();
        std::fs::write(repo.root.join("new.txt"), "new").unwrap();
        repo.add("new.txt").unwrap();

        let output = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(&repo.root)
            .output()
            .unwrap();
        let staged = String::from_utf8_lossy(&output.stdout);
        assert!(staged.contains("new.txt"));
    }

    #[test]
    fn test_hooks_installed_false() {
        let (_dir, repo) = make_test_repo();
        assert!(!repo.hooks_installed());
    }
}
