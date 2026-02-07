use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct TestRepo {
    pub dir: TempDir,
    pub root: PathBuf,
}

impl TestRepo {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // git init
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.name", "Test User"]);
        run_git(&root, &["config", "user.email", "test@example.com"]);

        Self { dir, root }
    }

    pub fn create_file(&self, path: &str, content: &str) {
        let file_path = self.root.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, content).unwrap();
    }

    pub fn read_file(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }

    pub fn commit(&self, message: &str) {
        run_git(&self.root, &["add", "-A"]);
        run_git(&self.root, &["commit", "-m", message]);
    }

    pub fn git_dir(&self) -> PathBuf {
        self.root.join(".git")
    }

    pub fn shadow_dir(&self) -> PathBuf {
        self.root.join(".git").join("shadow")
    }

    pub fn init_shadow(&self) {
        let shadow_dir = self.shadow_dir();
        std::fs::create_dir_all(shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(shadow_dir.join("stash")).unwrap();
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    if !output.status.success() {
        panic!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}
