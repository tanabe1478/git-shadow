# src/commands/

User-facing CLI commands. Each file corresponds to one subcommand and exposes a `run()` function called from `main.rs`.

## Command Map

| Command | File | Description |
|---------|------|-------------|
| `git-shadow install` | `install.rs` | Creates `.git/shadow/` dirs and installs hook scripts |
| `git-shadow add <file>` | `add.rs` | Registers overlay or phantom (with `--phantom`) |
| `git-shadow remove <file>` | `remove.rs` | Unregisters with confirmation prompt |
| `git-shadow status` | `status.rs` | Shows managed files, diff stats, warnings |
| `git-shadow diff [file]` | `diff.rs` | Shows shadow changes as unified diff |
| `git-shadow rebase [file]` | `rebase.rs` | Updates baseline via 3-way merge |
| `git-shadow restore [file]` | `restore.rs` | Recovers from interrupted commits |
| `git-shadow doctor` | `doctor.rs` | Diagnoses hooks, config, stale state |
| `git-shadow hook <name>` | `hook.rs` | Internal dispatcher called from hook scripts |

## Design Notes

### Command Pattern

Every command follows the same structure:
1. `GitRepo::discover()` to find the repo from `cwd`
2. `ShadowConfig::load()` to read current state
3. Perform the operation
4. `config.save()` if state changed

### install.rs: Hook Chaining

Generated hook scripts call `git-shadow hook <name>` first, then chain to any pre-existing hook (renamed to `<hook>.pre-shadow`). This preserves existing hooks from other tools. Idempotent -- re-running `install` skips already-installed hooks.

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

### hook.rs: Hidden Command

The `hook` subcommand is `#[command(hide = true)]` in clap -- it doesn't appear in `--help`. It's only called by the hook scripts installed by `install`.

### doctor.rs: Diagnostic Categories

Checks are split into **issues** (red, things that are broken) and **warnings** (yellow, things that need attention). Checks include: hook existence/permissions/content, competing hook managers (Husky, pre-commit, lefthook), config integrity, stash remnants, stale locks.
