# src/

Root source directory. Modules are split by responsibility into focused, small files.

## Module Roles

| Module | Responsibility | Key Types |
|--------|---------------|-----------|
| `error.rs` | All error types via `thiserror` | `ShadowError` enum |
| `config.rs` | JSON config load/save, file registry | `ShadowConfig`, `FileEntry`, `FileType`, `ExcludeMode` |
| `path.rs` | Path normalization + URL encoding for flat storage | `normalize_path()`, `encode_path()`, `decode_path()` |
| `lock.rs` | PID-based lockfile for concurrency safety | `LockStatus` (`Free`/`HeldByOther`/`Stale`/`Corrupt`), `acquire_lock()`, `release_lock()`, `check_lock()` |
| `fs_util.rs` | Atomic writes, binary detection, size checks | `atomic_write()`, `is_binary()`, `check_size()` |
| `git.rs` | Worktree-aware Git CLI wrapper (no git2 crate) | `GitRepo` (root, git_dir, common_dir, shadow_dir; `hooks_dir()`, `effective_hooks_dir()`) |
| `exclude.rs` | `.git/info/exclude` section management | `ExcludeManager` |
| `diff_util.rs` | Unified diff formatting with colors | `unified_diff()`, `print_colored_diff()` |
| `merge.rs` | 3-way merge via `git merge-file -p --diff3` | `three_way_merge()`, `MergeResult` |
| `ui.rs` | Locale (ja/en) detection + user-facing message/error formatting | `UiLocale`, `detect_locale()`, `format_error()` |
| `cli.rs` | clap derive definitions | `Cli`, `Commands` enum |
| `main.rs` | Entry point, dispatches to commands | - |
| `lib.rs` | Re-exports all modules for integration tests | - |

## Design Notes

### Error Strategy

Two error libraries are used intentionally:
- **`thiserror`** (`ShadowError`) -- For domain errors that callers match on (e.g., `PartialStage`, `StaleLock`, `FileMissing`). These carry structured data and produce user-facing messages.
- **`anyhow`** -- For plumbing errors with `.context()` (I/O failures, parse errors). These bubble up and are displayed as-is.

### GitRepo: Why Shell Out Instead of git2

`git.rs` wraps `std::process::Command` calls to the `git` binary. The `git2` crate was rejected because:
1. It requires libgit2 (C library), complicating builds
2. Staging operations (`git add`, `git rm --cached`, `git restore --staged`) lack good git2 APIs
3. Users can reproduce and debug commands by running them manually
4. File count is small (1-10), so subprocess overhead is negligible

### GitRepo: Worktree-Aware Discovery

`GitRepo::discover()` uses `git rev-parse --path-format=absolute --show-toplevel --git-dir --git-common-dir` to resolve three paths:
- **root** -- worktree working directory (`--show-toplevel`)
- **git_dir** -- per-worktree Git directory (`--git-dir`, e.g., `.git/worktrees/<name>`)
- **common_dir** -- shared Git directory (`--git-common-dir`, e.g., `.git/`)

`shadow_dir` is derived from `git_dir` (per-worktree state). `hooks_dir()` returns `common_dir.join("hooks")` so hooks are shared across worktrees; `effective_hooks_dir()` honors `core.hooksPath` when it is set (resolving a relative value against `root`) and otherwise falls back to `hooks_dir()` -- install/uninstall/doctor use the effective dir so hooks land where git actually runs them. A `discover_fallback()` handles Git versions < 2.31 that lack `--git-common-dir`.

### Atomic Writes

All file mutations go through `fs_util::atomic_write()` which uses `tempfile::NamedTempFile` + `persist()` (rename). This prevents corruption if the process is killed mid-write. This is critical for baseline and stash files.

### Path Encoding

`path.rs` handles the encoding of `/` in file paths so that `baselines/` and `stash/` can use flat directory storage. The encoding order matters:
1. `%` -> `%25` **first** (escape the escape character)
2. `/` -> `%2F`

Decoding reverses the order. This guarantees `decode(encode(p)) == p` for any path.

### Lock Protocol

`lock.rs` uses a PID + timestamp file, written atomically with `O_CREAT | O_EXCL` so concurrent acquirers cannot both win. Stale detection uses `libc::kill(pid, 0)` (signal 0 = existence check without sending a signal). `check_lock()` reports `Free`, `HeldByOther`, `Stale` (dead PID), or `Corrupt` (unparseable) without erroring, so callers can decide; `acquire_lock()` treats a corrupt lock like a stale one but does not silently clobber a live one. The lock is acquired by pre-commit and released by post-commit. If post-commit never runs (e.g., `--no-verify`), the lock becomes stale and `restore` cleans it up.

### ExcludeManager

`exclude.rs` manages a delimited section in `.git/info/exclude` between marker comments. It preserves all content outside the section. When the last entry is removed, the section markers are also removed to keep the file clean.
