use anyhow::Result;

use crate::commands::rebase;
use crate::git::GitRepo;

pub fn handle(git: &GitRepo) -> Result<()> {
    rebase::auto_rebase_all(git, "post-rewrite")
}
