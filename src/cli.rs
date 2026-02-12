use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-shadow",
    about = "Manage local-only changes in Git repositories"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Set up Git hooks
    Install,

    /// Register a file for shadow management
    Add {
        /// Target file path
        file: String,
        /// Register as a phantom (local-only file)
        #[arg(long)]
        phantom: bool,
        /// Skip adding to .git/info/exclude (phantom only)
        #[arg(long)]
        no_exclude: bool,
        /// Ignore file size limit
        #[arg(long)]
        force: bool,
    },

    /// Unregister a file from shadow management
    Remove {
        /// Target file path
        file: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Show managed files and their status
    Status,

    /// Show shadow changes as a diff
    Diff {
        /// Target file path (omit for all files)
        file: Option<String>,
    },

    /// Update baseline and re-apply shadow changes
    Rebase {
        /// Target file path (omit for all files)
        file: Option<String>,
    },

    /// Recover from abnormal state
    Restore {
        /// Target file path (omit for all files)
        file: Option<String>,
    },

    /// Suspend shadow changes for branch switching
    Suspend,

    /// Resume suspended shadow changes
    Resume,

    /// Diagnose hooks and configuration
    Doctor,

    /// Internal subcommand called from hooks
    #[command(hide = true)]
    Hook {
        /// Hook name (pre-commit, post-commit, post-merge)
        hook_name: String,
    },
}
