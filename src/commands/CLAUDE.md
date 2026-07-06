# src/commands/

User-facing CLI commands. Each file corresponds to one subcommand and exposes a `run()` function called from `main.rs`.

## Command Map

| Command | File | Description |
|---------|------|-------------|
| `git-shadow install` | `install.rs` | Creates `.git/shadow/` dirs and installs hook scripts |
| `git-shadow uninstall` | `uninstall.rs` | Removes shadow hooks and per-worktree state (refuses with active entries unless `--force`) |
| `git-shadow add <file>` | `add.rs` | Registers overlay or phantom (with `--phantom`) |
| `git-shadow remove <file>` | `remove.rs` | Unregisters with confirmation prompt |
| `git-shadow status` | `status.rs` | Shows managed files, diff stats, warnings (`--json`) |
| `git-shadow diff [file]` | `diff.rs` | Shows shadow changes as unified diff |
| `git-shadow rebase [file]` | `rebase.rs` | Updates baseline via 3-way merge |
| `git-shadow restore [file]` | `restore.rs` | Recovers from interrupted commits |
| `git-shadow suspend` | `suspend.rs` | Suspends shadow changes for branch switching |
| `git-shadow resume` | `resume.rs` | Resumes suspended shadow changes (with 3-way merge) |
| `git-shadow doctor` | `doctor.rs` | Diagnoses hooks, config, stale state (`--json`, non-zero exit on issues) |
| `git-shadow hook <name>` | `hook.rs` | Internal dispatcher called from hook scripts |

## Design Notes

### Command Pattern

Every command follows the same structure:
1. `GitRepo::discover()` to find the repo from `cwd`
2. `ShadowConfig::load()` to read current state
3. Perform the operation
4. `config.save()` if state changed

### install.rs: Hook Chaining

Generated hook scripts call `git-shadow hook <name>` first, then chain to any pre-existing hook (renamed to `<hook>.pre-shadow`). This preserves existing hooks from other tools. Idempotent -- re-running `install` skips already-installed hooks. Four hooks are installed (`pre-commit`, `post-commit`, `post-merge`, `post-rewrite`) into `git.effective_hooks_dir()`: it honors `core.hooksPath` when set (so hooks actually run under husky/lefthook/custom setups), otherwise `common_dir/hooks/` so they are shared across worktrees. In a worktree, `inherit_from_main_worktree()` auto-inherits the managed file list from the main repo if no local config exists yet -- overlay baselines are regenerated from the worktree's HEAD, phantom entries are copied as-is.

### uninstall.rs: Reverse of install

Refuses while a commit is mid-flight (stash remnant or live lock) and refuses if files are still managed (`UninstallHasEntries`) unless `--force` -- which first restores overlay baselines to the working tree (phantoms are left on disk). Removes the shadow hooks from `effective_hooks_dir()` (only those dispatching to `git-shadow hook`), restores any `<hook>.pre-shadow` backup, regenerates the shared exclude section from the *other* worktrees' configs, then deletes this worktree's `shadow_dir`.

### add.rs: Overlay vs Phantom Validation

- **Overlay**: File MUST be tracked by git. Binary and size checks are performed. HEAD content is saved as baseline.
- **Phantom**: File must NOT be tracked. Added to `.git/info/exclude` by default (`--no-exclude` to skip).

### remove.rs: Interactive Confirmation

Uses `is_terminal::IsTerminal` to detect TTY. Non-interactive environments require `--force`. The confirmation prompt explains what will happen (overlay: shadow changes discarded; phantom: file remains on disk).

### rebase.rs: 3-Way Merge

Delegates to `merge::three_way_merge()`. The three inputs are:
- **base**: old baseline (stored in `baselines/`)
- **ours**: current working tree content (baseline + shadow changes)
- **theirs**: new HEAD content (upstream changes)

On conflict, standard markers are written and the user resolves manually.

### suspend.rs: Branch Switching Support

Saves shadow changes to `.git/shadow/suspended/` (separate from `stash/` which is for commit cycles). For overlays, restores baseline to working tree. For phantoms (non-directory), removes file from working tree. Guards: already suspended, lock held, stash remnants. Sets `config.suspended = true`.

### resume.rs: Restore Suspended Changes

Restores suspended shadow changes. If baseline is unchanged, restores directly. If baseline changed (different branch), performs 3-way merge via `merge::three_way_merge()`. Creates parent directories before writing (may be missing after branch switch). Cleans up `suspended/` directory and sets `config.suspended = false`.

### hook.rs: Hidden Command

The `hook` subcommand is `#[command(hide = true)]` in clap -- it doesn't appear in `--help`. It's only called by the hook scripts installed by `install`. It discovers the repo from `cwd`, so it works correctly in worktrees.

### doctor.rs: Diagnostic Categories

Checks are split into **issues** (red, things that are broken) and **warnings** (yellow, things that need attention). Any issue makes `doctor` exit non-zero (so scripts/CI can gate on it); warnings alone keep a zero exit. `--json` emits a structured, English-only report. Checks include: hook existence/permissions/content, inert hooks (installed in the default dir while `core.hooksPath` points elsewhere), competing hook managers (Husky, pre-commit, lefthook), config integrity, stash remnants, stale locks, suspended state, and worktree initialization (`check_worktree()` warns if running in a worktree where `git-shadow install` has not been run yet).
