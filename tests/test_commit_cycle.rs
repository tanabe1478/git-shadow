//! E2E integration test: full commit cycle
//! install → add → edit → commit → verify

mod common;

use git_shadow::config::ShadowConfig;
use git_shadow::git::GitRepo;
use git_shadow::hooks;
use git_shadow::path;
use git_shadow::{fs_util, lock};

#[test]
fn test_full_overlay_commit_cycle() {
    let repo = common::TestRepo::new();

    // 1. Create initial file and commit
    repo.create_file("CLAUDE.md", "# Team\n");
    repo.commit("initial commit");

    let git = GitRepo::discover(&repo.root).unwrap();

    // 2. Install shadow
    repo.init_shadow();
    install_hooks_for_test(&git);

    // 3. Add overlay
    let commit = git.head_commit().unwrap();
    let baseline_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
    let encoded = path::encode_path("CLAUDE.md");
    fs_util::atomic_write(
        &git.shadow_dir.join("baselines").join(&encoded),
        &baseline_content,
    )
    .unwrap();
    let mut config = ShadowConfig::new();
    config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
    config.save(&git.shadow_dir).unwrap();

    // 4. Add shadow changes
    std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My personal notes\n").unwrap();

    // 5. Stage the file
    git.add("CLAUDE.md").unwrap();

    // 6. Run pre-commit hook
    hooks::pre_commit::handle(&git).unwrap();

    // Verify: working tree has baseline content
    let wt_content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
    assert_eq!(
        wt_content, "# Team\n",
        "Working tree should have baseline after pre-commit"
    );

    // Verify: stash has shadow content
    let stash_content =
        std::fs::read_to_string(git.shadow_dir.join("stash").join("CLAUDE.md")).unwrap();
    assert_eq!(
        stash_content, "# Team\n# My personal notes\n",
        "Stash should have shadow content"
    );

    // 7. Actually commit (simulated - just verify the index)
    std::process::Command::new("git")
        .args(["commit", "-m", "team update", "--no-verify"])
        .current_dir(&git.root)
        .output()
        .unwrap();

    // 8. Run post-commit hook
    hooks::post_commit::handle(&git).unwrap();

    // Verify: working tree has shadow content back
    let wt_after = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
    assert_eq!(
        wt_after, "# Team\n# My personal notes\n",
        "Working tree should have shadow content after post-commit"
    );

    // Verify: stash is clean
    assert!(
        !git.shadow_dir.join("stash").join("CLAUDE.md").exists(),
        "Stash should be clean after post-commit"
    );

    // Verify: lock is released
    assert!(
        matches!(
            lock::check_lock(&git.shadow_dir).unwrap(),
            lock::LockStatus::Free
        ),
        "Lock should be released after post-commit"
    );

    // 9. Verify commit content (should have baseline, not shadow)
    let committed_content = git.show_file("HEAD", "CLAUDE.md").unwrap();
    assert_eq!(
        String::from_utf8_lossy(&committed_content),
        "# Team\n",
        "Committed content should be baseline, not shadow"
    );
}

#[test]
fn test_full_phantom_commit_cycle() {
    let repo = common::TestRepo::new();

    // 1. Create initial file and commit
    repo.create_file("README.md", "# Project\n");
    repo.commit("initial commit");

    let git = GitRepo::discover(&repo.root).unwrap();

    // 2. Install shadow
    repo.init_shadow();
    install_hooks_for_test(&git);

    // 3. Add phantom
    let mut config = ShadowConfig::new();
    repo.create_file("local-notes.md", "# My local notes\n");
    config
        .add_phantom(
            "local-notes.md".to_string(),
            git_shadow::config::ExcludeMode::None,
        )
        .unwrap();
    config.save(&git.shadow_dir).unwrap();

    // 4. Stage phantom file (simulating accidental git add)
    std::process::Command::new("git")
        .args(["add", "local-notes.md"])
        .current_dir(&git.root)
        .output()
        .unwrap();

    // 5. Run pre-commit hook
    hooks::pre_commit::handle(&git).unwrap();

    // Verify: phantom file is stashed
    let stash_content =
        std::fs::read_to_string(git.shadow_dir.join("stash").join("local-notes.md")).unwrap();
    assert_eq!(stash_content, "# My local notes\n");

    // 6. Commit
    std::process::Command::new("git")
        .args([
            "commit",
            "-m",
            "some commit",
            "--no-verify",
            "--allow-empty",
        ])
        .current_dir(&git.root)
        .output()
        .unwrap();

    // 7. Run post-commit hook
    hooks::post_commit::handle(&git).unwrap();

    // Verify: phantom file restored to working tree
    let wt = std::fs::read_to_string(git.root.join("local-notes.md")).unwrap();
    assert_eq!(wt, "# My local notes\n");

    // Verify: phantom file is NOT in the commit
    let show_result = git.show_file("HEAD", "local-notes.md");
    assert!(
        show_result.is_err(),
        "Phantom file should NOT be in the commit"
    );
}

#[test]
fn test_pre_commit_rollback_on_error() {
    let repo = common::TestRepo::new();

    repo.create_file("CLAUDE.md", "# Team\n");
    repo.commit("initial commit");

    let git = GitRepo::discover(&repo.root).unwrap();
    repo.init_shadow();

    // Setup overlay
    let commit = git.head_commit().unwrap();
    let mut config = ShadowConfig::new();
    let baseline = git.show_file("HEAD", "CLAUDE.md").unwrap();
    let encoded = path::encode_path("CLAUDE.md");
    fs_util::atomic_write(&git.shadow_dir.join("baselines").join(&encoded), &baseline).unwrap();
    config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
    config.save(&git.shadow_dir).unwrap();

    // Add shadow changes
    std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# My shadow\n").unwrap();

    // Create stash remnant to trigger error
    std::fs::write(git.shadow_dir.join("stash").join("old.md"), "remnant").unwrap();

    // Pre-commit should fail
    let result = hooks::pre_commit::handle(&git);
    assert!(
        result.is_err(),
        "Pre-commit should fail due to stash remnants"
    );

    // Working tree should still have shadow content (not overwritten)
    let content = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
    assert_eq!(
        content, "# Team\n# My shadow\n",
        "Shadow content should be preserved after failed pre-commit"
    );
}

fn install_hooks_for_test(git: &GitRepo) {
    let hooks_dir = git.git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();

    for name in &["pre-commit", "post-commit", "post-merge"] {
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
