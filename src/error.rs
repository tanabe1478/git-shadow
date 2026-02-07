use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShadowError {
    #[error("Git リポジトリではありません")]
    NotAGitRepo,

    #[error("shadow ディレクトリが未初期化です。`git-shadow install` を実行してください")]
    NotInitialized,

    #[error("ファイル '{0}' は Git で追跡されていません")]
    FileNotTracked(String),

    #[error("ファイル '{0}' は既に git-shadow で管理されています")]
    AlreadyManaged(String),

    #[error("ファイル '{0}' は git-shadow で管理されていません")]
    NotManaged(String),

    #[error("ファイル '{0}' はバイナリファイルです")]
    BinaryFile(String),

    #[error(
        "ファイル '{0}' がサイズ上限を超えています ({1} bytes > {2} bytes)。--force で突破可能です"
    )]
    FileTooLarge(String, u64, u64),

    #[error("ロックがプロセス {pid} に保持されています (開始: {timestamp})")]
    LockHeld { pid: u32, timestamp: String },

    #[error("stale ロックを検出しました (PID {0} は既に終了しています)。`git-shadow restore` を実行してください")]
    StaleLock(u32),

    #[error("stash にファイルが残っています。`git-shadow restore` を実行してください")]
    StashRemaining,

    #[error("git-shadow 管理下のファイル '{0}' に部分ステージが検出されました。`git add {0}` でファイル全体をステージしてから再度コミットしてください")]
    PartialStage(String),

    #[error("ファイル '{0}' のベースラインがありません")]
    BaselineMissing(String),

    #[error("ファイル '{0}' がワーキングツリーに存在しません")]
    FileMissing(String),

    #[error("phantom ファイル '{0}' をステージングから除外できませんでした。手動で `git reset -- {0}` を実行してください")]
    UnstageFailure(String),

    #[error("Git コマンドが失敗しました: {command}\n{stderr}")]
    GitCommand { command: String, stderr: String },

    #[error("hooks がインストールされていません。`git-shadow install` を実行してください")]
    HooksNotInstalled,

    #[error("--force なしでは非対話環境で実行できません")]
    NonInteractiveWithoutForce,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
