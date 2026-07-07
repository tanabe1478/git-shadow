use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::ui::UiLocale;

#[derive(Parser)]
#[command(
    name = "git-shadow",
    version,
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

    /// Remove git-shadow hooks, exclude entries, and state
    Uninstall {
        /// Restore overlay baselines and wipe state even if files are still managed
        #[arg(long)]
        force: bool,
    },

    /// Register a file for shadow management
    Add {
        /// Target file paths
        files: Vec<String>,
        /// Register as overlay even if auto-detection would choose phantom
        #[arg(long, conflicts_with = "phantom")]
        overlay: bool,
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
    Status {
        /// Show `git status --short --branch` before the shadow summary
        #[arg(long)]
        git: bool,
        /// Emit machine-readable JSON (English, not localized)
        #[arg(long)]
        json: bool,
    },

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

    /// Export git-shadow state to a portable archive
    Export {
        /// Output archive path (default: git-shadow-export.tar.gz)
        output: Option<String>,
        /// Overwrite the output file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Import git-shadow state from a portable archive
    Import {
        /// Path to the archive to import
        archive: String,
        /// Overwrite conflicting files and replace differing entries
        #[arg(long)]
        force: bool,
    },

    /// Suspend shadow changes for branch switching
    Suspend,

    /// Resume suspended shadow changes
    Resume,

    /// Diagnose hooks and configuration
    Doctor {
        /// Emit machine-readable JSON (English, not localized)
        #[arg(long)]
        json: bool,
    },

    /// Internal subcommand called from hooks
    #[command(hide = true)]
    Hook {
        /// Hook name (pre-commit, post-commit, post-merge, post-rewrite)
        hook_name: String,
    },
}

impl Cli {
    pub fn parse_localized(locale: UiLocale) -> Self {
        let command = localize_command(Self::command(), locale);
        let matches = command.get_matches();
        Self::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
    }
}

fn localize_command(command: clap::Command, locale: UiLocale) -> clap::Command {
    let command = command
        .about(match locale {
            UiLocale::Ja => "Git リポジトリ内のローカル専用変更を管理します",
            UiLocale::En => "Manage local-only changes in Git repositories",
        })
        .long_about(match locale {
            UiLocale::Ja => "Git リポジトリ内のローカル専用変更を管理します",
            UiLocale::En => "Manage local-only changes in Git repositories",
        });

    localize_subcommands(command, locale)
}

fn localize_subcommands(command: clap::Command, locale: UiLocale) -> clap::Command {
    let command = command.mut_subcommand("install", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "Git hooks を設定する",
            UiLocale::En => "Set up Git hooks",
        })
    });
    let command = command.mut_subcommand("uninstall", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "git-shadow の hooks・exclude・state を削除する",
            UiLocale::En => "Remove git-shadow hooks, exclude entries, and state",
        })
        .mut_arg("force", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "管理対象が残っていても overlay を復元して state を削除する",
                UiLocale::En => {
                    "Restore overlay baselines and wipe state even if files are still managed"
                }
            })
        })
    });
    let command = command.mut_subcommand("add", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "ファイルを shadow 管理に登録する",
            UiLocale::En => "Register a file for shadow management",
        })
        .mut_arg("files", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "対象ファイルのパス (複数指定可)",
                UiLocale::En => "Target file paths (multiple allowed)",
            })
        })
        .mut_arg("overlay", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "overlay として強制登録する",
                UiLocale::En => "Force overlay registration",
            })
        })
        .mut_arg("phantom", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "phantom (ローカル専用ファイル) として強制登録する",
                UiLocale::En => "Force phantom registration",
            })
        })
        .mut_arg("no_exclude", |arg| {
            arg.help(match locale {
                UiLocale::Ja => ".git/info/exclude への追加をスキップする (phantom のみ)",
                UiLocale::En => "Skip adding to .git/info/exclude (phantom only)",
            })
        })
        .mut_arg("force", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "ファイルサイズ制限を無視する",
                UiLocale::En => "Ignore file size limit",
            })
        })
    });
    let command = command.mut_subcommand("remove", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "ファイルを shadow 管理から外す",
            UiLocale::En => "Unregister a file from shadow management",
        })
        .mut_arg("file", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "対象ファイルのパス",
                UiLocale::En => "Target file path",
            })
        })
        .mut_arg("force", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "確認プロンプトをスキップする",
                UiLocale::En => "Skip confirmation prompt",
            })
        })
    });
    let command = command.mut_subcommand("status", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "管理対象ファイルと状態を表示する",
            UiLocale::En => "Show managed files and their status",
        })
        .mut_arg("git", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "`git status --short --branch` も先に表示する",
                UiLocale::En => "Also show `git status --short --branch` first",
            })
        })
        .mut_arg("json", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "機械可読な JSON を出力する (英語・非ローカライズ)",
                UiLocale::En => "Emit machine-readable JSON (English, not localized)",
            })
        })
    });
    let command = command.mut_subcommand("diff", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "shadow changes を diff 表示する",
            UiLocale::En => "Show shadow changes as a diff",
        })
        .mut_arg("file", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "対象ファイルのパス (省略時は全件)",
                UiLocale::En => "Target file path (omit for all files)",
            })
        })
    });
    let command = command.mut_subcommand("rebase", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "baseline を更新して shadow changes を再適用する",
            UiLocale::En => "Update baseline and re-apply shadow changes",
        })
        .mut_arg("file", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "対象ファイルのパス (省略時は全件)",
                UiLocale::En => "Target file path (omit for all files)",
            })
        })
    });
    let command = command.mut_subcommand("restore", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "異常状態から回復する",
            UiLocale::En => "Recover from abnormal state",
        })
        .mut_arg("file", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "対象ファイルのパス (省略時は全件)",
                UiLocale::En => "Target file path (omit for all files)",
            })
        })
    });
    let command = command.mut_subcommand("export", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "git-shadow の state をポータブルな archive に export する",
            UiLocale::En => "Export git-shadow state to a portable archive",
        })
        .mut_arg("output", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "出力先の archive パス (既定: git-shadow-export.tar.gz)",
                UiLocale::En => "Output archive path (default: git-shadow-export.tar.gz)",
            })
        })
        .mut_arg("force", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "出力先が既に存在しても上書きする",
                UiLocale::En => "Overwrite the output file if it already exists",
            })
        })
    });
    let command = command.mut_subcommand("import", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "ポータブルな archive から git-shadow の state を import する",
            UiLocale::En => "Import git-shadow state from a portable archive",
        })
        .mut_arg("archive", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "import する archive のパス",
                UiLocale::En => "Path to the archive to import",
            })
        })
        .mut_arg("force", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "衝突するファイルを上書きし、異なる登録を置き換える",
                UiLocale::En => "Overwrite conflicting files and replace differing entries",
            })
        })
    });
    let command = command.mut_subcommand("suspend", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "ブランチ切り替えのため shadow changes を退避する",
            UiLocale::En => "Suspend shadow changes for branch switching",
        })
    });
    let command = command.mut_subcommand("resume", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "suspend された shadow changes を復元する",
            UiLocale::En => "Resume suspended shadow changes",
        })
    });
    command.mut_subcommand("doctor", |sub| {
        sub.about(match locale {
            UiLocale::Ja => "hooks と設定を診断する",
            UiLocale::En => "Diagnose hooks and configuration",
        })
        .mut_arg("json", |arg| {
            arg.help(match locale {
                UiLocale::Ja => "機械可読な JSON を出力する (英語・非ローカライズ)",
                UiLocale::En => "Emit machine-readable JSON (English, not localized)",
            })
        })
    })
}
