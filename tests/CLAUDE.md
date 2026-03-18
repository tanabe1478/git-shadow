# tests/

Integration and E2E tests. Unit tests live alongside their modules in `src/` via `#[cfg(test)]`.

## Structure

| File | Purpose |
|------|---------|
| `common/mod.rs` | `TestRepo` helper for creating isolated git repos |
| `test_commit_cycle.rs` | E2E tests for the full commit lifecycle |
| `test_worktree.rs` | E2E tests for git worktree support |

## TestRepo Helper

`common::TestRepo` creates a temporary git repository with:
- `git init` + user config
- Helper methods: `create_file()`, `read_file()`, `commit()`, `init_shadow()`, `add_worktree()`

Used by E2E tests to set up realistic scenarios without touching the real filesystem.

## E2E Tests (test_commit_cycle.rs)

Three scenarios covering the core commit lifecycle:

1. **`test_full_overlay_commit_cycle`**: install -> add overlay -> edit -> pre-commit -> commit -> post-commit -> verify (committed content = baseline, working tree = shadow)
2. **`test_full_phantom_commit_cycle`**: install -> add phantom -> stage -> pre-commit -> commit -> post-commit -> verify (phantom not in commit, restored to working tree)
3. **`test_pre_commit_rollback_on_error`**: Simulates stash remnant causing pre-commit to fail, verifies shadow content is preserved (not lost during rollback)

## E2E Tests (test_worktree.rs)

Three scenarios covering git worktree support:

1. **`test_worktree_discover`**: Creates a worktree, verifies `GitRepo::discover()` resolves correct `root`, `git_dir`, and `common_dir` paths
2. **`test_worktree_install_shares_hooks`**: Installs hooks from a worktree, verifies hooks are written to `common_dir/hooks/` (shared with main worktree)
3. **`test_worktree_overlay_commit_cycle`**: Full overlay commit cycle in a worktree -- install, add, edit, pre-commit, commit, post-commit -- verifying per-worktree shadow state isolation

## Testing Patterns

### Bypassing `GitRepo::discover()`

Unit tests in `src/` cannot rely on `std::env::current_dir()` because tests run in parallel. Each test module has a `make_test_repo()` function that creates a `tempfile::tempdir()`, runs `git init`, and calls `GitRepo::discover()` with the temp path directly.

### `*_for_test` Helper Functions

Some commands (e.g., `remove`, `restore`, `rebase`) have `_for_test` variants in their test modules that bypass interactive prompts or `cwd` discovery, calling the core logic directly.

### Test Coverage

183 total tests: 175 unit tests (in `src/`) + 8 E2E tests (in `tests/`). All commands, hooks, and core modules have dedicated test coverage.
