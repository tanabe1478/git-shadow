use anyhow::{bail, Result};

use crate::git::GitRepo;
use crate::hooks;

pub fn run(hook_name: &str) -> Result<()> {
    let git = GitRepo::discover(&std::env::current_dir()?)?;

    match hook_name {
        "pre-commit" => hooks::pre_commit::handle(&git),
        "post-commit" => hooks::post_commit::handle(&git),
        "post-merge" => hooks::post_merge::handle(&git),
        _ => bail!("unknown hook name: {}", hook_name),
    }
}
