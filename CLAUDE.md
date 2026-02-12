# CLAUDE.md

## Project Overview

**git-shadow** is a Rust CLI tool that manages local-only changes in Git repositories. It lets users maintain personal edits (e.g., debug settings, local config overrides, private notes) that stay active in the working tree but are automatically stripped before each commit, keeping Git history clean.

- **Requirements spec**: `docs/requirements.md` (v4, Japanese)
- **Detailed usage**: `docs/usage.md` (English), `docs/usage.ja.md` (Japanese)

## Key Concepts

| Type | Description |
|------|-------------|
| **overlay** | Layer local changes on top of an existing tracked file |
| **phantom** | A file that exists only locally and is never committed |
| **phantom dir** | A directory that exists only locally (exclude-only management, no stash/restore) |

## Architecture

### Design Decisions

- **No git2 crate** -- Uses `std::process::Command` to shell out to git directly. Reasons: simpler builds (no libgit2/C dependency), better staging API coverage, debuggable by running the same commands manually. File count is small (1-10) so subprocess overhead is negligible.
- **Atomic writes** -- All file mutations use tempfile + rename via `fs_util::atomic_write()` to prevent corruption.
- **PID-based lockfile** -- Uses `libc::kill(pid, 0)` for stale lock detection.
- **PreCommitTransaction pattern** -- The pre-commit hook tracks state for rollback on failure.

### Module Structure

```
src/
  main.rs              # Entry point
  lib.rs               # Public modules (for integration tests)
  cli.rs               # clap derive structs (Commands enum)
  error.rs             # ShadowError (thiserror)
  config.rs            # ShadowConfig, FileEntry, FileType, ExcludeMode
  path.rs              # Path normalization + URL encoding (%25 before %2F)
  lock.rs              # Lockfile acquire/release/stale detection
  fs_util.rs           # Atomic write, binary detection, size check
  git.rs               # GitRepo struct wrapping git commands
  exclude.rs           # .git/info/exclude section management
  diff_util.rs         # Unified diff formatting (similar crate)
  merge.rs             # 3-way merge via `git merge-file -p --diff3`
  commands/
    install.rs         # Set up hooks + .git/shadow/ structure
    add.rs             # Register overlay or phantom
    remove.rs          # Unregister with confirmation prompt
    status.rs          # Show managed files, warnings
    diff.rs            # Show shadow changes as unified diff
    rebase.rs          # Update baseline with 3-way merge
    restore.rs         # Recover from interrupted commits
    suspend.rs         # Suspend shadow changes for branch switching
    resume.rs          # Resume suspended changes (with 3-way merge)
    doctor.rs          # Diagnose hooks, config, stale state
    hook.rs            # Dispatcher for `git-shadow hook <name>`
  hooks/
    pre_commit.rs      # Stash shadow -> restore baseline -> stage
    post_commit.rs     # Restore shadow from stash -> release lock
    post_merge.rs      # Detect baseline drift, warn user
tests/
  common/mod.rs        # TestRepo helper
  test_commit_cycle.rs # E2E: overlay cycle, phantom cycle, rollback
```

### Path Encoding

Nested paths are URL-encoded for flat storage in `baselines/` and `stash/`:
- Encode order: `%` -> `%25` first, then `/` -> `%2F`
- Decode order: `%2F` -> `/` first, then `%25` -> `%`
- Roundtrip: `decode(encode(path)) == path`

## Development

### Build & Test

```bash
cargo build
cargo test                      # 176 tests (171 unit + 5 E2E)
cargo clippy -- -D warnings     # Must pass with zero warnings
cargo fmt --check               # Must pass
```

### CI

GitHub Actions runs on every push to `main` and on pull requests:
- **fmt** -- `cargo fmt --check`
- **clippy** -- `cargo clippy -- -D warnings`
- **test** -- `cargo test`

Configuration: `.github/workflows/ci.yml`

### Pre-commit Hook

A development pre-commit hook is provided in `dev-hooks/`. To enable it:

```bash
git config core.hooksPath dev-hooks
```

This runs `cargo fmt --check` and `cargo clippy -- -D warnings` before each commit.

### TDD Pattern

All features were developed test-first. Unit tests use `*_for_test` helper functions to bypass `std::env::current_dir()` dependency in `GitRepo::discover()`. Each test creates an isolated git repo via `tempfile::tempdir()`.

### Common Clippy Issues

- `useless_format!`: Use `"literal".to_string()` instead of `format!("literal")`
- `single_match`: Use `if let` instead of `match` with one arm + wildcard
- `new_without_default`: Add `Default` impl when defining `new()`

## Git Workflow

- **Branch protection**: Direct pushes to `main` are blocked (including admins). All changes must go through pull requests.
- **Commit style**: Conventional commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`)

## Language Policy

- **Source code**: All messages, errors, CLI help text, and comments are in English.
- **Documentation**: English primary (`README.md`, `docs/usage.md`) with Japanese translations (`README.ja.md`, `docs/usage.ja.md`). Cross-links between language versions.
- **Requirements spec**: `docs/requirements.md` remains in Japanese (original spec).
