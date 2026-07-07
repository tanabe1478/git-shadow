# tests/

Integration and E2E tests. Unit tests live alongside their modules in `src/` via `#[cfg(test)]`.

## Structure

| File | Purpose |
|------|---------|
| `common/mod.rs` | `TestRepo` helper for creating isolated git repos |
| `test_commit_cycle.rs` | E2E tests for the full commit lifecycle (hook handlers driven in-process) |
| `test_worktree.rs` | E2E tests for git worktree support |
| `test_git_operations.rs` | E2E tests running real `git` commands against the installed hooks |
| `test_localized_errors.rs` | E2E tests for locale-aware output, `--version`, `--json`, `uninstall` |
| `test_export_import.rs` | E2E tests for `export`/`import`: clone roundtrip, upstream merge/conflict, binary, URL-encoded paths, refusals, idempotency, locale |

## TestRepo Helper

`common::TestRepo` creates a temporary git repository with:
- `git init` + user config
- Helper methods: `create_file()`, `create_dir()`, `read_file()`, `commit()`, `git_dir()`, `shadow_dir()`, `init_shadow()`, `add_worktree()`

Used by E2E tests to set up realistic scenarios without touching the real filesystem. `test_git_operations.rs` and `test_localized_errors.rs` do not use `TestRepo` -- they build their own repos and shell out to the installed `git-shadow` binary.

## E2E Tests (test_commit_cycle.rs)

Five scenarios covering the core commit lifecycle:

1. **`test_full_overlay_commit_cycle`**: install -> add overlay -> edit -> pre-commit -> commit -> post-commit -> verify (committed content = baseline, working tree = shadow)
2. **`test_full_phantom_commit_cycle`**: install -> add phantom -> stage -> pre-commit -> commit -> post-commit -> verify (phantom not in commit, restored to working tree)
3. **`test_pre_commit_rollback_on_error`**: Simulates stash remnant causing pre-commit to fail, verifies shadow content is preserved (not lost during rollback)
4. **`test_full_phantom_directory_commit_cycle`**: phantom-directory (exclude-only) lifecycle -- verifies the directory stays local and is never committed
5. **`test_mixed_overlay_and_phantom_directory`**: overlay and phantom directory managed together in one commit cycle

## E2E Tests (test_worktree.rs)

Four scenarios covering git worktree support:

1. **`test_discover_in_worktree`**: Creates a worktree, verifies `GitRepo::discover()` resolves correct `root`, `git_dir`, and `common_dir` paths
2. **`test_install_in_worktree`**: Installs hooks from a worktree, verifies hooks land in the effective (shared) hooks dir
3. **`test_overlay_commit_cycle_in_worktree`**: Full overlay commit cycle in a worktree -- install, add, edit, pre-commit, commit, post-commit -- verifying per-worktree shadow state isolation
4. **`test_install_inherits_config_from_main_worktree`**: Verifies that `install` in a worktree auto-inherits managed files from the main repo -- overlay baselines are regenerated from worktree HEAD, phantom entries are copied as-is

## E2E Tests (test_git_operations.rs)

Six scenarios that install the real hooks and run real `git` commands (the built binary is put on `PATH`):

1. **`test_amend_keeps_shadow_out_of_commit_and_intact_in_worktree`**: `git commit --amend` keeps shadow out of history and intact in the working tree
2. **`test_rebase_triggers_post_rewrite_and_preserves_shadow`**: `git rebase` fires post-rewrite and preserves shadow
3. **`test_merge_triggers_post_merge_and_preserves_shadow`**: `git merge` fires post-merge and preserves shadow
4. **`test_cherry_pick_does_not_leak_shadow`**: `git cherry-pick` does not leak shadow content into the picked commit
5. **`test_pathspec_commit_isolates_shadow`**: a pathspec-limited commit still isolates shadow
6. **`test_commit_blocked_by_live_lock_holder`**: a live lock holder blocks the commit rather than corrupting state

## E2E Tests (test_localized_errors.rs)

Locale-aware output and CLI-surface scenarios, including: English/Japanese error and help/status messages selected from locale env vars, `--version`, `doctor` non-zero exit on issues plus valid English `--json`, valid `status --json`, and `uninstall` removing hooks/state (and refusing with active entries).

## E2E Tests (test_export_import.rs)

Ten scenarios building a source repo, `export`ing, `git clone`ing to a fresh repo, and `import`ing the built binary via `PATH` (locale pinned hermetically):

1. **`test_full_roundtrip_and_commit_cycle`**: overlay + phantom file + nested phantom dir → export → clone → install → import; phantom files/dir byte-identical, overlay carries shadow, then a real commit cycle leaks no shadow and restores it
2. **`test_upstream_moved_clean_merge`**: upstream changed a non-overlapping region → import merges upstream + shadow; commit cycle stays clean
3. **`test_upstream_moved_conflict_skips_then_force`**: overlapping change → import skips the overlay (non-zero) but imports phantoms; `--force` re-run keeps the shadow version
4. **`test_binary_phantom_roundtrip`**: phantom with null bytes round-trips byte-identical
5. **`test_nested_url_encoding_path_roundtrip`**: phantom dir with a space and `%` in the path round-trips
6. **`test_import_without_install_is_rejected`**, **`test_export_nothing_managed_is_rejected_localized`** (ja + en), **`test_export_refuses_with_stash_remnant`**, **`test_import_existing_differing_phantom_conflict_and_force`**: guard/refusal coverage
7. **`test_import_twice_is_idempotent`**: a second import exits 0 and changes nothing

## Testing Patterns

### Bypassing `GitRepo::discover()`

Unit tests in `src/` cannot rely on `std::env::current_dir()` because tests run in parallel. Each test module has a `make_test_repo()` function that creates a `tempfile::tempdir()`, runs `git init`, and calls `GitRepo::discover()` with the temp path directly.

### `*_for_test` Helper Functions

Some commands (e.g., `remove`, `restore`, `rebase`) have `_for_test` variants in their test modules that bypass interactive prompts or `cwd` discovery, calling the core logic directly.

### Test Coverage

Unit tests live in `src/` (via `#[cfg(test)]`) and E2E tests in `tests/`. All commands, hooks, and core modules have dedicated test coverage including worktree scenarios.
