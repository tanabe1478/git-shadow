# git-shadow

> **[日本語版はこちら (Japanese)](README.ja.md)**

A CLI tool for managing **local-only changes** in Git repositories. Your edits stay active in the working tree during development, but are automatically stripped before each commit — keeping Git history clean.

## Why?

Sometimes you need personal changes to shared files — debug settings in a config, local environment overrides, or private notes. git-shadow lets you maintain those local edits without them ever appearing in the team's commit history.

## Concepts

| Type | Description | Example |
|------|-------------|---------|
| **overlay** | Layer local changes on top of an existing tracked file | Add personal debug settings to a shared `docker-compose.yml` |
| **phantom** | Create a file that exists only locally and is never committed | Create a local-only `scripts/local-setup.sh` for your environment |
| **phantom dir** | Manage an entire directory that exists only locally (exclude-only, no stash/restore) | Keep a local-only `.claude/` directory out of every commit |

## Installation

### Download pre-built binary

Download the latest binary for your platform from [GitHub Releases](https://github.com/tanabe1478/git-shadow/releases/latest):

| Platform | Architecture | Download |
|----------|-------------|----------|
| Linux | x86_64 | [git-shadow-x86_64-unknown-linux-gnu.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-x86_64-unknown-linux-gnu.tar.gz) |
| Linux | aarch64 | [git-shadow-aarch64-unknown-linux-gnu.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-unknown-linux-gnu.tar.gz) |
| macOS | Apple Silicon | [git-shadow-aarch64-apple-darwin.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-apple-darwin.tar.gz) |
| macOS | Intel | [git-shadow-x86_64-apple-darwin.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-x86_64-apple-darwin.tar.gz) |

```bash
# Example: macOS Apple Silicon
curl -LO https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-apple-darwin.tar.gz
tar xzf git-shadow-aarch64-apple-darwin.tar.gz
sudo mv git-shadow /usr/local/bin/
```

### Build from source

```bash
cargo install --path .
```

## Quick Start

```bash
# Initialize in your repo
cd your-repo
git-shadow install

# Add managed files (tracked => overlay, untracked => phantom)
git-shadow add docker-compose.yml
git-shadow add scripts/local-setup.sh
echo "  # my debug port override" >> docker-compose.yml

# If you want to force phantom/overlay explicitly, the old flags still work
git-shadow add --phantom another-local-file.sh

# Inspect normal Git state plus shadow meaning
git shadow status --git

# Commit as usual — shadow changes are automatically excluded
git add -A && git commit -m "team changes"

# Verify: your personal changes are still in the working tree
cat docker-compose.yml        # includes your additions
git show HEAD:docker-compose.yml  # clean, team-only content
```

## Commands

| Command | Description |
|---------|-------------|
| `git-shadow install` | Set up Git hooks (pre-commit, post-commit, post-merge, post-rewrite); respects `core.hooksPath` |
| `git-shadow uninstall [--force]` | Remove hooks, exclude entries, and state; `--force` restores overlays even when files are still managed |
| `git-shadow add <file>...` | Register tracked files as overlays and existing untracked paths as phantoms automatically |
| `git-shadow add --phantom <file>...` | Force local-only files/directories to be phantoms |
| `git-shadow remove <file>` | Unregister a file from shadow management |
| `git-shadow status [--git] [--json]` | Show managed files and their state, optionally prefixed by `git status --short --branch`; `--json` emits stable English JSON for scripting |
| `git-shadow diff [file]` | Show shadow changes as a unified diff |
| `git-shadow rebase [file]` | Update baseline after upstream changes (3-way merge) |
| `git-shadow restore [file]` | Recover from interrupted commits or crashes |
| `git-shadow suspend` | Suspend shadow changes for branch switching |
| `git-shadow resume` | Resume suspended shadow changes (with 3-way merge if needed) |
| `git-shadow export [path] [--force]` | Bundle managed state into a portable archive for moving to a new machine |
| `git-shadow import <archive> [--force]` | Restore managed state from an archive into a freshly cloned repo (3-way merge, safe-by-default) |
| `git-shadow doctor [--json]` | Diagnose hooks, config integrity, and stale state; exits non-zero when issues are found; `--json` emits stable English JSON |

`git-shadow --version` prints the installed version.

## How It Works

1. **pre-commit hook**: Stashes your shadow changes, restores baseline content, updates the index
2. **git commit**: Records the clean baseline (no shadow changes)
3. **post-commit hook**: Restores your shadow changes from the stash

All data is stored in `.git/shadow/` — inside `.git/`, so it's never committed.

**Worktree support**: In `git worktree` setups, hooks and exclude rules are shared across worktrees, but shadow state (config, baselines, stash) is per-worktree. Each worktree needs its own `git-shadow install`. If the main repo already has shadow-managed files, `install` automatically inherits the file list — overlay baselines are regenerated from the worktree's HEAD, and phantom entries are copied as-is. This means a single `install` command is all you need to set up a worktree.

## Safety

- **Atomic writes**: File operations use temp-file-then-rename to prevent corruption
- **Lockfile**: PID-based lock prevents concurrent operations
- **Rollback**: Failed pre-commit operations are rolled back automatically
- **Recovery**: `git-shadow restore` recovers from any interrupted state
- **Auto-healing**: stale locks are recovered automatically when doing so is safe; ambiguous cases still stop and ask for manual restore

## Daily Flow Notes

- `git status` itself is not replaced by default. Use `git shadow status --git` if you want an opt-in combined view.
- Git does not provide a general pre-`add` hook, so early warnings for overlay files happen in `git-shadow status` and at commit time, not during `git add`.

## Claude Code Plugin

This repo ships a [Claude Code](https://code.claude.com) skill so an AI coding
agent can drive `git-shadow` correctly (see [`skills/git-shadow/SKILL.md`](skills/git-shadow/SKILL.md)).
The repo is itself a plugin marketplace.

**Install via the marketplace (recommended):**

```text
/plugin marketplace add tanabe1478/git-shadow
/plugin install git-shadow@git-shadow
```

**Fallback for non-plugin setups** — copy or symlink the skill into your personal
skills directory:

```bash
ln -s "$(pwd)/skills/git-shadow" ~/.claude/skills/git-shadow   # or: cp -r skills/git-shadow ~/.claude/skills/git-shadow
```

## Documentation

- [Detailed Usage Guide](docs/usage.md) | [日本語](docs/usage.ja.md)
- [Requirements Specification (Japanese)](docs/requirements.md)

## Requirements

- Git 2.20+ (Git 2.31+ recommended for full worktree support)
- Rust 1.70+ (only if building from source)

## License

MIT
