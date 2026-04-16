use std::path::Path;
use std::process::Command;

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git(root, &["init"]);
    run_git(root, &["config", "user.name", "Test"]);
    run_git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "init"]);

    dir
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_shadow(
    root: &Path,
    bin_dir: &Path,
    locale: &str,
    args: &[&str],
) -> std::process::Output {
    let path = prepend_path(bin_dir);
    Command::new("git-shadow")
        .args(args)
        .current_dir(root)
        .env("PATH", path)
        .env("LANG", locale)
        .output()
        .unwrap()
}

fn prepend_path(bin_dir: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin_dir.to_path_buf()];
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn test_not_initialized_message_is_english_for_english_locale() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["add", "tracked.txt"]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("git-shadow is not initialized yet"));
    assert!(stderr.contains("git-shadow install"));
}

#[test]
fn test_partial_stage_message_is_japanese_for_japanese_locale() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let install = run_git_shadow(repo.path(), bin_dir, "ja_JP.UTF-8", &["install"]);
    assert!(install.status.success());

    let add = run_git_shadow(repo.path(), bin_dir, "ja_JP.UTF-8", &["add", "tracked.txt"]);
    assert!(add.status.success());

    std::fs::write(repo.path().join("tracked.txt"), "base\nline1\nline2\n").unwrap();
    run_git(repo.path(), &["add", "tracked.txt"]);
    std::fs::write(
        repo.path().join("tracked.txt"),
        "base\nline1\nline2\nline3\n",
    )
    .unwrap();

    let output = Command::new("git")
        .args(["commit", "-m", "partial"])
        .current_dir(repo.path())
        .env("PATH", prepend_path(bin_dir))
        .env("LANG", "ja_JP.UTF-8")
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("コミットを止めました"));
    assert!(stderr.contains("git add tracked.txt"));
}

#[test]
fn test_help_is_japanese_for_japanese_locale() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let output = run_git_shadow(repo.path(), bin_dir, "ja_JP.UTF-8", &["--help"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Git リポジトリ内のローカル専用変更を管理します"));
    assert!(stdout.contains("管理対象ファイルと状態を表示する"));
}

#[test]
fn test_status_is_english_for_english_locale() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let install = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["install"]);
    assert!(install.status.success());
    let add = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["add", "tracked.txt"]);
    assert!(add.status.success());

    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["status"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("managed files:"));
    assert!(stdout.contains("shadow changes: +0 lines / -0 lines"));
}
