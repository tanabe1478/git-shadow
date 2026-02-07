use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShadowError {
    #[error("not a Git repository")]
    NotAGitRepo,

    #[error("shadow directory not initialized. Run `git-shadow install`")]
    NotInitialized,

    #[error("file '{0}' is not tracked by Git")]
    FileNotTracked(String),

    #[error("file '{0}' is already managed by git-shadow")]
    AlreadyManaged(String),

    #[error("file '{0}' is not managed by git-shadow")]
    NotManaged(String),

    #[error("file '{0}' is a binary file")]
    BinaryFile(String),

    #[error("file '{0}' exceeds size limit ({1} bytes > {2} bytes). Use --force to override")]
    FileTooLarge(String, u64, u64),

    #[error("lock held by process {pid} (started: {timestamp})")]
    LockHeld { pid: u32, timestamp: String },

    #[error("stale lock detected (PID {0} no longer exists). Run `git-shadow restore`")]
    StaleLock(u32),

    #[error("stash has remaining files. Run `git-shadow restore`")]
    StashRemaining,

    #[error("partial staging detected for shadow-managed file '{0}'. Run `git add {0}` to stage the entire file before committing")]
    PartialStage(String),

    #[error("baseline missing for file '{0}'")]
    BaselineMissing(String),

    #[error("file '{0}' does not exist in the working tree")]
    FileMissing(String),

    #[error("failed to unstage phantom file '{0}'. Run `git reset -- {0}` manually")]
    UnstageFailure(String),

    #[error("git command failed: {command}\n{stderr}")]
    GitCommand { command: String, stderr: String },

    #[error("hooks not installed. Run `git-shadow install`")]
    HooksNotInstalled,

    #[error("cannot run in non-interactive mode without --force")]
    NonInteractiveWithoutForce,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
