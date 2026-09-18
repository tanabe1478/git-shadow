//! E2E tests for the experimental clean/smudge filter backend.
//!
//! Each test builds a real repository, runs `git-shadow filter install` (which
//! registers the `shadow` filter driver in `.git/config` and a managed section
//! in `.git/info/attributes`), then drives real `git` commands. The filter
//! commands (`filter clean`/`filter smudge`) are invoked by Git via the
//! absolute binary path baked at install time, so no PATH manipulation is
//! needed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin!("git-shadow").to_path_buf()
}

fn git(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("LANG", "en_US.UTF-8")
        .env("GIT_EDITOR", "true")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap()
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

/// Run `git-shadow` with the given args.
fn shadow(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}

fn shadow_ok(root: &Path, args: &[&str]) -> Output {
    let out = shadow(root, args);
    assert!(
        out.status.success(),
        "git-shadow {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Pipe `input` through `git-shadow filter <clean|smudge> <file>`.
fn run_filter(root: &Path, mode: &str, file: &str, input: &[u8]) -> (Vec<u8>, bool) {
    use std::io::Write;
    let mut child = Command::new(bin())
        .args(["filter", mode, file])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.stdout, out.status.success())
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    String::from_utf8(std::fs::read(root.join(rel)).unwrap()).unwrap()
}

fn show_head(root: &Path, rel: &str) -> String {
    let out = git_ok(root, &["show", &format!("HEAD:{rel}")]);
    String::from_utf8(out.stdout).unwrap()
}

/// A repository with the filter backend installed and `file` committed.
fn repo_with_file(file: &str, content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q"]);
    git_ok(root, &["config", "user.name", "T"]);
    git_ok(root, &["config", "user.email", "t@t"]);
    shadow_ok(root, &["filter", "install"]);
    write(root, file, content);
    git_ok(root, &["add", file]);
    git_ok(root, &["commit", "-qm", "base"]);
    dir
}

/// Register `file`'s current worktree content as the overlay.
fn filter_add(root: &Path, file: &str) {
    shadow_ok(root, &["filter", "add", file]);
}

// ---------------------------------------------------------------------------
// basics
// ---------------------------------------------------------------------------

#[test]
fn test_no_overlay_behaves_like_plain_git() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb\nc\nTEAM\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "t"]);
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\nTEAM\n");
}

#[test]
fn test_overlay_only_change_never_committed() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // Status must be clean: clean(L) == B == index.
    let out = git_ok(root, &["status", "--short"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "overlay should be invisible to git status: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    git_ok(root, &["add", "f.txt"]);
    // Staging the file stages clean(L) == B: nothing is actually staged.
    let staged = git_ok(root, &["diff", "--cached"]);
    assert!(staged.stdout.is_empty());
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\n");
    assert_eq!(read(root, "f.txt"), "a\nb-local\nc\n");
}

#[test]
fn test_team_edit_only_commits_normally() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    write(root, "f.txt", "a\nb-local\nc\nd-team\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "team"]);
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\nd-team\n");
    assert_eq!(read(root, "f.txt"), "a\nb-local\nc\nd-team\n");
}

#[test]
fn test_overlay_plus_team_edit_same_file() {
    // The headline scenario: baseline A/B/C, overlay A/B-local/C, then the
    // user appends D-team. Commit must store A/B/C/D-team; the worktree keeps
    // A/B-local/C/D-team.
    let dir = repo_with_file("f.txt", "A\nB\nC\n");
    let root = dir.path();
    write(root, "f.txt", "A\nB-local\nC\n");
    filter_add(root, "f.txt");

    write(root, "f.txt", "A\nB-local\nC\nD-team\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "mixed"]);

    assert_eq!(show_head(root, "f.txt"), "A\nB\nC\nD-team\n");
    assert_eq!(read(root, "f.txt"), "A\nB-local\nC\nD-team\n");
}

#[test]
fn test_multiple_overlay_files() {
    let dir = repo_with_file("f1.txt", "x1\n");
    let root = dir.path();
    write(root, "f2.txt", "y1\n");
    git_ok(root, &["add", "f2.txt"]);
    git_ok(root, &["commit", "-qm", "f2"]);

    write(root, "f1.txt", "x1\nlocal-a\n");
    write(root, "f2.txt", "y1\nlocal-b\n");
    filter_add(root, "f1.txt");
    filter_add(root, "f2.txt");

    write(root, "f1.txt", "x1\nlocal-a\nteam-a\n");
    write(root, "f2.txt", "y1\nlocal-b\nteam-b\n");
    git_ok(root, &["add", "f1.txt", "f2.txt"]);
    git_ok(root, &["commit", "-qm", "both"]);

    assert_eq!(show_head(root, "f1.txt"), "x1\nteam-a\n");
    assert_eq!(show_head(root, "f2.txt"), "y1\nteam-b\n");
    assert_eq!(read(root, "f1.txt"), "x1\nlocal-a\nteam-a\n");
    assert_eq!(read(root, "f2.txt"), "y1\nlocal-b\nteam-b\n");
}

#[test]
fn test_git_add_all_variants() {
    let dir = repo_with_file("f.txt", "base\n");
    let root = dir.path();
    write(root, "f.txt", "base\noverlay\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "base\noverlay\nteam\n");

    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-qm", "add-all"]);
    assert_eq!(show_head(root, "f.txt"), "base\nteam\n");

    write(root, "f.txt", "base\noverlay\nteam\nteam2\n");
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-qm", "add-dot"]);
    assert_eq!(show_head(root, "f.txt"), "base\nteam\nteam2\n");
}

#[test]
fn test_commit_amend() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "f.txt", "a\noverlay\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\noverlay\nt\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "c1"]);
    write(root, "f.txt", "a\noverlay\nt\nt2\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-q", "--amend", "--no-edit"]);
    assert_eq!(show_head(root, "f.txt"), "a\nt\nt2\n");
    assert_eq!(read(root, "f.txt"), "a\noverlay\nt\nt2\n");
}

// ---------------------------------------------------------------------------
// partial staging
// ---------------------------------------------------------------------------

#[test]
fn test_add_p_stages_team_hunks_only() {
    // 20-line file so the two team edits form separate hunks.
    let base = (1..=20).map(|i| format!("l{i:02}\n")).collect::<String>();
    let dir = repo_with_file("f.txt", &base);
    let root = dir.path();
    write(root, "f.txt", &base.replacen("l02\n", "LOCAL-OVERLAY\n", 1));
    filter_add(root, "f.txt");

    let edited = base
        .replacen("l02\n", "LOCAL-OVERLAY\n", 1)
        .replacen("l04\n", "top-team\n", 1)
        .replacen("l17\n", "bottom-team\n", 1);
    write(root, "f.txt", &edited);

    // git add -p presents hunks in the clean domain (overlay invisible).
    // Stage only the first.
    let output = Command::new("git")
        .args(["add", "-p", "f.txt"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("LANG", "en_US.UTF-8")
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(b"y\nn\nq\n")?;
            c.wait_with_output()
        })
        .unwrap();
    assert!(
        output.status.success(),
        "add -p failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let staged = git_ok(root, &["diff", "--cached"]);
    let staged = String::from_utf8_lossy(&staged.stdout);
    assert!(staged.contains("+top-team"), "staged diff: {staged}");
    assert!(
        !staged.contains("bottom-team"),
        "second hunk should be unstaged: {staged}"
    );
    assert!(
        !staged.contains("LOCAL-OVERLAY") && !staged.contains("-l02"),
        "overlay must not appear in the staged diff: {staged}"
    );

    git_ok(root, &["commit", "-qm", "partial"]);
    assert_eq!(
        show_head(root, "f.txt"),
        base.replacen("l04\n", "top-team\n", 1)
    );
}

#[test]
fn test_partial_stage_then_commit_rest() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nLOCAL\nb\nc\n");
    filter_add(root, "f.txt");

    write(root, "f.txt", "a\nLOCAL\nb\nc\nt\n");
    // stage whole thing the normal way for the first commit
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "first"]);
    write(root, "f.txt", "a\nLOCAL\nb\nc\nt\nt2\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "second"]);
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\nt\nt2\n");
}

// ---------------------------------------------------------------------------
// commit variants
// ---------------------------------------------------------------------------

#[test]
fn test_commit_a_strips_overlay() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "f.txt", "a\nlocal\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nlocal\nt\n");
    git_ok(root, &["commit", "-qam", "via -a"]);
    assert_eq!(show_head(root, "f.txt"), "a\nt\n");
    assert_eq!(read(root, "f.txt"), "a\nlocal\nt\n");
}

#[test]
fn test_commit_no_verify_cannot_leak_overlay() {
    // The headline `--no-verify` case: with hooks, skipping pre-commit leaks
    // the overlay. With the filter backend, stripping happens at `git add`
    // time — `--no-verify` only skips hooks, the index already holds B+T.
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "f.txt", "a\nlocal\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nlocal\nt\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(
        root,
        &["commit", "-qm", "--no-verify is safe now", "--no-verify"],
    );
    assert_eq!(show_head(root, "f.txt"), "a\nt\n");

    // Even `commit -a --no-verify` (add+commit in one shot) stays clean.
    write(root, "f.txt", "a\nlocal\nt\nt2\n");
    git_ok(root, &["commit", "-qam", "t2", "--no-verify"]);
    assert_eq!(show_head(root, "f.txt"), "a\nt\nt2\n");
}

#[test]
fn test_commit_pathspec_strips_overlay() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "f.txt", "a\nlocal\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nlocal\nt\n");
    git_ok(root, &["commit", "-qm", "pathspec", "f.txt"]);
    assert_eq!(show_head(root, "f.txt"), "a\nt\n");
}

// ---------------------------------------------------------------------------
// checkout / switch — suspend/resume obsoleted
// ---------------------------------------------------------------------------

#[test]
fn test_branch_switch_same_file_content() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    git_ok(root, &["switch", "-qc", "side"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
    git_ok(root, &["switch", "-q", "master"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
    // nothing diverged: status clean, no suspend/resume needed
    let out = git_ok(root, &["status", "--short"]);
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    assert_eq!(show_head(root, "f.txt"), "a\nb\n");
}

#[test]
fn test_branch_switch_file_changed_across_branches() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    git_ok(root, &["switch", "-qc", "side"]);
    write(root, "f.txt", "a\nb-local\nc-side\n"); // overlay + team edit
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "side"]); // commits a/b/c-side

    git_ok(root, &["switch", "-q", "master"]);
    // smudge re-applies overlay on master's content: a/b-local/c
    assert_eq!(read(root, "f.txt"), "a\nb-local\nc\n");
}

#[test]
fn test_branch_where_file_absent() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    git_ok(root, &["switch", "-qc", "nofile"]);
    git_ok(root, &["rm", "-q", "f.txt"]);
    git_ok(root, &["commit", "-qm", "del"]);
    assert!(!root.join("f.txt").exists());

    git_ok(root, &["switch", "-q", "master"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
}

#[test]
fn test_checkout_discards_team_edit_but_keeps_overlay() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nb-local\nteam\n");

    git_ok(root, &["checkout", "--", "f.txt"]);
    // index holds B+T? No: T was never staged, so index = B; smudge(B) = L.
    // Wait: index content is still B (never staged). So worktree = L = overlay only.
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
}

// ---------------------------------------------------------------------------
// upstream changes / merge / rebase
// ---------------------------------------------------------------------------

/// Commit `side_content` for `file` on a new branch `side`, created inside a
/// sibling worktree — this is how a real "upstream" change reaches this repo
/// (another worktree/clone where the file isn't overlay-managed). Writing
/// plain upstream content directly in the managed worktree would trip the
/// overlay-region check in clean (covered by
/// test_overlay_line_edit_conflicts).
fn diverge(root: &Path, file: &str, side_content: &str) {
    let wt = root.join("upstream-wt");
    git_ok(
        root,
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    write(&wt, file, side_content);
    git_ok(&wt, &["add", file]);
    git_ok(&wt, &["commit", "-qm", "side"]);
    git_ok(
        root,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
}

#[test]
fn test_merge_upstream_disjoint_lines() {
    let dir = repo_with_file("f.txt", "a\nb\nc\nd\ne\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\nd\ne\n"); // overlay on line 2
    filter_add(root, "f.txt");

    diverge(root, "f.txt", "a-up\nb\nc\nd\ne\n"); // upstream edits line 1
    git_ok(root, &["merge", "--no-edit", "side"]);

    assert_eq!(read(root, "f.txt"), "a-up\nb-local\nc\nd\ne\n");
    // The merge commit itself (already created by git merge) stores clean
    // content: upstream + no overlay.
    assert_eq!(show_head(root, "f.txt"), "a-up\nb\nc\nd\ne\n");
}

#[test]
fn test_merge_upstream_adjacent_line_conflict() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n"); // overlay on line 2
    filter_add(root, "f.txt");

    diverge(root, "f.txt", "a\nb\nside-on-line3\n"); // upstream edits adjacent line 3
    let out = git(root, &["merge", "--no-edit", "side"]);
    // Whether git-merge succeeds or not, worktree must not silently corrupt.
    let content = read(root, "f.txt");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("conflict") || content.contains("<<<<<<<") {
        // conflict markers surfaced in worktree — acceptable, must be visible
        assert!(content.contains("b-local") || content.contains("side-on-line3"));
    } else {
        // clean merge keeps overlay + upstream change
        assert!(content.contains("b-local"));
        assert!(content.contains("side-on-line3"));
    }
}

#[test]
fn test_merge_upstream_same_line_conflict() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    diverge(root, "f.txt", "a\nb-upstream\nc\n"); // upstream edits the overlay line
    let out = git(root, &["merge", "--no-edit", "side"]);
    // The clean-domain merge sees b vs b-upstream on the same line -> conflict.
    assert!(!out.status.success() || read(root, "f.txt").contains("<<<<<<<"));
    let content = read(root, "f.txt");
    assert!(content.contains("<<<<<<<"), "expected markers: {content}");

    // While conflicted, the file is failed-closed: add refuses.
    assert!(!git(root, &["add", "f.txt"]).status.success());

    // Resolve keeping the overlay line; `filter resolve` re-bases the pair.
    write(root, "f.txt", "a\nb-local\nc-resolved\n");
    shadow_ok(root, &["filter", "resolve", "f.txt"]);
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "resolved", "--no-edit"]);
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc-resolved\n");
    assert_eq!(read(root, "f.txt"), "a\nb-local\nc-resolved\n");
}

#[test]
fn test_rebase_preserves_overlay() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // Upstream moves on `side` (committed from a sibling worktree); we rebase
    // our topic branch onto it. Non-adjacent change so the replay merges.
    diverge(root, "f.txt", "a-side\nb\nc\n");
    git_ok(root, &["switch", "-qc", "topic", "master"]);
    write(root, "f.txt", "a\nb-local\nc\ntopic\n"); // edit on top of overlay
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "topic"]); // stores a/b/c/topic
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\ntopic\n");

    git_ok(root, &["rebase", "-q", "side"]);
    // Replayed topic onto side: clean domain = a-side/b/c/topic.
    assert_eq!(show_head(root, "f.txt"), "a-side\nb\nc\ntopic\n");
    assert_eq!(read(root, "f.txt"), "a-side\nb-local\nc\ntopic\n");
}

#[test]
fn test_rebase_conflict_resolve_continue() {
    // Rebase hitting a content conflict on the managed file: git writes
    // markers, smudge marks the file conflicted, resolution + `filter
    // resolve` + `git add` + `rebase --continue` completes it.
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    diverge(root, "f.txt", "a\nb\nc-side\n"); // changes the last line
    git_ok(root, &["switch", "-qc", "topic", "master"]);
    write(root, "f.txt", "a\nb-local\nc\ntopic\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "topic"]);

    let out = git(root, &["rebase", "side"]);
    assert!(!out.status.success(), "rebase should stop on conflict");
    assert!(read(root, "f.txt").contains("<<<<<<<"));

    // While stopped, staging is refused (UU or conflicted flag).
    assert!(!git(root, &["add", "f.txt"]).status.success());

    // Resolve keeping the overlay line; replay c-side's intent by hand.
    write(root, "f.txt", "a\nb-local\nc-side\ntopic\n");
    shadow_ok(root, &["filter", "resolve", "f.txt"]);
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["rebase", "--continue"]);

    assert_eq!(show_head(root, "f.txt"), "a\nb\nc-side\ntopic\n");
    assert_eq!(read(root, "f.txt"), "a\nb-local\nc-side\ntopic\n");
}

// ---------------------------------------------------------------------------
// reset / restore / stash
// ---------------------------------------------------------------------------

#[test]
fn test_reset_hard_keeps_overlay_drops_unstaged() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nb-local\nteam\n");

    git_ok(root, &["reset", "-q", "--hard"]);
    // index=HEAD=B; smudge(B)=L — overlay restored, unstaged team edit dropped.
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
}

#[test]
fn test_stash_pop_preserves_overlay() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nb-local\nteam\n");

    git_ok(root, &["stash", "push", "-qm", "wip"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\n"); // stash restores to L
                                                     // stash entry stored cleaned content — overlay stripped, team edit kept
    let out = git_ok(root, &["show", "stash@{0}:f.txt"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a\nb\nteam\n");
    git_ok(root, &["stash", "pop", "-q"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\nteam\n");
}

// ---------------------------------------------------------------------------
// worktree
// ---------------------------------------------------------------------------

#[test]
fn test_worktree_gets_plain_content_until_registered() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    let wt = dir.path().join("wt2");
    git_ok(
        root,
        &["worktree", "add", "-b", "wtb", wt.to_str().unwrap()],
    );
    // attributes are shared (common dir) but wt2 has no state -> passthrough
    assert_eq!(read(&wt, "f.txt"), "a\nb\n");

    // wt2 edits are plain team edits
    write(&wt, "f.txt", "a\nb\nwt-team\n");
    git_ok(&wt, &["add", "f.txt"]);
    git_ok(&wt, &["commit", "-qm", "wt"]);
    assert_eq!(
        String::from_utf8_lossy(&git_ok(&wt, &["show", "HEAD:f.txt"]).stdout),
        "a\nb\nwt-team\n"
    );
    // main worktree's checkout of the same blob is unaffected until the blob
    // itself changes: merge wtb so HEAD's f.txt moves, then the file update
    // runs smudge -> overlay re-applied on top of the new repo content.
    assert_eq!(read(root, "f.txt"), "a\nb-local\n");
    git_ok(root, &["merge", "-q", "--no-edit", "wtb"]);
    assert_eq!(read(root, "f.txt"), "a\nb-local\nwt-team\n");
    assert_eq!(show_head(root, "f.txt"), "a\nb\nwt-team\n");
}

#[test]
fn test_worktree_independent_overlays() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-main\n");
    filter_add(root, "f.txt");

    let wt = dir.path().join("wt2");
    git_ok(
        root,
        &["worktree", "add", "-b", "wtb", wt.to_str().unwrap()],
    );
    write(&wt, "f.txt", "a\nb-wt2\n");
    shadow_ok(&wt, &["filter", "install"]); // idempotent: same shared config
    shadow_ok(&wt, &["filter", "add", "f.txt"]);

    assert_eq!(read(root, "f.txt"), "a\nb-main\n");
    assert_eq!(read(&wt, "f.txt"), "a\nb-wt2\n");

    // commits in each worktree strip their own overlay
    write(&wt, "f.txt", "a\nb-wt2\nteam\n");
    git_ok(&wt, &["add", "f.txt"]);
    git_ok(&wt, &["commit", "-qm", "wt"]);
    assert_eq!(
        String::from_utf8_lossy(&git_ok(&wt, &["show", "HEAD:f.txt"]).stdout),
        "a\nb\nteam\n"
    );
    assert_eq!(read(&wt, "f.txt"), "a\nb-wt2\nteam\n");
    // main worktree overlay untouched
    assert_eq!(read(root, "f.txt"), "a\nb-main\n");
}

// ---------------------------------------------------------------------------
// info/attributes placement (explicitly required verification)
// ---------------------------------------------------------------------------

#[test]
fn test_attributes_live_in_common_dir_not_worktree() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "f.txt", "a\nlocal\n");
    filter_add(root, "f.txt");

    let wt = dir.path().join("wt2");
    git_ok(
        root,
        &["worktree", "add", "-b", "wtb", wt.to_str().unwrap()],
    );

    let wt_git_file = wt.join(".git");
    let wt_gitdir = String::from_utf8_lossy(&std::fs::read(&wt_git_file).unwrap())
        .trim()
        .strip_prefix("gitdir: ")
        .unwrap()
        .to_string();

    // The worktree gitdir has no info/attributes of its own...
    assert!(
        !PathBuf::from(&wt_gitdir).join("info/attributes").exists(),
        "git-shadow does not create per-worktree info/attributes"
    );
    // ...and even if it existed, Git reads info/attributes from the COMMON dir.
    std::fs::create_dir_all(PathBuf::from(&wt_gitdir).join("info")).unwrap();
    std::fs::write(
        PathBuf::from(&wt_gitdir).join("info/attributes"),
        "f.txt filter=nonexistent\n",
    )
    .unwrap();
    // Re-checkout in wt2: smudge still resolves via common attributes.
    std::fs::remove_file(wt.join("f.txt")).unwrap();
    git_ok(&wt, &["checkout", "--", "f.txt"]);
    // content is whatever the shared (shadow) filter produced — plain blob here
    assert_eq!(read(&wt, "f.txt"), "a\n");
}

// ---------------------------------------------------------------------------
// safety / failure modes
// ---------------------------------------------------------------------------

#[test]
fn test_clean_refuses_while_conflicted() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // Same-line upstream change -> git-level conflict; smudge sees the
    // marker-laden merge product and marks the file conflicted.
    diverge(root, "f.txt", "a\nb-upstream\nc\n");
    git(root, &["merge", "--no-edit", "side"]);
    assert!(read(root, "f.txt").contains("<<<<<<<"));

    // While conflicted, `git add` must fail rather than guess.
    let out = git(root, &["add", "f.txt"]);
    assert!(
        !out.status.success(),
        "conflicted file must refuse staging (filter fail-closed or UU)"
    );
}

#[test]
fn test_corrupt_state_fails_closed() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    // Corrupt the registry: clean can no longer prove W is overlay-free.
    let cfg = root.join(".git/shadow-filter/config.json");
    std::fs::write(&cfg, "{ not json !!!").unwrap();

    let out = git(root, &["add", "f.txt"]);
    assert!(
        !out.status.success(),
        "corrupt state must fail the add (required=true)"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("filter"),
        "stderr should mention the filter: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_attributes_leftover_without_state_leaks() {
    // DOCUMENTED HAZARD: if the attributes entry survives but the state dir is
    // gone, clean passes through and the overlay CAN be committed. This is why
    // `filter uninstall`/`remove` must keep attrs and state in sync.
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    // Simulate lost state (e.g. deleted .git/shadow-filter).
    std::fs::remove_dir_all(root.join(".git/shadow-filter")).unwrap();

    write(root, "f.txt", "a\nb-local\nteam\n");
    git_ok(root, &["add", "f.txt"]);
    git_ok(root, &["commit", "-qm", "leak"]);
    // The overlay leaked into the commit — documented, asserted, not hidden.
    assert_eq!(show_head(root, "f.txt"), "a\nb-local\nteam\n");
}

#[test]
fn test_missing_baseline_fails_closed() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");

    std::fs::remove_file(root.join(".git/shadow-filter/baselines/f.txt")).unwrap();
    let out = git(root, &["add", "f.txt"]);
    assert!(!out.status.success());
}

// ---------------------------------------------------------------------------
// file-type guards
// ---------------------------------------------------------------------------

#[test]
fn test_add_rejects_symlink() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    std::os::unix::fs::symlink("f.txt", root.join("link.txt")).unwrap();
    git_ok(root, &["add", "link.txt"]);
    git_ok(root, &["commit", "-qm", "link"]);
    let out = shadow(root, &["filter", "add", "link.txt"]);
    assert!(!out.status.success());
}

#[test]
fn test_add_rejects_binary() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    std::fs::write(root.join("bin.dat"), b"\x00\x01\x02binary").unwrap();
    git_ok(root, &["add", "bin.dat"]);
    git_ok(root, &["commit", "-qm", "bin"]);
    let out = shadow(root, &["filter", "add", "bin.dat"]);
    assert!(!out.status.success());
}

#[test]
fn test_add_rejects_untracked() {
    let dir = repo_with_file("f.txt", "a\n");
    let root = dir.path();
    write(root, "new.txt", "n\n");
    let out = shadow(root, &["filter", "add", "new.txt"]);
    assert!(!out.status.success());
}

// ---------------------------------------------------------------------------
// idempotency
// ---------------------------------------------------------------------------

#[test]
fn test_clean_smudge_roundtrip_identity() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // clean(smudge(R)) == R for a moved-upstream blob R
    let r = b"a-up\nb\nc\n";
    let (smudged, ok) = run_filter(root, "smudge", "f.txt", r);
    assert!(ok);
    assert_eq!(smudged, b"a-up\nb-local\nc\n");
    let (cleaned, ok) = run_filter(root, "clean", "f.txt", &smudged);
    assert!(ok);
    assert_eq!(cleaned, r, "clean(smudge(R)) must equal R");
}

#[test]
fn test_smudge_clean_roundtrip_worktree() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // smudge(clean(W)) == W — repeated conversion must not grow diffs.
    let w = b"a\nb-local\nc\nteam\n";
    let (cleaned, ok) = run_filter(root, "clean", "f.txt", w);
    assert!(ok);
    assert_eq!(cleaned, b"a\nb\nc\nteam\n");
    let (smudged, ok) = run_filter(root, "smudge", "f.txt", &cleaned);
    assert!(ok);
    assert_eq!(smudged, w, "smudge(clean(W)) must equal W");

    // Applying clean repeatedly is a no-op.
    let (again, ok) = run_filter(root, "clean", "f.txt", &cleaned);
    assert!(ok);
    assert_eq!(again, cleaned, "clean must be idempotent");
}

#[test]
fn test_repeated_status_does_not_drift() {
    let dir = repo_with_file("f.txt", "a\nb\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\n");
    filter_add(root, "f.txt");
    for _ in 0..3 {
        let out = git_ok(root, &["status", "--short"]);
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }
}

// ---------------------------------------------------------------------------
// overlay-region edits
// ---------------------------------------------------------------------------

#[test]
fn test_edit_inside_overlay_region_conflicts_then_sync() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");

    // Edit the overlay line itself — ambiguous: T or new overlay?
    write(root, "f.txt", "a\nb-local-edited\nc\n");
    let out = git(root, &["add", "f.txt"]);
    assert!(
        !out.status.success(),
        "editing an overlay line must fail the add (ambiguous L vs T)"
    );

    // `filter sync` declares: current worktree IS the new overlay.
    shadow_ok(root, &["filter", "sync", "f.txt"]);
    git_ok(root, &["add", "f.txt"]);
    // clean(W) == B again: nothing staged, HEAD keeps the baseline.
    let staged = git_ok(root, &["diff", "--cached"]);
    assert!(staged.stdout.is_empty());
    assert_eq!(show_head(root, "f.txt"), "a\nb\nc\n");
    assert_eq!(read(root, "f.txt"), "a\nb-local-edited\nc\n");
}

#[test]
fn test_status_and_diff_show_clean_view() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nb-local\nc\nteam\n");

    let diff = git_ok(root, &["diff"]);
    let diff = String::from_utf8_lossy(&diff.stdout);
    assert!(diff.contains("+team"), "diff should show team edit: {diff}");
    assert!(
        !diff.contains("b-local") && !diff.contains("-b"),
        "diff must not show the overlay: {diff}"
    );

    let log = git_ok(root, &["log", "-p", "-1"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        !log.contains("b-local"),
        "history must not contain overlay: {log}"
    );
}

#[test]
fn test_filter_status_output() {
    let dir = repo_with_file("f.txt", "a\nb\nc\n");
    let root = dir.path();
    write(root, "f.txt", "a\nb-local\nc\n");
    filter_add(root, "f.txt");
    write(root, "f.txt", "a\nb-local\nc\nteam\n");

    let out = shadow_ok(root, &["filter", "status"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("f.txt"), "{s}");
    assert!(s.contains("commit would include"), "{s}");
    assert!(s.contains("local overlay"), "{s}");
}
