# git-shadow

A CLI tool for managing **local-only changes** in Git repositories. Your edits stay active in the working tree during development, but are automatically stripped before each commit — keeping Git history clean.

## Why?

The primary use case is managing personal additions to shared files like `CLAUDE.md`. You can append your own prompts, notes, or coding conventions without polluting the team's commit history.

## Concepts

| Type | Description | Example |
|------|-------------|---------|
| **overlay** | Layer local changes on top of an existing tracked file | Append personal notes to the root `CLAUDE.md` |
| **phantom** | Create a file that exists only locally and is never committed | Add `src/components/CLAUDE.md` as a new local-only file |

## Quick Start

```bash
# Build from source
cargo install --path .

# Initialize in your repo
cd your-repo
git-shadow install

# Add an overlay (existing tracked file)
git-shadow add CLAUDE.md
echo "# My personal notes" >> CLAUDE.md

# Add a phantom (new local-only file)
echo "# Component docs" > src/components/CLAUDE.md
git-shadow add --phantom src/components/CLAUDE.md

# Commit as usual — shadow changes are automatically excluded
git add -A && git commit -m "team changes"

# Verify: your personal notes are still in the working tree
cat CLAUDE.md  # includes your additions
git show HEAD:CLAUDE.md  # clean, team-only content
```

## Commands

| Command | Description |
|---------|-------------|
| `git-shadow install` | Set up Git hooks (pre-commit, post-commit, post-merge) |
| `git-shadow add <file>` | Register a tracked file as an overlay |
| `git-shadow add --phantom <file>` | Register a local-only file as a phantom |
| `git-shadow remove <file>` | Unregister a file from shadow management |
| `git-shadow status` | Show managed files and their state |
| `git-shadow diff [file]` | Show shadow changes as a unified diff |
| `git-shadow rebase [file]` | Update baseline after upstream changes (3-way merge) |
| `git-shadow restore [file]` | Recover from interrupted commits or crashes |
| `git-shadow doctor` | Diagnose hooks, config integrity, and stale state |

## How It Works

1. **pre-commit hook**: Stashes your shadow changes, restores baseline content, updates the index
2. **git commit**: Records the clean baseline (no shadow changes)
3. **post-commit hook**: Restores your shadow changes from the stash

All data is stored in `.git/shadow/` — inside `.git/`, so it's never committed.

## Safety

- **Atomic writes**: File operations use temp-file-then-rename to prevent corruption
- **Lockfile**: PID-based lock prevents concurrent operations
- **Rollback**: Failed pre-commit operations are rolled back automatically
- **Recovery**: `git-shadow restore` recovers from any interrupted state

## Documentation

- [Detailed Usage Guide](docs/usage.md)
- [Requirements Specification (Japanese)](docs/requirements.md)

## Requirements

- Git 2.20+
- Rust 1.70+ (to build from source)

## License

MIT
