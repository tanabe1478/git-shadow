use git_shadow::cli::{Cli, Commands};
use git_shadow::commands;
use git_shadow::ui;

fn main() {
    if let Err(err) = run() {
        eprintln!("{}", ui::format_error(&err, ui::detect_locale()));
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let locale = ui::detect_locale();
    let cli = Cli::parse_localized(locale);

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
        Commands::Suspend => commands::suspend::run()?,
        Commands::Resume => commands::resume::run()?,
        Commands::Doctor => commands::doctor::run()?,
        Commands::Hook { hook_name } => commands::hook::run(&hook_name)?,
    }

    Ok(())
}
