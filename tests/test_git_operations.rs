//! E2E integration tests for real git operations that interact with the
//! shadow hooks (pre-commit / post-commit / post-merge / post-rewrite).
//!
//! Unlike `test_commit_cycle.rs`, which drives the hook handlers in-process,
//! these tests install the real hooks via `git-shadow install` and run real
//! `git` commands (`commit`, `commit --amend`, `rebase`, `merge`,
//! `cherry-pick`). The installed hooks invoke `git-shadow` from `PATH`, so all
//! git commands that can fire a hook are run with the built binary's directory
//! prepended to `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Directory containing the built `git-shadow` binary under test.
fn bin_dir() -> PathBuf {
    assert_cmd::cargo::cargo_bin!("git-shadow")
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build a `PATH` value with `dir` prepended so the installed hooks resolve the
/// binary under test rather than any system-installed `git-shadow`.
fn prepend_path(dir: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Run a raw `git` command with the shadow binary on `PATH` (so any hook fires
/// correctly). Returns the raw `Output` without asserting success.
fn git(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("PATH", prepend_path(&bin_dir()))
        .env("LANG", "en_US.UTF-8")
        .output()
        .unwrap()
}

/// Run a `git` command and assert it succeeded.
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

/// Run a `git-shadow` subcommand and assert it succeeded.
fn shadow_ok(root: &Path, args: &[&str]) -> Output {
    let out = Command::new("git-shadow")
        .args(args)
        .current_dir(root)
        .env("PATH", prepend_path(&bin_dir()))
        .env("LANG", "en_US.UTF-8")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git-shadow {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Run a `git-shadow` subcommand and return the raw `Output`.
fn shadow(root: &Path, args: &[&str]) -> Output {
    Command::new("git-shadow")
        .args(args)
        .current_dir(root)
        .env("PATH", prepend_path(&bin_dir()))
        .env("LANG", "en_US.UTF-8")
        .output()
        .unwrap()
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

/// `git show HEAD:<rel>` -> committed content (None if not present in HEAD).
fn show_head(root: &Path, rel: &str) -> Option<String> {
    let out = git(root, &["show", &format!("HEAD:{rel}")]);
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

fn head_hash(root: &Path) -> String {
    String::from_utf8_lossy(&git_ok(root, &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string()
}

/// Number of regular files sitting in `.git/shadow/stash/`.
fn stash_file_count(root: &Path) -> usize {
    let stash = root.join(".git/shadow/stash");
    match std::fs::read_dir(&stash) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .count(),
        Err(_) => 0,
    }
}

/// Read the overlay baseline file for `rel` under `.git/shadow/baselines/`.
/// `rel` here has no `/`, so no path encoding is needed.
fn baseline(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(".git/shadow/baselines").join(rel)).unwrap()
}

const BASE: &str = "top\nmid\nbot\n";
const SHADOW: &str = "top\nmid\nbot\nSHADOW_LOCAL\n";

/// Create a repo with `app.txt` committed at [`BASE`], shadow installed, an
/// overlay registered on `app.txt`, and an unstaged shadow edit ([`SHADOW`]) in
/// the working tree. Returns the temp dir; its `path()` is the repo root.
fn setup_overlay_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    git_ok(root, &["init"]);
    git_ok(root, &["config", "user.name", "Test User"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);

    write(root, "app.txt", BASE);
    git_ok(root, &["add", "app.txt"]);
    git_ok(root, &["commit", "-m", "init"]);
    // Normalize the branch name regardless of the host git default.
    git_ok(root, &["branch", "-M", "main"]);

    shadow_ok(root, &["install"]);
    shadow_ok(root, &["add", "app.txt"]);

    // Introduce the local-only (shadow) change, unstaged.
    write(root, "app.txt", SHADOW);

    dir
}

// ---------------------------------------------------------------------------
// a) git commit --amend
// ---------------------------------------------------------------------------

#[test]
fn test_amend_keeps_shadow_out_of_commit_and_intact_in_worktree() {
    let dir = setup_overlay_repo();
    let root = dir.path();

    // A normal commit of an unrelated file: shadow is stashed then restored.
    write(root, "feature.txt", "f1\n");
    git_ok(root, &["add", "feature.txt"]);
    git_ok(root, &["commit", "-m", "add feature"]);

    // Sanity: the first commit has the baseline, not the shadow, and the
    // worktree still carries the shadow change.
    assert_eq!(show_head(root, "app.txt").as_deref(), Some(BASE));
    assert_eq!(read(root, "app.txt"), SHADOW);
    assert_eq!(
        stash_file_count(root),
        0,
        "stash must be clean after commit"
    );

    // Amend: change the message and add a new staged file.
    write(root, "extra.txt", "e1\n");
    git_ok(root, &["add", "extra.txt"]);
    git_ok(root, &["commit", "--amend", "-m", "add feature (amended)"]);

    // The amended commit must not contain shadow content and must include the
    // newly staged file.
    assert_eq!(
        show_head(root, "app.txt").as_deref(),
        Some(BASE),
        "amended commit must contain baseline, not shadow"
    );
    assert_eq!(show_head(root, "extra.txt").as_deref(), Some("e1\n"));
    assert_eq!(show_head(root, "feature.txt").as_deref(), Some("f1\n"));

    // Worktree shadow content survives the amend.
    assert_eq!(read(root, "app.txt"), SHADOW);
    assert_eq!(stash_file_count(root), 0, "stash must be clean after amend");

    // Document current baseline-consistency behavior. `commit --amend` fires
    // post-rewrite, which auto-rebases the overlay baseline_commit onto the new
    // (amended) HEAD. Because the committed content of app.txt is unchanged
    // (still the baseline), the auto-rebase succeeds and doctor reports no
    // issues.
    let doctor = shadow(root, &["doctor"]);
    assert!(
        doctor.status.success(),
        "doctor should report no issues after amend; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
}

// ---------------------------------------------------------------------------
// b) real git rebase -> post-rewrite auto-rebases the baseline
// ---------------------------------------------------------------------------

#[test]
fn test_rebase_triggers_post_rewrite_and_preserves_shadow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    git_ok(root, &["init"]);
    git_ok(root, &["config", "user.name", "Test User"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);

    write(root, "app.txt", BASE);
    git_ok(root, &["add", "app.txt"]);
    git_ok(root, &["commit", "-m", "init"]);
    git_ok(root, &["branch", "-M", "main"]);

    shadow_ok(root, &["install"]);

    // feature: a commit that does NOT touch app.txt.
    git_ok(root, &["checkout", "-b", "feature"]);
    write(root, "feature.txt", "f1\n");
    git_ok(root, &["add", "feature.txt"]);
    git_ok(root, &["commit", "-m", "feat work"]);

    // main advances with a change to app.txt (upstream change to overlay file).
    git_ok(root, &["checkout", "main"]);
    write(root, "app.txt", "TOP_UPSTREAM\nmid\nbot\n");
    git_ok(root, &["add", "app.txt"]);
    git_ok(root, &["commit", "-m", "upstream change to app.txt"]);

    // Register the overlay on feature and add a shadow edit at the end of the
    // file (a region disjoint from the upstream change so autostash pops cleanly).
    git_ok(root, &["checkout", "feature"]);
    shadow_ok(root, &["add", "app.txt"]);
    write(root, "app.txt", SHADOW);

    // Rebase feature onto main. The overlay makes the worktree dirty, so use
    // --autostash (the realistic way to rebase with active shadow changes).
    let out = git(root, &["rebase", "--autostash", "main"]);
    assert!(
        out.status.success(),
        "rebase --autostash should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // post-rewrite ran: the baseline was auto-rebased to include the upstream
    // change to app.txt.
    assert!(
        baseline(root, "app.txt").contains("TOP_UPSTREAM"),
        "baseline should have been auto-rebased to the upstream content; got: {:?}",
        baseline(root, "app.txt")
    );

    // Shadow change survived in the worktree, layered on top of the new base.
    let wt = read(root, "app.txt");
    assert!(
        wt.contains("TOP_UPSTREAM"),
        "worktree should have upstream change: {wt:?}"
    );
    assert!(
        wt.contains("SHADOW_LOCAL"),
        "worktree should still carry shadow: {wt:?}"
    );

    assert_eq!(stash_file_count(root), 0, "shadow stash must be clean");
}

// ---------------------------------------------------------------------------
// c) real git merge -> post-merge auto-rebases the baseline (clean case)
// ---------------------------------------------------------------------------

#[test]
fn test_merge_triggers_post_merge_and_preserves_shadow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    git_ok(root, &["init"]);
    git_ok(root, &["config", "user.name", "Test User"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);

    write(root, "app.txt", BASE);
    write(root, "side.txt", "s0\n");
    git_ok(root, &["add", "app.txt", "side.txt"]);
    git_ok(root, &["commit", "-m", "init"]);
    git_ok(root, &["branch", "-M", "main"]);

    shadow_ok(root, &["install"]);

    // other: change app.txt (the overlay's underlying file).
    git_ok(root, &["checkout", "-b", "other"]);
    write(root, "app.txt", "TOP_MERGE\nmid\nbot\n");
    git_ok(root, &["add", "app.txt"]);
    git_ok(root, &["commit", "-m", "other changes app.txt"]);

    // main diverges by touching a different file, forcing a real merge commit.
    git_ok(root, &["checkout", "main"]);
    write(root, "side.txt", "s1\n");
    git_ok(root, &["add", "side.txt"]);
    git_ok(root, &["commit", "-m", "main changes side.txt"]);

    // Register the overlay and add a shadow edit at the file's end.
    shadow_ok(root, &["add", "app.txt"]);
    write(root, "app.txt", SHADOW);

    // Merge `other`. The overlay makes the worktree dirty, so enable
    // merge.autoStash (the realistic way to merge with active shadow changes).
    git_ok(root, &["config", "merge.autoStash", "true"]);
    let out = git(root, &["merge", "--no-edit", "other"]);
    assert!(
        out.status.success(),
        "merge should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // post-merge ran: baseline auto-rebased to the merged app.txt content.
    assert!(
        baseline(root, "app.txt").contains("TOP_MERGE"),
        "baseline should have been auto-rebased to the merged content; got: {:?}",
        baseline(root, "app.txt")
    );

    // Shadow change survived on top of the merged base.
    let wt = read(root, "app.txt");
    assert!(
        wt.contains("TOP_MERGE"),
        "worktree should have merged change: {wt:?}"
    );
    assert!(
        wt.contains("SHADOW_LOCAL"),
        "worktree should still carry shadow: {wt:?}"
    );

    assert_eq!(stash_file_count(root), 0, "shadow stash must be clean");
}

// ---------------------------------------------------------------------------
// d) git cherry-pick
// ---------------------------------------------------------------------------

#[test]
fn test_cherry_pick_does_not_leak_shadow() {
    let dir = setup_overlay_repo();
    let root = dir.path();

    // Create a commit on a side branch that adds a new file only.
    git_ok(root, &["checkout", "-b", "donor"]);
    // Reset the working tree to baseline first is unnecessary: the shadow edit
    // is unstaged and does not block a new-file commit on the donor branch.
    write(root, "payload.txt", "payload\n");
    git_ok(root, &["add", "payload.txt"]);
    git_ok(root, &["commit", "-m", "add payload"]);
    let donor = head_hash(root);

    // Back on main, remove the payload from the worktree state that the donor
    // branch introduced (checkout back to main drops payload.txt).
    git_ok(root, &["checkout", "main"]);
    // Re-assert the shadow edit (branch checkout preserves it as it is unstaged,
    // but be explicit for clarity).
    write(root, "app.txt", SHADOW);

    let out = git(root, &["cherry-pick", &donor]);
    assert!(
        out.status.success(),
        "cherry-pick should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The picked commit brought in payload.txt but must not contain shadow
    // content for app.txt.
    assert_eq!(show_head(root, "payload.txt").as_deref(), Some("payload\n"));
    assert_eq!(
        show_head(root, "app.txt").as_deref(),
        Some(BASE),
        "cherry-picked commit must not contain shadow content for app.txt"
    );

    // Worktree shadow content is intact.
    assert_eq!(read(root, "app.txt"), SHADOW);
    assert_eq!(stash_file_count(root), 0, "shadow stash must be clean");
}

// ---------------------------------------------------------------------------
// e) partial commit with pathspec: git commit -- <other-file>
// ---------------------------------------------------------------------------

#[test]
fn test_pathspec_commit_isolates_shadow() {
    let dir = setup_overlay_repo();
    let root = dir.path();

    // A second tracked file with an unstaged change.
    write(root, "b.txt", "b1\n");
    git_ok(root, &["add", "b.txt"]);
    git_ok(root, &["commit", "-m", "add b"]);
    // Shadow edit on app.txt survives the commit; re-assert for clarity.
    write(root, "app.txt", SHADOW);
    write(root, "b.txt", "b1\nb2\n");

    // Commit only b.txt via pathspec. This uses a temporary index
    // (GIT_INDEX_FILE) that the pre-commit hook must cope with.
    let out = git(root, &["commit", "-m", "update b only", "--", "b.txt"]);
    assert!(
        out.status.success(),
        "pathspec commit should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The commit contains only b.txt's change.
    assert_eq!(show_head(root, "b.txt").as_deref(), Some("b1\nb2\n"));
    // No shadow content of app.txt leaked into the commit.
    assert_eq!(
        show_head(root, "app.txt").as_deref(),
        Some(BASE),
        "pathspec commit must not contain shadow content for app.txt"
    );

    // app.txt shadow content is restored in the worktree.
    assert_eq!(
        read(root, "app.txt"),
        SHADOW,
        "app.txt shadow content must be restored after pathspec commit"
    );
    assert_eq!(stash_file_count(root), 0, "shadow stash must be clean");
}

// ---------------------------------------------------------------------------
// f) concurrent commit lock contention
// ---------------------------------------------------------------------------

#[test]
fn test_commit_blocked_by_live_lock_holder() {
    let dir = setup_overlay_repo();
    let root = dir.path();

    // Something to commit so the commit is not rejected for being empty.
    write(root, "c.txt", "c1\n");
    git_ok(root, &["add", "c.txt"]);

    // Simulate a concurrent git-shadow process holding the lock: write a lock
    // file pointing at a live child process.
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let lock_path = root.join(".git/shadow/lock");
    std::fs::write(
        &lock_path,
        format!("pid={}\ntimestamp=2026-01-01T00:00:00+00:00", child.id()),
    )
    .unwrap();

    let out = git(root, &["commit", "-m", "should be blocked"]);
    let blocked = !out.status.success();

    // Clean up the helper process before asserting.
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        blocked,
        "commit must be blocked while the lock is held by a live process;\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // HEAD unchanged (the blocked commit was not created): still "init".
    let subject = String::from_utf8_lossy(&git_ok(root, &["log", "-1", "--format=%s"]).stdout)
        .trim()
        .to_string();
    assert_eq!(subject, "init", "no new commit should have been created");

    // Worktree shadow intact and no orphaned stash entry.
    assert_eq!(read(root, "app.txt"), SHADOW, "shadow must be intact");
    assert_eq!(
        stash_file_count(root),
        0,
        "no stash entry should be orphaned by the blocked commit"
    );
}
