use crate::error::ShadowError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiLocale {
    Ja,
    En,
}

pub fn detect_locale() -> UiLocale {
    detect_locale_from_values(
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok()),
    )
}

pub fn format_error(err: &anyhow::Error, locale: UiLocale) -> String {
    if let Some(shadow_error) = find_shadow_error(err) {
        return format_shadow_error(shadow_error, locale);
    }

    match locale {
        UiLocale::Ja => format!("エラー: {}", err),
        UiLocale::En => format!("Error: {}", err),
    }
}

pub fn warning_hooks_not_installed(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "warning: hooks が未インストールです。`git-shadow install` を実行してください",
        "warning: hooks not installed. Run `git-shadow install`",
    )
}

pub fn registered_overlay(locale: UiLocale, path: &str, baseline: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} を overlay として登録しました (baseline: {baseline})"),
        UiLocale::En => format!("registered {path} as overlay (baseline: {baseline})"),
    }
}

pub fn registered_phantom(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} を phantom として登録しました"),
        UiLocale::En => format!("registered {path} as phantom"),
    }
}

pub fn registered_phantom_directory(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} を phantom directory として登録しました"),
        UiLocale::En => format!("registered {path} as phantom directory"),
    }
}

pub fn add_failed(locale: UiLocale, path: &str, message: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} の登録に失敗しました: {message}"),
        UiLocale::En => format!("failed to register {path}: {message}"),
    }
}

pub fn no_managed_files(locale: UiLocale) -> &'static str {
    choose(locale, "管理対象ファイルはありません", "no managed files")
}

pub fn not_managed_message(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} は git-shadow の管理対象ではありません"),
        UiLocale::En => format!("{path} is not managed by git-shadow"),
    }
}

pub fn remove_prompt_overlay(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => {
            format!("{path} の shadow changes は破棄されます。続行しますか? [y/N]")
        }
        UiLocale::En => {
            format!("Shadow changes for {path} will be discarded. Continue? [y/N]")
        }
    }
}

pub fn remove_prompt_phantom(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "{path} を shadow 管理から外します。ファイル自体は残ります。続行しますか? [y/N]"
        ),
        UiLocale::En => format!(
            "{path} will be unregistered from shadow management. The file itself will remain. Continue? [y/N]"
        ),
    }
}

pub fn remove_prompt_phantom_directory(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "{path} (directory) を shadow 管理から外します。ディレクトリと中身は残ります。続行しますか? [y/N]"
        ),
        UiLocale::En => format!(
            "{path} (directory) will be unregistered from shadow management. The directory and its contents will remain. Continue? [y/N]"
        ),
    }
}

pub fn aborted(locale: UiLocale) -> &'static str {
    choose(locale, "中止しました", "aborted")
}

pub fn unregistered(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} を shadow 管理から解除しました"),
        UiLocale::En => format!("unregistered {path} from shadow management"),
    }
}

pub fn install_success(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "git-shadow hooks をインストールしました",
        "git-shadow hooks installed successfully",
    )
}

pub fn install_custom_hooks_path(locale: UiLocale, hooks_path: &str, resolved: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "note: core.hooksPath ({hooks_path}) が設定されているため、hooks を {resolved} にインストールしました"
        ),
        UiLocale::En => format!(
            "note: core.hooksPath ({hooks_path}) is set, so hooks were installed into {resolved}"
        ),
    }
}

pub fn uninstall_success(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "git-shadow を uninstall しました (hooks・exclude・state を削除しました)",
        "git-shadow uninstalled (hooks, exclude entries, and state removed)",
    )
}

pub fn uninstall_hook_restored(locale: UiLocale, hook: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{hook} hook の pre-shadow backup を復元しました"),
        UiLocale::En => format!("restored pre-shadow backup for {hook} hook"),
    }
}

pub fn uninstall_forced_overlays(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => {
            format!("{count} 件の overlay の baseline を working tree に復元しました")
        }
        UiLocale::En => format!("restored baselines to the working tree for {count} overlay(s)"),
    }
}

pub fn inherited_from_main_worktree(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("main worktree から {count} 件のファイル設定を引き継ぎました"),
        UiLocale::En => format!("inherited {count} file(s) from main worktree"),
    }
}

pub fn diff_not_managed(locale: UiLocale, path: &str) -> String {
    not_managed_message(locale, path)
}

pub fn diff_no_shadow_changes(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: shadow changes はありません"),
        UiLocale::En => format!("{path}: no shadow changes"),
    }
}

pub fn diff_phantom_directory(locale: UiLocale, path: &str, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: phantom directory ({count} entries)"),
        UiLocale::En => format!("{path}: phantom directory ({count} entries)"),
    }
}

pub fn diff_phantom_directory_missing(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: phantom directory が存在しません"),
        UiLocale::En => format!("{path}: phantom directory does not exist"),
    }
}

pub fn diff_file_missing(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: ファイルが存在しません"),
        UiLocale::En => format!("{path}: file does not exist"),
    }
}

pub fn diff_baseline_label(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("a/{path} (baseline)"),
        UiLocale::En => format!("a/{path} (baseline)"),
    }
}

pub fn diff_shadow_label(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("b/{path} (shadow)"),
        UiLocale::En => format!("b/{path} (shadow)"),
    }
}

pub fn restore_nothing(locale: UiLocale) -> &'static str {
    choose(locale, "復旧するものはありません", "nothing to restore")
}

pub fn restore_heading(locale: UiLocale) -> &'static str {
    choose(locale, "復旧したファイル:", "restored files:")
}

pub fn lockfile_removed(locale: UiLocale) -> &'static str {
    choose(locale, "lockfile を削除しました", "lockfile removed")
}

pub fn suspend_no_managed_files(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "suspend する管理対象ファイルはありません",
        "no managed files to suspend",
    )
}

pub fn suspend_success(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("{count} 件の shadow changes を suspend しました"),
        UiLocale::En => format!("shadow changes suspended for {count} file(s)"),
    }
}

pub fn suspend_worktree_clean(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "working tree は clean になりました。ブランチを切り替えられます",
        "working tree is now clean — you can switch branches",
    )
}

pub fn resume_success(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("{count} 件の shadow changes を resume しました"),
        UiLocale::En => format!("shadow changes resumed for {count} file(s)"),
    }
}

pub fn resume_warning_no_suspended_content(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("warning: {path} の suspended content がありません"),
        UiLocale::En => format!("warning: no suspended content for {path}"),
    }
}

pub fn resume_restored_file_absent_from_head(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: shadow changes を戻しました (HEAD にファイルなし)"),
        UiLocale::En => format!("{path}: shadow changes restored (file absent from HEAD)"),
    }
}

pub fn resume_restored_shadow_changes(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: shadow changes を戻しました"),
        UiLocale::En => format!("{path}: shadow changes restored"),
    }
}

pub fn resume_conflicts(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => {
            format!("warning: {path} で conflict が発生しました。手動で解消してください")
        }
        UiLocale::En => format!("warning: conflicts detected in {path}. Please resolve manually"),
    }
}

pub fn resume_merged(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: baseline を更新し shadow changes を merge しました"),
        UiLocale::En => format!("{path}: baseline updated and shadow changes merged"),
    }
}

pub fn resume_phantom_restored(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: phantom file を復元しました"),
        UiLocale::En => format!("{path}: phantom file restored"),
    }
}

pub fn rebase_no_overlay_files(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "overlay ファイルはありません",
        "no overlay files found",
    )
}

pub fn rebase_commit_ref_updated(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: baseline の内容は同じでした (commit ref を更新)"),
        UiLocale::En => format!("{path}: baseline content unchanged (commit ref updated)"),
    }
}

pub fn rebase_conflicts(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => {
            format!("warning: {path} で conflict が発生しました。手動で解消してください")
        }
        UiLocale::En => format!("warning: conflicts detected in {path}. Please resolve manually"),
    }
}

pub fn rebase_updated(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} の baseline を更新しました"),
        UiLocale::En => format!("baseline updated for {path}"),
    }
}

pub fn baseline_outdated_warning(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "warning: {path} の baseline は古くなっています。`git-shadow rebase {path}` を実行してください"
        ),
        UiLocale::En => format!(
            "warning: baseline for {path} is outdated. Run `git-shadow rebase {path}`"
        ),
    }
}

pub fn stale_lock_recovered(locale: UiLocale, pid: u32) -> String {
    match locale {
        UiLocale::Ja => {
            format!("warning: stale lock (PID {pid}) を安全に回復しました。commit を続行します")
        }
        UiLocale::En => {
            format!("warning: recovered stale lock from PID {pid} safely; continuing commit")
        }
    }
}

pub fn pre_commit_overlay_staged_warning(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "warning: `{path}` の local-only changes は現在 stage されていますが、commit 時に取り除かれます"
        ),
        UiLocale::En => format!(
            "warning: local-only changes in `{path}` are staged right now, but will be stripped before commit"
        ),
    }
}

pub fn auto_rebase_commit_ref_updated(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: baseline の commit ref を自動更新しました"),
        UiLocale::En => format!("{path}: auto-updated baseline commit reference"),
    }
}

pub fn auto_rebase_updated(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: baseline を自動更新して shadow changes を再適用しました"),
        UiLocale::En => {
            format!("{path}: auto-updated baseline and re-applied shadow changes")
        }
    }
}

pub fn auto_rebase_conflict_warning(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "warning: {path} は自動 rebase で conflict しそうなので作業ツリーは変更しません。`git-shadow rebase {path}` を実行してください"
        ),
        UiLocale::En => format!(
            "warning: auto-rebase for {path} would conflict, so the working tree was left untouched. Run `git-shadow rebase {path}`"
        ),
    }
}

pub fn auto_rebase_skipped_locked(locale: UiLocale, trigger: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "warning: {trigger} の自動 rebase をスキップしました (別の git-shadow 処理が lock を保持しています)。必要なら後で `git-shadow rebase` を実行してください"
        ),
        UiLocale::En => format!(
            "warning: skipped {trigger} auto-rebase because another git-shadow process holds the lock. Run `git-shadow rebase` later if needed"
        ),
    }
}

pub fn auto_rebase_failed(locale: UiLocale, path: &str, trigger: &str, error: &str) -> String {
    match locale {
        UiLocale::Ja => {
            format!("warning: {trigger} で {path} の自動 rebase に失敗しました: {error}")
        }
        UiLocale::En => {
            format!("warning: {trigger} auto-rebase failed for {path}: {error}")
        }
    }
}

pub fn post_commit_restore_failed(locale: UiLocale, path: &str, error: &str) -> String {
    match locale {
        UiLocale::Ja => format!("warning: {path} の復元に失敗しました: {error}"),
        UiLocale::En => format!("warning: failed to restore {path}: {error}"),
    }
}

pub fn post_commit_read_stash_failed(locale: UiLocale, path: &str, error: &str) -> String {
    match locale {
        UiLocale::Ja => format!("warning: {path} の stash 読み込みに失敗しました: {error}"),
        UiLocale::En => format!("warning: failed to read stash for {path}: {error}"),
    }
}

pub fn post_commit_restore_conflict(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "warning: {path} は commit 後に編集されているため上書きしませんでした。`git-shadow restore` で確認してください"
        ),
        UiLocale::En => format!(
            "warning: {path} was edited after the commit, so it was left untouched. Run `git-shadow restore` to review"
        ),
    }
}

pub fn post_commit_partial_failure(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "warning: 復元できなかったファイルがあります。`git-shadow restore` を実行してください",
        "warning: some files could not be restored. Run `git-shadow restore`",
    )
}

pub fn doctor_all_checks_passed(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "すべてのチェックを通過しました",
        "all checks passed",
    )
}

pub fn doctor_issues_heading(locale: UiLocale) -> &'static str {
    choose(locale, "issues:", "issues:")
}

pub fn doctor_warnings_heading(locale: UiLocale) -> &'static str {
    choose(locale, "warnings:", "warnings:")
}

pub fn doctor_hook_missing(locale: UiLocale, hook: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{hook} hook が存在しません"),
        UiLocale::En => format!("{hook} hook does not exist"),
    }
}

pub fn doctor_hook_not_executable(locale: UiLocale, hook: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{hook} hook に実行権限がありません"),
        UiLocale::En => format!("{hook} hook is not executable"),
    }
}

pub fn doctor_hooks_inert(locale: UiLocale, hooks_path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "hooks は既定の hooks dir にありますが core.hooksPath ({hooks_path}) が別を指しているため実行されません。`git-shadow install` を再実行してください"
        ),
        UiLocale::En => format!(
            "hooks are installed in the default hooks dir but core.hooksPath ({hooks_path}) points elsewhere, so they never run. Re-run `git-shadow install`"
        ),
    }
}

pub fn doctor_hook_not_calling_shadow(locale: UiLocale, hook: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{hook} hook が git-shadow を呼び出していません"),
        UiLocale::En => format!("{hook} hook does not call git-shadow"),
    }
}

pub fn doctor_competing_hook_manager(locale: UiLocale, marker: &str) -> String {
    match locale {
        UiLocale::Ja => format!("競合する hook manager を検出しました: {marker}"),
        UiLocale::En => format!("competing hook manager detected: {marker}"),
    }
}

pub fn doctor_overlay_missing_worktree(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} が working tree に存在しません"),
        UiLocale::En => format!("{path} does not exist in working tree"),
    }
}

pub fn doctor_baseline_missing(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} の baseline file が存在しません"),
        UiLocale::En => format!("baseline file for {path} does not exist"),
    }
}

pub fn doctor_phantom_dir_missing(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} (phantom dir) が working tree に存在しません"),
        UiLocale::En => format!("{path} (phantom dir) does not exist in working tree"),
    }
}

pub fn doctor_phantom_missing(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path} (phantom) が working tree に存在しません"),
        UiLocale::En => format!("{path} (phantom) does not exist in working tree"),
    }
}

pub fn doctor_stash_remaining(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "stash に残りファイルがあります。`git-shadow restore` を実行してください",
        "stash has remaining files. Run `git-shadow restore`",
    )
}

pub fn doctor_suspended(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "shadow changes は suspend 中です。`git-shadow resume` を実行してください",
        "shadow changes are suspended. Run `git-shadow resume`",
    )
}

pub fn doctor_suspended_dir_missing(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "suspended directory がありません (状態が壊れている可能性があります)",
        "suspended directory is missing (state may be corrupted)",
    )
}

pub fn doctor_worktree_not_initialized(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "worktree を検出しましたが、この worktree では git-shadow が未初期化です。`git-shadow install` を実行してください",
        "worktree detected but git-shadow is not initialized here. Run `git-shadow install` to set up this worktree",
    )
}

pub fn doctor_worktree_no_config(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "worktree を検出しましたが shadow config がありません。`git-shadow add <file>` でこの worktree のファイルを登録してください",
        "worktree detected but no shadow config found. Run `git-shadow add <file>` to register files in this worktree",
    )
}

pub fn doctor_stale_lock(locale: UiLocale, pid: u32) -> String {
    match locale {
        UiLocale::Ja => format!(
            "stale lockfile を検出しました (PID {pid})。`git-shadow restore` を実行してください"
        ),
        UiLocale::En => format!("stale lockfile detected (PID {pid}). Run `git-shadow restore`"),
    }
}

pub fn doctor_lock_held(locale: UiLocale, pid: u32) -> String {
    match locale {
        UiLocale::Ja => format!("lockfile は別プロセス (PID {pid}) が保持しています"),
        UiLocale::En => format!("lockfile is held by another process (PID {pid})"),
    }
}

pub fn status_warning_stash_remaining(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "  warning: stash に残りファイルがあります (前回の commit が中断された可能性があります)",
        "  warning: stash has remaining files (a previous commit may have been interrupted)",
    )
}

pub fn status_warning_stale_lock(locale: UiLocale, pid: u32) -> String {
    match locale {
        UiLocale::Ja => {
            format!("  warning: stale lockfile を検出しました (PID {pid} は既に存在しません)")
        }
        UiLocale::En => {
            format!("  warning: stale lockfile detected (PID {pid} no longer exists)")
        }
    }
}

pub fn status_action_run_restore(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    -> `git-shadow restore` を実行",
        "    -> Run `git-shadow restore`",
    )
}

pub fn status_suspended(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "  status: SUSPENDED (`git-shadow resume` で shadow changes を戻します)",
        "  status: SUSPENDED (run `git-shadow resume` to restore shadow changes)",
    )
}

pub fn status_heading_managed_files(locale: UiLocale) -> &'static str {
    choose(locale, "managed files:", "managed files:")
}

pub fn status_overlay_local_only(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    local-only: このファイルの変更は working tree には残りますが commit には入りません",
        "    local-only: changes in this file stay in your working tree but are not committed",
    )
}

pub fn label_overlay(locale: UiLocale) -> &'static str {
    choose(locale, "overlay", "overlay")
}

pub fn label_phantom(locale: UiLocale) -> &'static str {
    choose(locale, "phantom", "phantom")
}

pub fn label_phantom_dir(locale: UiLocale) -> &'static str {
    choose(locale, "phantom dir", "phantom dir")
}

pub fn status_baseline(locale: UiLocale, commit: &str) -> String {
    match locale {
        UiLocale::Ja => format!("    baseline: {commit}"),
        UiLocale::En => format!("    baseline: {commit}"),
    }
}

pub fn status_overlay_git_state(locale: UiLocale, state: &str) -> String {
    match locale {
        UiLocale::Ja => format!("    git state: {state}"),
        UiLocale::En => format!("    git state: {state}"),
    }
}

pub fn status_overlay_staged_warning(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    warning: stage されている local-only changes は commit 前に取り除かれます",
        "    warning: staged local-only changes will be stripped before commit",
    )
}

pub fn status_warning_file_missing_worktree(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    warning: file が working tree に存在しません",
        "    warning: file does not exist in working tree",
    )
}

pub fn status_shadow_changes(locale: UiLocale, added: usize, removed: usize) -> String {
    match locale {
        UiLocale::Ja => format!("    shadow changes: +{added} 行 / -{removed} 行"),
        UiLocale::En => format!("    shadow changes: +{added} lines / -{removed} lines"),
    }
}

pub fn status_warning_baseline_outdated(
    locale: UiLocale,
    old_commit: &str,
    new_commit: &str,
) -> String {
    match locale {
        UiLocale::Ja => {
            format!("    warning: baseline が古くなっています ({old_commit} -> {new_commit})")
        }
        UiLocale::En => {
            format!("    warning: baseline is outdated ({old_commit} -> {new_commit})")
        }
    }
}

pub fn status_action_run_rebase(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("    -> `git-shadow rebase {path}` を実行"),
        UiLocale::En => format!("    -> Run `git-shadow rebase {path}`"),
    }
}

pub fn status_exclude_git_info(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    exclude: .git/info/exclude",
        "    exclude: .git/info/exclude",
    )
}

pub fn status_exclude_none(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    exclude: なし (hook 保護のみ)",
        "    exclude: none (hook protection only)",
    )
}

pub fn status_phantom_dir_explainer(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    local-only directory: Git には含まれず、git-shadow が存在だけを保護します",
        "    local-only directory: not committed to Git; git-shadow only protects its presence",
    )
}

pub fn status_contents(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("    contents: {count} entries"),
        UiLocale::En => format!("    contents: {count} entries"),
    }
}

pub fn status_warning_directory_missing(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    warning: directory が存在しません",
        "    warning: directory does not exist",
    )
}

pub fn status_warning_file_missing(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "    warning: file が存在しません",
        "    warning: file does not exist",
    )
}

pub fn status_file_size(locale: UiLocale, size: &str) -> String {
    match locale {
        UiLocale::Ja => format!("    file size: {size}"),
        UiLocale::En => format!("    file size: {size}"),
    }
}

pub fn status_git_wrapper_hint(locale: UiLocale) -> &'static str {
    choose(
        locale,
        "tip: 普段使いには `git shadow status --git` を shell alias にすると扱いやすいです",
        "tip: for daily use, consider a shell alias for `git shadow status --git`",
    )
}

pub fn export_success(locale: UiLocale, count: usize, output: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{count} 件の管理対象を `{output}` に export しました"),
        UiLocale::En => format!("exported {count} managed file(s) to `{output}`"),
    }
}

pub fn import_success(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("{count} 件のファイルを import しました"),
        UiLocale::En => format!("imported {count} file(s)"),
    }
}

pub fn import_summary(locale: UiLocale, imported: usize, skipped: usize) -> String {
    match locale {
        UiLocale::Ja => format!("import 完了: {imported} 件成功 / {skipped} 件スキップ"),
        UiLocale::En => format!("import finished: {imported} imported, {skipped} skipped"),
    }
}

pub fn import_imported_overlay(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: overlay を import しました"),
        UiLocale::En => format!("{path}: imported overlay"),
    }
}

pub fn import_merged_overlay(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: overlay を import し upstream の変更と merge しました"),
        UiLocale::En => format!("{path}: imported overlay and merged with upstream changes"),
    }
}

pub fn import_imported_phantom(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: phantom を import しました"),
        UiLocale::En => format!("{path}: imported phantom"),
    }
}

pub fn import_imported_phantom_dir(locale: UiLocale, path: &str, count: usize) -> String {
    match locale {
        UiLocale::Ja => format!("{path}: phantom directory を import しました ({count} entries)"),
        UiLocale::En => format!("{path}: imported phantom directory ({count} entries)"),
    }
}

pub fn import_skip_overlay_untracked(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "skip {path}: HEAD に追跡されていません (このリポジトリは export 元と一致しません)"
        ),
        UiLocale::En => {
            format!("skip {path}: not tracked in HEAD (this repository does not match the export)")
        }
    }
}

pub fn import_skip_overlay_conflict(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "skip {path}: upstream の変更と衝突しました。手動で解消してから再度 import してください (`--force` で shadow を優先)"
        ),
        UiLocale::En => format!(
            "skip {path}: conflicts with upstream changes. Resolve manually and re-import (use `--force` to keep the shadow version)"
        ),
    }
}

pub fn import_skip_overlay_worktree_modified(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!(
            "skip {path}: working tree に import 由来でないローカル変更があります (`--force` で上書き)"
        ),
        UiLocale::En => format!(
            "skip {path}: the working tree has local modifications that did not come from this import (use `--force` to overwrite)"
        ),
    }
}

pub fn import_skip_phantom_conflict(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("skip {path}: 既に存在し内容が異なります (`--force` で上書き)"),
        UiLocale::En => format!(
            "skip {path}: already exists with different content (use `--force` to overwrite)"
        ),
    }
}

pub fn import_skip_already_managed(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => {
            format!("skip {path}: 既に別の種類として管理されています (`--force` で置き換え)")
        }
        UiLocale::En => {
            format!("skip {path}: already managed with a different type (use `--force` to replace)")
        }
    }
}

pub fn import_missing_content(locale: UiLocale, path: &str) -> String {
    match locale {
        UiLocale::Ja => format!("skip {path}: archive に内容が含まれていません"),
        UiLocale::En => format!("skip {path}: archive is missing its content"),
    }
}

fn choose(locale: UiLocale, ja: &'static str, en: &'static str) -> &'static str {
    match locale {
        UiLocale::Ja => ja,
        UiLocale::En => en,
    }
}

fn find_shadow_error(err: &anyhow::Error) -> Option<&ShadowError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<ShadowError>())
}

fn format_shadow_error(err: &ShadowError, locale: UiLocale) -> String {
    match locale {
        UiLocale::Ja => format_shadow_error_ja(err),
        UiLocale::En => format_shadow_error_en(err),
    }
}

fn format_shadow_error_ja(err: &ShadowError) -> String {
    match err {
        ShadowError::NotAGitRepo => "エラー: Git リポジトリではありません".to_string(),
        ShadowError::PathDoesNotExist(path) => {
            format!("エラー: `{path}` は存在しません")
        }
        ShadowError::FileNotTracked(path) => {
            format!("エラー: `{path}` は Git に追跡されていません")
        }
        ShadowError::TrackedFileNeedsOverlay(path) => format!(
            "エラー: `{path}` は既に Git に追跡されています。`--phantom` を外して overlay として登録してください"
        ),
        ShadowError::AlreadyManaged(path) => {
            format!("エラー: `{path}` は既に git-shadow の管理対象です")
        }
        ShadowError::NotManaged(path) => {
            format!("エラー: `{path}` は git-shadow の管理対象ではありません")
        }
        ShadowError::BinaryFile(path) => {
            format!("エラー: `{path}` はバイナリファイルです")
        }
        ShadowError::FileTooLarge(path, size, limit) => format!(
            "エラー: `{path}` がサイズ制限を超えています ({size} bytes > {limit} bytes)。`--force` を使ってください"
        ),
        ShadowError::PartialStage(file) => format!(
            "コミットを止めました: shadow 管理中の `{file}` が部分的に stage されています。\n\
             対処:\n\
             1. `git add {file}` でファイル全体を stage し直す\n\
             2. もう一度 `git commit` する"
        ),
        ShadowError::Suspended => "\
コミットを止めました: shadow changes が suspend されたままです。\n\
対処:\n\
1. `git-shadow resume` を実行して shadow changes を戻す\n\
2. 必要なら内容を確認してから `git commit` をやり直す"
            .to_string(),
        ShadowError::StashRemaining => "\
コミットを止めました: 前回の commit 処理の残骸が `.git/shadow/stash/` に残っています。\n\
対処:\n\
1. `git-shadow restore` を実行して stash と lock を回復する\n\
2. `git status` で作業ツリーを確認する\n\
3. もう一度 `git commit` する"
            .to_string(),
        ShadowError::BaselineMissing(file) => format!(
            "コミットを止めました: `{file}` の baseline が見つかりません。\n\
             対処:\n\
             1. `{file}` の現在内容を残したいなら先に別の場所へ退避する\n\
             2. `git-shadow remove --force {file}` で登録を外す\n\
             3. `git-shadow add {file}` で登録し直し、必要なら local changes を戻す"
        ),
        ShadowError::FileMissing(file) => format!(
            "コミットを止めました: overlay 管理中の `{file}` が作業ツリーにありません。\n\
             対処:\n\
             1. 誤って消したなら `git restore --source=HEAD -- {file}` で戻す\n\
             2. もう管理しないなら `git-shadow remove --force {file}` を実行する\n\
             3. その後でもう一度 `git commit` する"
        ),
        ShadowError::UnstageFailure(file) => format!(
            "コミットを止めました: phantom の `{file}` を index から外せませんでした。\n\
             対処:\n\
             1. `git reset -- {file}` を実行する\n\
             2. `git status` で stage 状態を確認する\n\
             3. もう一度 `git commit` する"
        ),
        ShadowError::LockHeld { pid, timestamp } => format!(
            "コミットを止めました: 別の git-shadow 処理が lock を保持しています。\n\
             詳細: PID {pid}, started: {timestamp}\n\
             対処:\n\
             1. その commit / hook 処理が終わるまで待つ\n\
             2. もし既に止まっているはずなら `git-shadow restore` を実行する\n\
             3. その後で `git commit` をやり直す"
        ),
        ShadowError::StaleLock(pid) => format!(
            "コミットを止めました: stale lock が残っています。\n\
             詳細: PID {pid} は既に存在しません。\n\
             対処:\n\
             1. `git-shadow restore` を実行して lock を片付ける\n\
             2. `git status` を確認する\n\
             3. もう一度 `git commit` する"
        ),
        ShadowError::CorruptLock => "\
コミットを止めました: lockfile が壊れています (内容を解釈できません)。\n\
対処:\n\
1. `git-shadow restore` を実行して lock を片付ける\n\
2. `git status` を確認する\n\
3. もう一度 `git commit` する"
            .to_string(),
        ShadowError::MergeFailed(stderr) => {
            format!("エラー: 3-way merge に失敗しました:\n{stderr}")
        }
        ShadowError::AutoRestoreConflict(file) => format!(
            "コミットを止めました: stale lock の自動回復で `{file}` を上書きする可能性があります。\n\
             対処:\n\
             1. `{file}` の作業内容を確認する\n\
             2. 必要なら退避してから `git-shadow restore` を実行する\n\
             3. その後で `git commit` をやり直す"
        ),
        ShadowError::ResumeEditConflict(file) => format!(
            "resume を止めました: suspend 中に編集された `{file}` を上書きする可能性があります。\n\
             対処:\n\
             1. `{file}` の現在内容を確認する\n\
             2. 残したい内容を退避する\n\
             3. `.git/shadow/suspended/` の内容と統合してから再度 `git-shadow resume` を実行する"
        ),
        ShadowError::NotInitialized => "\
エラー: git-shadow がまだ初期化されていません。\n\
対処:\n\
1. リポジトリで `git-shadow install` を実行する\n\
2. その後に `git-shadow add ...` や `git commit` をやり直す"
            .to_string(),
        ShadowError::AddSomeFailed(count) => {
            format!("エラー: {count} 件のパス登録に失敗しました。上のエラーを確認してください")
        }
        ShadowError::AlreadySuspended => {
            "エラー: shadow changes は既に suspend されています".to_string()
        }
        ShadowError::NotSuspended => {
            "エラー: shadow changes は suspend されていません".to_string()
        }
        ShadowError::HooksNotInstalled => {
            "エラー: hooks が未インストールです。`git-shadow install` を実行してください"
                .to_string()
        }
        ShadowError::UninstallHasEntries(count) => format!(
            "エラー: {count} 件のファイルがまだ git-shadow の管理対象です。\n\
             対処:\n\
             1. `git-shadow remove <file>` で個別に解除する\n\
             2. または `git-shadow uninstall --force` で overlay を復元して state を削除する"
        ),
        ShadowError::DoctorFoundIssues(count) => {
            format!("エラー: doctor が {count} 件の問題を検出しました")
        }
        ShadowError::NothingToExport => {
            "エラー: export 対象がありません。git-shadow が管理しているファイルはありません"
                .to_string()
        }
        ShadowError::ExportFileExists(path) => format!(
            "エラー: 出力先 `{path}` が既に存在します。上書きするには `--force` を使ってください"
        ),
        ShadowError::UnsupportedExportVersion(version) => format!(
            "エラー: 未対応の export フォーマット version ({version}) です。git-shadow を更新してください"
        ),
        ShadowError::ImportSomeSkipped(count) => format!(
            "エラー: {count} 件のファイルを import できませんでした。上のメッセージを確認してください"
        ),
        ShadowError::NonInteractiveWithoutForce => {
            "エラー: 非対話モードでは `--force` が必要です".to_string()
        }
        ShadowError::GitCommand { command, stderr } => {
            format!("エラー: Git コマンドに失敗しました: {command}\n{stderr}")
        }
        _ => format!("エラー: {}", err),
    }
}

fn format_shadow_error_en(err: &ShadowError) -> String {
    match err {
        ShadowError::NotAGitRepo => "Error: not a Git repository".to_string(),
        ShadowError::PathDoesNotExist(path) => {
            format!("Error: `{path}` does not exist")
        }
        ShadowError::FileNotTracked(path) => {
            format!("Error: `{path}` is not tracked by Git")
        }
        ShadowError::TrackedFileNeedsOverlay(path) => format!(
            "Error: `{path}` is already tracked by Git. Remove `--phantom` to register it as overlay"
        ),
        ShadowError::AlreadyManaged(path) => {
            format!("Error: `{path}` is already managed by git-shadow")
        }
        ShadowError::NotManaged(path) => {
            format!("Error: `{path}` is not managed by git-shadow")
        }
        ShadowError::BinaryFile(path) => format!("Error: `{path}` is a binary file"),
        ShadowError::FileTooLarge(path, size, limit) => format!(
            "Error: `{path}` exceeds the size limit ({size} bytes > {limit} bytes). Use `--force` to override"
        ),
        ShadowError::PartialStage(file) => format!(
            "Commit blocked: `{file}` is partially staged while managed by git-shadow.\n\
             What to do:\n\
             1. Run `git add {file}` to stage the whole file\n\
             2. Run `git commit` again"
        ),
        ShadowError::Suspended => "\
Commit blocked: shadow changes are still suspended.\n\
What to do:\n\
1. Run `git-shadow resume`\n\
2. Review the restored changes if needed\n\
3. Run `git commit` again"
            .to_string(),
        ShadowError::StashRemaining => "\
Commit blocked: leftover files remain in `.git/shadow/stash/` from an earlier interrupted commit.\n\
What to do:\n\
1. Run `git-shadow restore`\n\
2. Check `git status`\n\
3. Run `git commit` again"
            .to_string(),
        ShadowError::BaselineMissing(file) => format!(
            "Commit blocked: the baseline for `{file}` is missing.\n\
             What to do:\n\
             1. Save a copy first if you need the current contents\n\
             2. Run `git-shadow remove --force {file}`\n\
             3. Run `git-shadow add {file}` and re-apply your local-only edits if needed"
        ),
        ShadowError::FileMissing(file) => format!(
            "Commit blocked: overlay-managed file `{file}` is missing from the working tree.\n\
             What to do:\n\
             1. If you deleted it by accident, run `git restore --source=HEAD -- {file}`\n\
             2. If you no longer want it managed, run `git-shadow remove --force {file}`\n\
             3. Run `git commit` again"
        ),
        ShadowError::UnstageFailure(file) => format!(
            "Commit blocked: git-shadow could not unstage phantom file `{file}`.\n\
             What to do:\n\
             1. Run `git reset -- {file}`\n\
             2. Check `git status`\n\
             3. Run `git commit` again"
        ),
        ShadowError::LockHeld { pid, timestamp } => format!(
            "Commit blocked: another git-shadow process still holds the lock.\n\
             Details: PID {pid}, started: {timestamp}\n\
             What to do:\n\
             1. Wait for the other commit or hook to finish\n\
             2. If it should already be done, run `git-shadow restore`\n\
             3. Run `git commit` again"
        ),
        ShadowError::StaleLock(pid) => format!(
            "Commit blocked: a stale lock was found.\n\
             Details: PID {pid} no longer exists.\n\
             What to do:\n\
             1. Run `git-shadow restore`\n\
             2. Check `git status`\n\
             3. Run `git commit` again"
        ),
        ShadowError::CorruptLock => "\
Commit blocked: the lockfile is corrupted (its contents could not be parsed).\n\
What to do:\n\
1. Run `git-shadow restore`\n\
2. Check `git status`\n\
3. Run `git commit` again"
            .to_string(),
        ShadowError::MergeFailed(stderr) => {
            format!("Error: 3-way merge failed:\n{stderr}")
        }
        ShadowError::AutoRestoreConflict(file) => format!(
            "Commit blocked: automatic stale-lock recovery would overwrite newer working tree content in `{file}`.\n\
             What to do:\n\
             1. Review the current contents of `{file}`\n\
             2. Save anything you need, then run `git-shadow restore`\n\
             3. Run `git commit` again"
        ),
        ShadowError::ResumeEditConflict(file) => format!(
            "Resume blocked: `{file}` was edited in the working tree while suspended, and resume would overwrite it.\n\
             What to do:\n\
             1. Review the current contents of `{file}`\n\
             2. Save anything you want to keep\n\
             3. Reconcile with `.git/shadow/suspended/`, then run `git-shadow resume` again"
        ),
        ShadowError::NotInitialized => "\
Error: git-shadow is not initialized yet.\n\
What to do:\n\
1. Run `git-shadow install` in this repository\n\
2. Retry `git-shadow add ...` or `git commit`"
            .to_string(),
        ShadowError::AddSomeFailed(count) => {
            format!("Error: failed to register {count} path(s); see the errors above")
        }
        ShadowError::AlreadySuspended => {
            "Error: shadow changes are already suspended".to_string()
        }
        ShadowError::NotSuspended => "Error: shadow changes are not suspended".to_string(),
        ShadowError::HooksNotInstalled => {
            "Error: hooks not installed. Run `git-shadow install`".to_string()
        }
        ShadowError::UninstallHasEntries(count) => format!(
            "Error: {count} file(s) are still managed by git-shadow.\n\
             What to do:\n\
             1. Run `git-shadow remove <file>` for each file\n\
             2. Or run `git-shadow uninstall --force` to restore overlays and wipe state"
        ),
        ShadowError::DoctorFoundIssues(count) => {
            format!("Error: doctor found {count} issue(s)")
        }
        ShadowError::NothingToExport => {
            "Error: nothing to export; no files are managed by git-shadow".to_string()
        }
        ShadowError::ExportFileExists(path) => {
            format!("Error: output file `{path}` already exists. Use `--force` to overwrite")
        }
        ShadowError::UnsupportedExportVersion(version) => format!(
            "Error: unsupported export format version {version}; upgrade git-shadow"
        ),
        ShadowError::ImportSomeSkipped(count) => format!(
            "Error: {count} file(s) could not be imported; see the messages above"
        ),
        ShadowError::NonInteractiveWithoutForce => {
            "Error: `--force` is required in non-interactive mode".to_string()
        }
        ShadowError::GitCommand { command, stderr } => {
            format!("Error: git command failed: {command}\n{stderr}")
        }
        _ => format!("Error: {}", err),
    }
}

fn detect_locale_from_values(values: impl IntoIterator<Item = String>) -> UiLocale {
    for value in values {
        let normalized = value.to_ascii_lowercase();
        if normalized.starts_with("ja") {
            return UiLocale::Ja;
        }
        if normalized.starts_with("en") {
            return UiLocale::En;
        }
    }

    UiLocale::En
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_locale_defaults_to_en() {
        let locale = detect_locale_from_values(Vec::<String>::new());
        assert_eq!(locale, UiLocale::En);
    }

    #[test]
    fn test_detect_locale_prefers_ja() {
        let locale = detect_locale_from_values(vec!["ja_JP.UTF-8".to_string()]);
        assert_eq!(locale, UiLocale::Ja);
    }

    #[test]
    fn test_format_error_ja_for_partial_stage() {
        let err = anyhow::Error::new(ShadowError::PartialStage("tracked.txt".to_string()));
        let message = format_error(&err, UiLocale::Ja);
        assert!(message.contains("コミットを止めました"));
        assert!(message.contains("git add tracked.txt"));
    }

    #[test]
    fn test_format_error_en_for_stash_remaining() {
        let err = anyhow::Error::new(ShadowError::StashRemaining);
        let message = format_error(&err, UiLocale::En);
        assert!(message.contains("Commit blocked"));
        assert!(message.contains("git-shadow restore"));
    }
}
