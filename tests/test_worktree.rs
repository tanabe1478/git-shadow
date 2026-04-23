//! E2E integration test: git worktree support
//! Verify git-shadow works correctly inside git worktrees.

mod common;

use git_shadow::config::ShadowConfig;
use git_shadow::git::GitRepo;
use git_shadow::hooks;
use git_shadow::path;
use git_shadow::{fs_util, lock};

#[test]
fn test_discover_in_worktree() {
    let repo = common::TestRepo::new();
    repo.create_file("README.md", "# Hello\n");
    repo.commit("initial commit");

    let wt_path = repo.add_worktree("wt-discover");

    let wt_repo = GitRepo::discover(&wt_path).unwrap();

    // root should be the worktree path
    assert_eq!(
        wt_repo.root.canonicalize().unwrap(),
        wt_path.canonicalize().unwrap()
    );

    // git_dir should be under worktrees/
    assert!(
        wt_repo.git_dir.to_str().unwrap().contains("worktrees"),
        "git_dir should be under worktrees/: {:?}",
        wt_repo.git_dir
    );

    // common_dir should point to main repo's .git
    let main_git = GitRepo::discover(&repo.root).unwrap();
    assert_eq!(
        wt_repo.common_dir.canonicalize().unwrap(),
        main_git.git_dir.canonicalize().unwrap()
    );

    // git_dir != common_dir in a worktree
    assert_ne!(wt_repo.git_dir, wt_repo.common_dir);

    // shadow_dir should be under git_dir (per-worktree)
    assert!(wt_repo.shadow_dir.starts_with(&wt_repo.git_dir));
}

#[test]
fn test_install_in_worktree() {
    let repo = common::TestRepo::new();
    repo.create_file("README.md", "# Hello\n");
    repo.commit("initial commit");

    let wt_path = repo.add_worktree("wt-install");
    let wt_repo = GitRepo::discover(&wt_path).unwrap();

    // Initialize shadow in worktree
    std::fs::create_dir_all(wt_repo.shadow_dir.join("baselines")).unwrap();
    std::fs::create_dir_all(wt_repo.shadow_dir.join("stash")).unwrap();
    install_hooks_for_test(&wt_repo);

    // Hooks should be in common_dir (main repo's .git/hooks/)
    let hooks_dir = wt_repo.hooks_dir();
    assert!(hooks_dir.join("pre-commit").exists());
    assert!(hooks_dir.join("post-commit").exists());
    assert!(hooks_dir.join("post-merge").exists());
    assert!(hooks_dir.join("post-rewrite").exists());

    // Hooks should NOT be under worktree's git_dir
    assert!(!wt_repo.git_dir.join("hooks").join("pre-commit").exists());

    // Shadow dir should be under worktree's git_dir
    assert!(wt_repo.shadow_dir.exists());
    assert!(wt_repo.shadow_dir.starts_with(&wt_repo.git_dir));
}

#[test]
fn test_overlay_commit_cycle_in_worktree() {
    let repo = common::TestRepo::new();
    repo.create_file("CLAUDE.md", "# Team\n");
    repo.commit("initial commit");

    let wt_path = repo.add_worktree("wt-overlay");
    let wt_repo = GitRepo::discover(&wt_path).unwrap();

    // Install shadow in worktree
    std::fs::create_dir_all(wt_repo.shadow_dir.join("baselines")).unwrap();
    std::fs::create_dir_all(wt_repo.shadow_dir.join("stash")).unwrap();
    install_hooks_for_test(&wt_repo);

    // Add overlay
    let commit = wt_repo.head_commit().unwrap();
    let baseline_content = wt_repo.show_file("HEAD", "CLAUDE.md").unwrap();
    let encoded = path::encode_path("CLAUDE.md");
    fs_util::atomic_write(
        &wt_repo.shadow_dir.join("baselines").join(&encoded),
        &baseline_content,
    )
    .unwrap();

    let mut config = ShadowConfig::new();
    config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
    config.save(&wt_repo.shadow_dir).unwrap();

    // Apply shadow changes in worktree
    std::fs::write(wt_path.join("CLAUDE.md"), "# Team\n# My notes\n").unwrap();

    // Stage the file in the worktree
    std::process::Command::new("git")
        .args(["add", "CLAUDE.md"])
        .current_dir(&wt_path)
        .output()
        .unwrap();

    // Run pre-commit hook
    hooks::pre_commit::handle(&wt_repo).unwrap();

    // Working tree should have baseline (shadow stripped)
    let wt_content = std::fs::read_to_string(wt_path.join("CLAUDE.md")).unwrap();
    assert_eq!(
        wt_content, "# Team\n",
        "Working tree should have baseline during commit"
    );

    // Commit (simulated)
    std::process::Command::new("git")
        .args(["commit", "-m", "test commit", "--no-verify"])
        .current_dir(&wt_path)
        .output()
        .unwrap();

    // Run post-commit hook
    hooks::post_commit::handle(&wt_repo).unwrap();

    // Working tree should have shadow content back
    let wt_after = std::fs::read_to_string(wt_path.join("CLAUDE.md")).unwrap();
    assert_eq!(
        wt_after, "# Team\n# My notes\n",
        "Working tree should have shadow content after post-commit"
    );

    // Lock should be released
    assert!(matches!(
        lock::check_lock(&wt_repo.shadow_dir).unwrap(),
        lock::LockStatus::Free
    ));

    // Committed content should be baseline
    let committed = wt_repo.show_file("HEAD", "CLAUDE.md").unwrap();
    assert_eq!(
        String::from_utf8_lossy(&committed),
        "# Team\n",
        "Committed content should be baseline"
    );
}

#[test]
fn test_install_inherits_config_from_main_worktree() {
    let repo = common::TestRepo::new();
    repo.create_file("CLAUDE.md", "# Team\n");
    repo.create_file("local.md", "local notes");
    repo.commit("initial commit");

    let main_git = GitRepo::discover(&repo.root).unwrap();

    // Set up shadow in main repo
    std::fs::create_dir_all(main_git.shadow_dir.join("baselines")).unwrap();
    std::fs::create_dir_all(main_git.shadow_dir.join("stash")).unwrap();
    install_hooks_for_test(&main_git);

    // Register an overlay
    let commit = main_git.head_commit().unwrap();
    let baseline = main_git.show_file("HEAD", "CLAUDE.md").unwrap();
    let encoded = path::encode_path("CLAUDE.md");
    fs_util::atomic_write(
        &main_git.shadow_dir.join("baselines").join(&encoded),
        &baseline,
    )
    .unwrap();
    let mut config = ShadowConfig::new();
    config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
    config.save(&main_git.shadow_dir).unwrap();

    // Create worktree and install
    let wt_path = repo.add_worktree("wt-inherit");
    let wt_git = GitRepo::discover(&wt_path).unwrap();
    std::fs::create_dir_all(wt_git.shadow_dir.join("baselines")).unwrap();
    std::fs::create_dir_all(wt_git.shadow_dir.join("stash")).unwrap();
    git_shadow::commands::install::inherit_from_main_worktree(&wt_git).unwrap();

    // Config should be inherited
    let wt_config = ShadowConfig::load(&wt_git.shadow_dir).unwrap();
    assert_eq!(wt_config.files.len(), 1);
    assert!(wt_config.get("CLAUDE.md").is_some());

    // Baseline should exist and match HEAD
    let wt_baseline = wt_git.shadow_dir.join("baselines").join(&encoded);
    assert!(wt_baseline.exists());
    let content = std::fs::read_to_string(&wt_baseline).unwrap();
    assert_eq!(content, "# Team\n");
}

fn install_hooks_for_test(git: &GitRepo) {
    let hooks_dir = git.hooks_dir();
    std::fs::create_dir_all(&hooks_dir).unwrap();

    for name in &["pre-commit", "post-commit", "post-merge", "post-rewrite"] {
        let content = format!("#!/bin/sh\nexec git-shadow hook {}\n", name);
        std::fs::write(hooks_dir.join(name), content).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(hooks_dir.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }
}
