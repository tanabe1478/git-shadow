use anyhow::Result;
use clap::Parser;

use git_shadow::cli::{Cli, Commands};
use git_shadow::commands;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install => commands::install::run()?,
        Commands::Add {
            file,
            phantom,
            no_exclude,
            force,
        } => commands::add::run(&file, phantom, no_exclude, force)?,
        Commands::Remove { file, force } => commands::remove::run(&file, force)?,
        Commands::Status => commands::status::run()?,
        Commands::Diff { file } => commands::diff::run(file.as_deref())?,
        Commands::Rebase { file } => commands::rebase::run(file.as_deref())?,
        Commands::Restore { file } => commands::restore::run(file.as_deref())?,
        Commands::Doctor => commands::doctor::run()?,
        Commands::Hook { hook_name } => commands::hook::run(&hook_name)?,
    }

    Ok(())
}
