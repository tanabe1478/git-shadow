use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-shadow",
    about = "Git リポジトリ内のローカル限定変更を管理する"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Git hooks をセットアップする
    Install,

    /// ファイルを shadow 管理に登録する
    Add {
        /// 対象ファイルパス
        file: String,
        /// phantom (新規ローカルファイル) として登録
        #[arg(long)]
        phantom: bool,
        /// .git/info/exclude への追加をスキップ (phantom のみ)
        #[arg(long)]
        no_exclude: bool,
        /// サイズ上限を無視
        #[arg(long)]
        force: bool,
    },

    /// ファイルを shadow 管理から解除する
    Remove {
        /// 対象ファイルパス
        file: String,
        /// 確認プロンプトをスキップ
        #[arg(long)]
        force: bool,
    },

    /// 管理対象ファイルの一覧と状態を表示する
    Status,

    /// shadow 変更の差分を表示する
    Diff {
        /// 対象ファイルパス (省略時: 全ファイル)
        file: Option<String>,
    },

    /// ベースラインを更新し shadow 変更を再適用する
    Rebase {
        /// 対象ファイルパス (省略時: 全ファイル)
        file: Option<String>,
    },

    /// 異常状態からの復旧を行う
    Restore {
        /// 対象ファイルパス (省略時: 全ファイル)
        file: Option<String>,
    },

    /// hooks と設定の状態を診断する
    Doctor,

    /// hook から呼び出される内部サブコマンド
    #[command(hide = true)]
    Hook {
        /// hook 名 (pre-commit, post-commit, post-merge)
        hook_name: String,
    },
}
