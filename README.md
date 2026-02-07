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

## Quick Start

```bash
# Build from source
cargo install --path .

# Initialize in your repo
cd your-repo
git-shadow install

# Add an overlay (existing tracked file)
git-shadow add docker-compose.yml
echo "  # my debug port override" >> docker-compose.yml

# Add a phantom (new local-only file)
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add --phantom scripts/local-setup.sh

# Commit as usual — shadow changes are automatically excluded
git add -A && git commit -m "team changes"

# Verify: your personal changes are still in the working tree
cat docker-compose.yml        # includes your additions
git show HEAD:docker-compose.yml  # clean, team-only content
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

- [Detailed Usage Guide](docs/usage.md) | [日本語](docs/usage.ja.md)
- [Requirements Specification (Japanese)](docs/requirements.md)

## Requirements

- Git 2.20+
- Rust 1.70+ (to build from source)

## License

MIT
