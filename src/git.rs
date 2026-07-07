use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};

use crate::error::ShadowError;

pub struct GitRepo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub shadow_dir: PathBuf,
}

impl GitRepo {
    /// Discover git repo from current or given directory
    pub fn discover(start: &Path) -> anyhow::Result<Self> {
        // Try --path-format=absolute first (Git 2.31+), fall back to manual resolution
        let output = Command::new("git")
            .args([
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-dir",
                "--git-common-dir",
            ])
            .current_dir(start)
            .output()
            .context("failed to run git command")?;

        if !output.status.success() {
            // Fallback: try without --path-format=absolute for older Git
            return Self::discover_fallback(start);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.trim().lines().collect();

        if lines.len() < 3 {
            // --path-format=absolute may not be supported; fall back
            return Self::discover_fallback(start);
        }

        let root = PathBuf::from(lines[0]);
        let git_dir = PathBuf::from(lines[1]);
        let common_dir = PathBuf::from(lines[2]);
        let shadow_dir = git_dir.join("shadow");

        Ok(Self {
            root,
            git_dir,
            common_dir,
            shadow_dir,
        })
    }

    /// Fallback discovery for Git < 2.31 (no --path-format=absolute)
    fn discover_fallback(start: &Path) -> anyhow::Result<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(start)
            .output()
            .context("failed to run git command")?;

        if !output.status.success() {
            return Err(ShadowError::NotAGitRepo.into());
        }

        let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());

        // Resolve git_dir
        let git_dir_output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(start)
            .output()
            .context("failed to run git rev-parse --git-dir")?;

        if !git_dir_output.status.success() {
            bail!("git rev-parse --git-dir failed");
        }

        let git_dir_raw = String::from_utf8_lossy(&git_dir_output.stdout)
            .trim()
            .to_string();
        let git_dir = Self::resolve_path(start, &git_dir_raw)?;

        // Resolve common_dir (Git 2.5+), fall back to git_dir
        let common_dir = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(start)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                let raw = String::from_utf8_lossy(&o.stdout).trim().to_string();
                Self::resolve_path(start, &raw).unwrap_or_else(|_| git_dir.clone())
            })
            .unwrap_or_else(|| git_dir.clone());

        let shadow_dir = git_dir.join("shadow");

        Ok(Self {
            root,
            git_dir,
            common_dir,
            shadow_dir,
        })
    }

    /// Resolve a possibly-relative path against a base directory
    fn resolve_path(base: &Path, raw: &str) -> anyhow::Result<PathBuf> {
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            Ok(path)
        } else {
            base.join(&path)
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", raw))
        }
    }

    /// Get the default hooks directory (lives under common_dir, shared across worktrees).
    ///
    /// This ignores `core.hooksPath`; use [`GitRepo::effective_hooks_dir`] when you need
    /// the directory Git actually runs hooks from.
    pub fn hooks_dir(&self) -> PathBuf {
        self.common_dir.join("hooks")
    }

    /// Read the `core.hooksPath` configuration value.
    ///
    /// Returns `None` when it is unset or empty. The value may still be absolute or
    /// relative; relative values are resolved against the working-tree root by
    /// [`GitRepo::effective_hooks_dir`].
    ///
    /// The value is queried with `--type=path` so Git expands a leading `~`/`~user`
    /// (e.g. `~/.git-hooks-global` -> `/home/you/.git-hooks-global`). Without this, a
    /// tilde value would be treated as repo-relative and resolve under `<root>/~/...`.
    /// `--type=path` requires Git >= 2.18; on older Git the typed query fails and we fall
    /// back to the raw read (which cannot expand `~`, matching the previous behavior).
    pub fn hooks_path_config(&self) -> Option<String> {
        self.read_hooks_path(&["config", "--type=path", "--get", "core.hooksPath"])
            .or_else(|| self.read_hooks_path(&["config", "--get", "core.hooksPath"]))
    }

    /// Run a `git config` query for `core.hooksPath`, returning the trimmed value or
    /// `None` when the command fails or the value is unset/empty.
    fn read_hooks_path(&self, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }

    /// Resolve the effective hooks directory Git will run hooks from.
    ///
    /// When `core.hooksPath` is set (e.g. by husky, lefthook, or this repo's own
    /// `dev-hooks` recommendation), hooks in the default `common_dir/hooks` never run,
    /// so shadow hooks must be installed into the custom directory instead. When it is
    /// unset, this falls back to [`GitRepo::hooks_dir`] to preserve worktree semantics
    /// (hooks are shared via `common_dir`).
    pub fn effective_hooks_dir(&self) -> PathBuf {
        match self.hooks_path_config() {
            Some(hooks_path) => {
                let path = PathBuf::from(&hooks_path);
                if path.is_absolute() {
                    path
                } else {
                    self.root.join(path)
                }
            }
            None => self.hooks_dir(),
        }
    }

    /// Enumerate the working-tree paths of all worktrees attached to this repository.
    ///
    /// Uses `git worktree list --porcelain`. If enumeration fails (e.g. Git too old),
    /// falls back to just this worktree's root so callers keep working.
    pub fn list_worktree_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        if let Ok(output) = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.root)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Some(rest) = line.strip_prefix("worktree ") {
                        paths.push(PathBuf::from(rest.trim()));
                    }
                }
            }
        }

        if paths.is_empty() {
            paths.push(self.root.clone());
        }

        paths
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
            .context("failed to run git show")?;

        if !output.status.success() {
            bail!(
                "git show {} failed: {}",
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
            .context("failed to run git ls-files")?;

        Ok(output.status.success())
    }

    /// Check staging status for partial staging detection
    /// Returns (index_differs_from_head, worktree_differs_from_index)
    pub fn staging_status(&self, path: &str) -> anyhow::Result<(bool, bool)> {
        let output = Command::new("git")
            .args(["status", "--porcelain=v2", "--", path])
            .current_dir(&self.root)
            .output()
            .context("failed to run git status")?;

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

    /// Return `git status --short --branch` output.
    pub fn status_short(&self) -> anyhow::Result<String> {
        let output = Command::new("git")
            .args(["status", "--short", "--branch"])
            .current_dir(&self.root)
            .output()
            .context("failed to run git status")?;

        if !output.status.success() {
            bail!(
                "git status --short --branch failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
        let hooks_dir = self.effective_hooks_dir();
        ["pre-commit", "post-commit", "post-merge", "post-rewrite"]
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

    /// Check whether install-created shadow directories exist.
    pub fn ensure_initialized(&self) -> Result<(), ShadowError> {
        let baselines_dir = self.shadow_dir.join("baselines");
        let stash_dir = self.shadow_dir.join("stash");

        if baselines_dir.is_dir() && stash_dir.is_dir() {
            Ok(())
        } else {
            Err(ShadowError::NotInitialized)
        }
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

    #[test]
    fn test_discover_normal_repo_common_dir_equals_git_dir() {
        let (_dir, repo) = make_test_repo();
        assert_eq!(repo.common_dir, repo.git_dir);
    }

    #[test]
    fn test_discover_in_worktree() {
        let (dir, _repo) = make_test_repo();
        let root = dir.path().to_path_buf();

        // Create a worktree
        let wt_path = dir.path().join("worktree");
        run_cmd(
            &root,
            "git",
            &[
                "worktree",
                "add",
                "-b",
                "wt-branch",
                wt_path.to_str().unwrap(),
            ],
        );

        let wt_repo = GitRepo::discover(&wt_path).unwrap();

        // root should be the worktree path
        assert_eq!(
            wt_repo.root.canonicalize().unwrap(),
            wt_path.canonicalize().unwrap()
        );

        // git_dir should be under .git/worktrees/
        assert!(
            wt_repo.git_dir.to_str().unwrap().contains("worktrees"),
            "git_dir should be under worktrees/: {:?}",
            wt_repo.git_dir
        );

        // shadow_dir should be under git_dir (per-worktree)
        assert!(wt_repo.shadow_dir.starts_with(&wt_repo.git_dir));
    }

    #[test]
    fn test_discover_common_dir_in_worktree() {
        let (dir, main_repo) = make_test_repo();
        let root = dir.path().to_path_buf();

        let wt_path = dir.path().join("worktree2");
        run_cmd(
            &root,
            "git",
            &[
                "worktree",
                "add",
                "-b",
                "wt-branch2",
                wt_path.to_str().unwrap(),
            ],
        );

        let wt_repo = GitRepo::discover(&wt_path).unwrap();

        // common_dir should point to main repo's .git
        assert_eq!(
            wt_repo.common_dir.canonicalize().unwrap(),
            main_repo.git_dir.canonicalize().unwrap()
        );

        // git_dir should differ from common_dir in worktree
        assert_ne!(wt_repo.git_dir, wt_repo.common_dir);
    }

    #[test]
    fn test_hooks_path_config_none_when_unset() {
        let (_dir, repo) = make_test_repo();
        assert!(repo.hooks_path_config().is_none());
        assert_eq!(repo.effective_hooks_dir(), repo.hooks_dir());
    }

    #[test]
    fn test_effective_hooks_dir_honors_relative_hooks_path() {
        let (_dir, repo) = make_test_repo();
        run_cmd(
            &repo.root,
            "git",
            &["config", "core.hooksPath", "dev-hooks"],
        );

        assert_eq!(repo.hooks_path_config().as_deref(), Some("dev-hooks"));
        assert_eq!(repo.effective_hooks_dir(), repo.root.join("dev-hooks"));
    }

    #[test]
    fn test_hooks_path_config_expands_tilde() {
        // A `~`-prefixed hooksPath must be expanded to an absolute path by Git
        // (`--type=path`), not treated as the repo-relative directory `<root>/~/...`.
        let (_dir, repo) = make_test_repo();
        run_cmd(
            &repo.root,
            "git",
            &["config", "core.hooksPath", "~/git-shadow-test-hooks"],
        );

        // HOME governs `~` expansion on Unix; skip if the runner has none.
        if std::env::var_os("HOME").is_none() {
            return;
        }

        let resolved = repo
            .hooks_path_config()
            .expect("hooksPath should be reported when set");
        assert!(
            !resolved.starts_with('~'),
            "tilde should be expanded, got: {resolved}"
        );
        assert!(
            PathBuf::from(&resolved).is_absolute(),
            "expanded hooksPath should be absolute, got: {resolved}"
        );
        assert!(
            resolved.ends_with("git-shadow-test-hooks"),
            "expanded hooksPath should keep the suffix, got: {resolved}"
        );

        // effective_hooks_dir must use the expanded absolute path verbatim, not join it
        // onto the repo root.
        assert_eq!(repo.effective_hooks_dir(), PathBuf::from(&resolved));
    }

    #[test]
    fn test_effective_hooks_dir_honors_absolute_hooks_path() {
        let (_dir, repo) = make_test_repo();
        let abs = repo.root.join("custom-hooks");
        run_cmd(
            &repo.root,
            "git",
            &["config", "core.hooksPath", abs.to_str().unwrap()],
        );

        assert_eq!(repo.effective_hooks_dir(), abs);
    }

    #[test]
    fn test_hooks_dir_uses_common_dir() {
        let (dir, _repo) = make_test_repo();
        let root = dir.path().to_path_buf();

        let wt_path = dir.path().join("worktree3");
        run_cmd(
            &root,
            "git",
            &[
                "worktree",
                "add",
                "-b",
                "wt-branch3",
                wt_path.to_str().unwrap(),
            ],
        );

        let wt_repo = GitRepo::discover(&wt_path).unwrap();

        // hooks_dir should be under common_dir, not git_dir
        assert!(wt_repo.hooks_dir().starts_with(&wt_repo.common_dir));
        assert!(!wt_repo.hooks_dir().starts_with(&wt_repo.git_dir));
    }
}
