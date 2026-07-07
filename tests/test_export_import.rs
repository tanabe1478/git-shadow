//! E2E tests for `git-shadow export` / `import`.
//!
//! These install the real hooks via `git-shadow install` and run the built
//! binary from `PATH` (so committed hooks resolve it), mirroring the pattern in
//! `test_git_operations.rs` / `test_localized_errors.rs`. Locale is pinned
//! hermetically via `apply_locale` so assertions do not depend on the runner's
//! ambient `LC_ALL` / `LC_MESSAGES` / `LANG`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin_dir() -> PathBuf {
    assert_cmd::cargo::cargo_bin!("git-shadow")
        .parent()
        .unwrap()
        .to_path_buf()
}

fn prepend_path(dir: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Pin the process locale hermetically (see `test_localized_errors.rs`).
fn apply_locale(cmd: &mut Command, locale: &str) {
    cmd.env("LC_ALL", locale)
        .env("LC_MESSAGES", locale)
        .env("LANG", locale);
}

fn git(root: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .env("PATH", prepend_path(&bin_dir()));
    apply_locale(&mut cmd, "en_US.UTF-8");
    cmd.output().unwrap()
}

fn git_ok(root: &Path, args: &[&str]) -> Output {
    let out = git(root, args);
    assert!(
        out.status.success(),
        "git {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn shadow(root: &Path, locale: &str, args: &[&str]) -> Output {
    let mut cmd = Command::new("git-shadow");
    cmd.args(args)
        .current_dir(root)
        .env("PATH", prepend_path(&bin_dir()));
    apply_locale(&mut cmd, locale);
    cmd.output().unwrap()
}

fn shadow_ok(root: &Path, args: &[&str]) -> Output {
    let out = shadow(root, "en_US.UTF-8", args);
    assert!(
        out.status.success(),
        "git-shadow {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(root.join(rel)).unwrap()
}

fn show_head(root: &Path, rel: &str) -> Option<String> {
    let out = git(root, &["show", &format!("HEAD:{rel}")]);
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

fn init_repo(root: &Path) {
    git_ok(root, &["init"]);
    git_ok(root, &["config", "user.name", "Test"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);
    // Deterministic default branch name across git versions.
    git_ok(root, &["checkout", "-B", "main"]);
}

fn clone(src: &Path, dst: &Path) {
    let out = Command::new("git")
        .args(["clone", src.to_str().unwrap(), dst.to_str().unwrap()])
        .env("PATH", prepend_path(&bin_dir()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    git_ok(dst, &["config", "user.name", "Test"]);
    git_ok(dst, &["config", "user.email", "test@example.com"]);
}

/// Build a source repo A with an overlay (with shadow changes), a phantom file,
/// and a nested phantom directory. Returns the archive path.
fn build_and_export(a: &Path, config_base: &str, out_dir: &Path) -> PathBuf {
    init_repo(a);
    write(a, "config.txt", config_base.as_bytes());
    write(a, "README.md", b"# project\n");
    git_ok(a, &["add", "-A"]);
    git_ok(a, &["commit", "-m", "init"]);

    shadow_ok(a, &["install"]);
    shadow_ok(a, &["add", "config.txt"]);

    // Phantom file.
    write(a, "notes.md", b"private notes\n");
    shadow_ok(a, &["add", "notes.md"]);

    // Phantom directory with a nested file.
    write(a, ".claude/settings.json", b"{\"key\": true}\n");
    write(a, ".claude/nested/deep.txt", b"deep\n");
    shadow_ok(a, &["add", ".claude"]);

    let archive = out_dir.join("export.tar.gz");
    shadow_ok(a, &["export", archive.to_str().unwrap()]);
    assert!(archive.exists());
    archive
}

#[test]
fn test_full_roundtrip_and_commit_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    let archive = build_and_export(&a, "port=8080\n", tmp.path());

    // Apply the local shadow edit to the overlay in A (after add captured baseline).
    write(&a, "config.txt", b"port=8080\ndebug=true\n");
    // Re-export so the archive carries the shadow content.
    shadow_ok(&a, &["export", "--force", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);

    // Import into the fresh clone.
    shadow_ok(&b, &["import", archive.to_str().unwrap()]);

    // Phantom file and dir restored byte-identical.
    assert_eq!(read(&b, "notes.md"), b"private notes\n");
    assert_eq!(read(&b, ".claude/settings.json"), b"{\"key\": true}\n");
    assert_eq!(read(&b, ".claude/nested/deep.txt"), b"deep\n");
    // Overlay working tree carries the shadow content.
    assert_eq!(read(&b, "config.txt"), b"port=8080\ndebug=true\n");

    // git-shadow status works.
    shadow_ok(&b, &["status"]);

    // Real commit cycle: commit an unrelated real change; shadow must not leak
    // and must be restored afterwards.
    write(&b, "real.txt", b"real\n");
    git_ok(&b, &["add", "real.txt"]);
    git_ok(&b, &["commit", "-m", "add real"]);

    // Overlay shadow did not leak into HEAD.
    assert_eq!(show_head(&b, "config.txt").as_deref(), Some("port=8080\n"));
    // Phantom files are not committed.
    assert!(show_head(&b, "notes.md").is_none());
    assert!(show_head(&b, ".claude/settings.json").is_none());
    // Shadow content restored in the working tree after the commit.
    assert_eq!(read(&b, "config.txt"), b"port=8080\ndebug=true\n");
    assert_eq!(read(&b, "notes.md"), b"private notes\n");
}

#[test]
fn test_upstream_moved_clean_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    let archive = build_and_export(&a, "l1\nl2\nl3\nl4\n", tmp.path());
    // Shadow changes the FIRST line.
    write(&a, "config.txt", b"l1 shadow\nl2\nl3\nl4\n");
    shadow_ok(&a, &["export", "--force", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);

    // Upstream (in B, before import) changes the LAST line — non-overlapping.
    write(&b, "config.txt", b"l1\nl2\nl3\nl4 upstream\n");
    git_ok(&b, &["commit", "-am", "upstream change"]);

    shadow_ok(&b, &["import", archive.to_str().unwrap()]);

    let merged = String::from_utf8(read(&b, "config.txt")).unwrap();
    assert!(merged.contains("l1 shadow"), "shadow change kept: {merged}");
    assert!(merged.contains("l4 upstream"), "upstream kept: {merged}");
    assert!(!merged.contains("<<<<<<<"), "no conflict markers: {merged}");

    // Commit cycle stays clean: HEAD keeps the upstream line, not the shadow line.
    write(&b, "real.txt", b"x\n");
    git_ok(&b, &["add", "real.txt"]);
    git_ok(&b, &["commit", "-m", "real"]);
    let head = show_head(&b, "config.txt").unwrap();
    assert!(head.contains("l4 upstream"));
    assert!(!head.contains("l1 shadow"), "shadow must not leak: {head}");
}

#[test]
fn test_upstream_moved_conflict_skips_then_force() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    let archive = build_and_export(&a, "l1\nl2\nl3\n", tmp.path());
    // Shadow changes line 1.
    write(&a, "config.txt", b"l1 shadow\nl2\nl3\n");
    shadow_ok(&a, &["export", "--force", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);

    // Upstream changes the SAME line 1 — overlapping -> conflict.
    write(&b, "config.txt", b"l1 upstream\nl2\nl3\n");
    git_ok(&b, &["commit", "-am", "upstream conflict"]);

    // Import skips the overlay (non-zero exit) but still imports the phantoms.
    let out = shadow(&b, "en_US.UTF-8", &["import", archive.to_str().unwrap()]);
    assert!(!out.status.success(), "conflict import must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("conflicts with upstream"),
        "expected conflict message, got: {stderr}"
    );
    // Other entries imported despite the conflict.
    assert_eq!(read(&b, "notes.md"), b"private notes\n");
    // Overlay left untouched (no markers).
    let wt = String::from_utf8(read(&b, "config.txt")).unwrap();
    assert!(!wt.contains("<<<<<<<"));

    // Re-run with --force: shadow wins, import succeeds.
    let forced = shadow(
        &b,
        "en_US.UTF-8",
        &["import", "--force", archive.to_str().unwrap()],
    );
    assert!(
        forced.status.success(),
        "forced import should succeed:\n{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert_eq!(read(&b, "config.txt"), b"l1 shadow\nl2\nl3\n");
}

#[test]
fn test_binary_phantom_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    init_repo(&a);
    write(&a, "README.md", b"# p\n");
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-m", "init"]);
    shadow_ok(&a, &["install"]);

    let binary: Vec<u8> = vec![0x00, 0x01, 0xff, 0xfe, b'x', 0x00, 0x7f];
    write(&a, "blob.bin", &binary);
    shadow_ok(&a, &["add", "blob.bin"]);

    let archive = tmp.path().join("export.tar.gz");
    shadow_ok(&a, &["export", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);
    shadow_ok(&b, &["import", archive.to_str().unwrap()]);

    assert_eq!(
        read(&b, "blob.bin"),
        binary,
        "binary phantom must round-trip"
    );
}

#[test]
fn test_nested_url_encoding_path_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    init_repo(&a);
    write(&a, "README.md", b"# p\n");
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-m", "init"]);
    shadow_ok(&a, &["install"]);

    // Directory name with a space and a percent sign; nested file with a slash.
    write(&a, "weird 100%dir/sub/data.txt", b"encoded path\n");
    shadow_ok(&a, &["add", "weird 100%dir"]);

    let archive = tmp.path().join("export.tar.gz");
    shadow_ok(&a, &["export", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);
    shadow_ok(&b, &["import", archive.to_str().unwrap()]);

    assert_eq!(
        read(&b, "weird 100%dir/sub/data.txt"),
        b"encoded path\n",
        "URL-encoding-relevant nested path must round-trip"
    );
}

#[test]
fn test_import_without_install_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    init_repo(&a);
    write(&a, "README.md", b"# p\n");
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-m", "init"]);
    shadow_ok(&a, &["install"]);
    write(&a, "notes.md", b"n\n");
    shadow_ok(&a, &["add", "notes.md"]);
    let archive = tmp.path().join("export.tar.gz");
    shadow_ok(&a, &["export", archive.to_str().unwrap()]);

    clone(&a, &b);
    // No install in B.
    let out = shadow(&b, "en_US.UTF-8", &["import", archive.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("git-shadow install"),
        "hint install: {stderr}"
    );
}

#[test]
fn test_export_nothing_managed_is_rejected_localized() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    write(root, "README.md", b"# p\n");
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-m", "init"]);
    shadow_ok(root, &["install"]);

    // English message.
    let en = shadow(root, "en_US.UTF-8", &["export"]);
    assert!(!en.status.success());
    assert!(String::from_utf8_lossy(&en.stderr).contains("nothing to export"));

    // Japanese message (hermetic locale).
    let ja = shadow(root, "ja_JP.UTF-8", &["export"]);
    assert!(!ja.status.success());
    assert!(String::from_utf8_lossy(&ja.stderr).contains("export 対象がありません"));
}

#[test]
fn test_export_refuses_with_stash_remnant() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    write(root, "config.txt", b"a\n");
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-m", "init"]);
    shadow_ok(root, &["install"]);
    shadow_ok(root, &["add", "config.txt"]);

    // Simulate an interrupted commit: leave a file in the stash dir.
    let stash = root.join(".git/shadow/stash");
    std::fs::create_dir_all(&stash).unwrap();
    std::fs::write(stash.join("config.txt"), b"leftover").unwrap();

    let archive = tmp.path().join("export.tar.gz");
    let out = shadow(root, "en_US.UTF-8", &["export", archive.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("stash"));
    assert!(!archive.exists());
}

#[test]
fn test_import_existing_differing_phantom_conflict_and_force() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    init_repo(&a);
    write(&a, "README.md", b"# p\n");
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-m", "init"]);
    shadow_ok(&a, &["install"]);
    write(&a, "notes.md", b"from A\n");
    shadow_ok(&a, &["add", "notes.md"]);
    let archive = tmp.path().join("export.tar.gz");
    shadow_ok(&a, &["export", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);
    // Pre-existing differing file at the phantom path.
    write(&b, "notes.md", b"already here\n");

    let out = shadow(&b, "en_US.UTF-8", &["import", archive.to_str().unwrap()]);
    assert!(!out.status.success(), "conflict must exit non-zero");
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(read(&b, "notes.md"), b"already here\n", "left untouched");

    // --force overwrites.
    let forced = shadow(
        &b,
        "en_US.UTF-8",
        &["import", "--force", archive.to_str().unwrap()],
    );
    assert!(forced.status.success());
    assert_eq!(read(&b, "notes.md"), b"from A\n");
}

#[test]
fn test_import_twice_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("A");
    let b = tmp.path().join("B");
    std::fs::create_dir_all(&a).unwrap();

    let archive = build_and_export(&a, "port=8080\n", tmp.path());
    write(&a, "config.txt", b"port=8080\ndebug=true\n");
    shadow_ok(&a, &["export", "--force", archive.to_str().unwrap()]);

    clone(&a, &b);
    shadow_ok(&b, &["install"]);

    shadow_ok(&b, &["import", archive.to_str().unwrap()]);
    // Second import: everything already matches -> exit 0, no changes.
    let second = shadow(&b, "en_US.UTF-8", &["import", archive.to_str().unwrap()]);
    assert!(
        second.status.success(),
        "second import should exit 0:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(read(&b, "config.txt"), b"port=8080\ndebug=true\n");
    assert_eq!(read(&b, "notes.md"), b"private notes\n");
    assert_eq!(read(&b, ".claude/settings.json"), b"{\"key\": true}\n");
}
