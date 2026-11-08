use anyhow::{bail, Result};

use crate::git::GitRepo;
use crate::hooks;

pub fn run(hook_name: &str) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;

    // Hooks can live in a directory shared with repositories (or worktrees) where
    // `git-shadow install` was never run, e.g. a `core.hooksPath` set in the global
    // git config. Nothing is managed there, so let the git operation proceed.
    if !git.shadow_dir.exists() {
        return Ok(());
    }

    match hook_name {
        "pre-commit" => hooks::pre_commit::handle(&git),
        "post-commit" => hooks::post_commit::handle(&git),
        "post-merge" => hooks::post_merge::handle(&git),
        "post-rewrite" => hooks::post_rewrite::handle(&git),
        _ => bail!("unknown hook name: {}", hook_name),
    }
}
