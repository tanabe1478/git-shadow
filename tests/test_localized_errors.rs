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
    let mut cmd = Command::new("git-shadow");
    cmd.args(args).current_dir(root).env("PATH", path);
    apply_locale(&mut cmd, locale);
    cmd.output().unwrap()
}

/// Pin the process locale hermetically. `detect_locale()` reads
/// `LC_ALL` > `LC_MESSAGES` > `LANG`, so setting only `LANG` is not enough:
/// a runner that exports `LC_ALL`/`LC_MESSAGES` (macOS GitHub runners do)
/// would win and flip the detected locale. Clearing the higher-priority vars
/// and setting all three to the requested locale makes these assertions
/// independent of the ambient environment.
fn apply_locale(cmd: &mut Command, locale: &str) {
    cmd.env("LC_ALL", locale)
        .env("LC_MESSAGES", locale)
        .env("LANG", locale);
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

    let mut commit = Command::new("git");
    commit
        .args(["commit", "-m", "partial"])
        .current_dir(repo.path())
        .env("PATH", prepend_path(bin_dir));
    apply_locale(&mut commit, "ja_JP.UTF-8");
    let output = commit.output().unwrap();

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
fn test_version_flag_prints_version() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["--version"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version output should contain the crate version, got: {stdout}"
    );
}

#[test]
fn test_doctor_exits_nonzero_when_issues() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    // No install => hooks are missing => doctor reports issues => non-zero exit.
    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["doctor"]);
    assert!(
        !output.status.success(),
        "doctor should exit non-zero when issues are present"
    );
}

#[test]
fn test_doctor_json_is_valid_and_english() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    // Even under a Japanese locale, --json must stay English/stable.
    let output = run_git_shadow(repo.path(), bin_dir, "ja_JP.UTF-8", &["doctor", "--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("doctor --json valid JSON");
    assert!(value.get("ok").is_some());
    assert!(value.get("issues").is_some());
    assert!(value.get("warnings").is_some());
}

#[test]
fn test_status_json_is_valid() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let install = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["install"]);
    assert!(install.status.success());
    let add = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["add", "tracked.txt"]);
    assert!(add.status.success());

    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["status", "--json"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("status --json valid JSON");
    assert_eq!(value["suspended"], false);
    assert_eq!(value["files"][0]["path"], "tracked.txt");
    assert_eq!(value["files"][0]["type"], "overlay");
}

#[test]
fn test_uninstall_removes_hooks_and_state() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let install = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["install"]);
    assert!(install.status.success());

    // No managed files => clean uninstall succeeds.
    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["uninstall"]);
    assert!(output.status.success());

    let hooks = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let hooks_dir = repo
        .path()
        .join(String::from_utf8_lossy(&hooks.stdout).trim());
    assert!(!hooks_dir.join("pre-commit").exists());
}

#[test]
fn test_uninstall_refuses_with_active_entries() {
    let repo = init_repo();
    let bin = assert_cmd::cargo::cargo_bin!("git-shadow");
    let bin_dir = bin.parent().unwrap();

    let install = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["install"]);
    assert!(install.status.success());
    let add = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["add", "tracked.txt"]);
    assert!(add.status.success());

    let output = run_git_shadow(repo.path(), bin_dir, "en_US.UTF-8", &["uninstall"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("still managed"));

    // --force restores the overlay and wipes state.
    let forced = run_git_shadow(
        repo.path(),
        bin_dir,
        "en_US.UTF-8",
        &["uninstall", "--force"],
    );
    assert!(forced.status.success());
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
