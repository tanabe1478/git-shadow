# git-shadow Usage Guide

> **[日本語版はこちら (Japanese)](usage.ja.md)**

## Installation

```bash
# Build and install
cargo install --path .

# Verify
git-shadow --help
```

## Setup

Run `install` once per repository:

```bash
cd your-repo
git-shadow install
```

This creates:
- `.git/shadow/` directory (baselines, stash, config)
- Git hooks: `pre-commit`, `post-commit`, `post-merge`

If hooks already exist, they are renamed to `<hook>.pre-shadow` and chained after git-shadow's processing.

## Managing Files

### Overlay: Local Changes on Tracked Files

Use overlays when you want to add personal content to a file that the team already tracks.

```bash
# Register a tracked file
git-shadow add docker-compose.yml

# Edit freely — your changes are "shadow" changes
echo "  # my debug port override" >> docker-compose.yml
```

**What happens on commit:**
1. Your additions are stashed away
2. The original (baseline) content is committed
3. Your additions are restored immediately after

**Options:**
- `--force` — Skip the 1MB file size limit

### Phantom: Local-Only Files

Use phantoms for files that should exist only on your machine.

```bash
# Create and register a new local-only file
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add --phantom scripts/local-setup.sh
```

By default, phantom files are added to `.git/info/exclude` to hide them from `git status`.

**Options:**
- `--no-exclude` — Skip the `.git/info/exclude` entry. The file will appear in `git status` as untracked but will still be excluded from commits by the pre-commit hook.

#### Phantom Directories

You can also register entire directories as phantoms:

```bash
# Register a local-only directory
git-shadow add --phantom .claude/
git-shadow add --phantom codemaps/
```

Directory phantoms are managed via `.git/info/exclude` only — no stash/restore is needed. The directory and its contents remain in the working tree at all times, and any accidentally staged files are automatically unstaged by the pre-commit hook.

`git-shadow status` shows directory phantoms with a `(phantom dir)` label and an entry count instead of file size.

### Removing Files from Management

```bash
git-shadow remove docker-compose.yml
```

- **Overlay**: Restores the file to its baseline content. Shadow changes are discarded.
- **Phantom**: The file remains on disk but is no longer managed. Its `.git/info/exclude` entry is removed.

A confirmation prompt is shown before removal. Use `--force` to skip it (required in non-interactive environments).

## Viewing Status and Changes

### Status

```bash
git-shadow status
```

Shows all managed files with:
- Overlay: baseline commit hash, diff line counts (+/- lines)
- Phantom: exclude mode, file size
- Warnings for stale locks, stash remnants, or baseline drift

### Diff

```bash
# Show all shadow changes
git-shadow diff

# Show changes for a specific file
git-shadow diff docker-compose.yml
```

- **Overlay**: Shows a colored unified diff between the baseline and current content
- **Phantom**: Shows the entire file content as a new-file diff

## Handling Upstream Changes

When the team updates a file you have an overlay on (e.g., after `git pull`):

```bash
# post-merge hook will warn you:
# "warning: baseline for docker-compose.yml is outdated. Run `git-shadow rebase docker-compose.yml`"

# Update your baseline and re-apply shadow changes
git-shadow rebase docker-compose.yml
```

The rebase performs a 3-way merge:
1. Old baseline (common ancestor)
2. Your current content (with shadow changes)
3. New HEAD content (upstream changes)

If there's a conflict, standard conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`) are written to the file for manual resolution.

```bash
# Rebase all overlay files at once
git-shadow rebase
```

## Branch Switching

Overlay changes modify the working tree, which can block `git checkout`. Use `suspend` and `resume` to cleanly switch branches.

### Suspend

```bash
# Save shadow changes and restore baselines
git-shadow suspend
```

This:
1. Saves each overlay's working tree content to `.git/shadow/suspended/`
2. Restores baseline content to the working tree
3. Saves each phantom file to `.git/shadow/suspended/` and removes it from the working tree
4. Sets the config to "suspended" state

The working tree is now clean — you can switch branches freely.

### Resume

```bash
# After switching branches, restore shadow changes
git-shadow resume
```

If the baseline has not changed (same branch or file unchanged), suspended content is restored directly. If the baseline has changed (different branch), a 3-way merge is performed:

1. Old baseline (from before suspend)
2. Suspended content (your shadow changes)
3. New HEAD content (current branch's version)

If there's a conflict, standard conflict markers are written for manual resolution.

### Typical Workflow

```bash
# Working on feature branch with shadow changes
git-shadow suspend
git checkout main
git-shadow resume          # shadow changes re-applied to main's content

# Switch back
git-shadow suspend
git checkout feature
git-shadow resume          # shadow changes restored
```

### Restrictions While Suspended

- `git commit` is blocked (pre-commit hook will error)
- `git-shadow diff` and `git-shadow rebase` are blocked
- `git-shadow status` shows "SUSPENDED" state
- `git-shadow doctor` reports suspended state as a warning

## Recovery

### Automatic Recovery

If a commit is interrupted (e.g., commit message editor closed, commit-msg hook failed), shadow changes are stashed but not restored. The next git-shadow command will detect this and prompt you:

```
warning: stash has remaining files (a previous commit may have been interrupted)
  -> Run `git-shadow restore`
```

### Manual Recovery

```bash
# Restore all stashed files and clean up locks
git-shadow restore

# Restore a specific file
git-shadow restore docker-compose.yml
```

`restore` handles all abnormal states:
- Restores stashed files to the working tree
- Removes stale lockfiles
- Cleans up the stash directory

## Diagnostics

```bash
git-shadow doctor
```

Checks:
- Hook files exist with correct permissions and content
- No competing hook managers (Husky, pre-commit, lefthook)
- Config integrity (managed files and baselines exist)
- No stash remnants or stale locks

## Data Storage

All data lives inside `.git/shadow/`, which is automatically excluded from commits:

```
.git/shadow/
├── config.json          # Managed file list and metadata
├── lock                 # PID-based lockfile
├── baselines/           # Baseline snapshots (URL-encoded filenames)
│   └── docker-compose.yml
│   └── scripts%2Flocal-setup.sh
├── stash/               # Temporary stash during commits
│   └── ...
└── suspended/           # Shadow changes saved during suspend (branch switching)
    └── ...
```

### Path Encoding

Nested paths are URL-encoded for flat storage:
- `scripts/local-setup.sh` → `scripts%2Flocal-setup.sh`
- `docs/100%done.md` → `docs%2F100%25done.md`

Encoding order: `%` → `%25` first, then `/` → `%2F`.

## Important Notes

### `git commit --no-verify`

Using `--no-verify` skips the pre-commit hook, so shadow changes will be included in the commit. This is a Git limitation and cannot be prevented. Avoid using `--no-verify` when shadow-managed files have changes.

### Partial Staging

git-shadow does not support partial staging (`git add -p`) of overlay files. If both staged and unstaged changes exist for an overlay file, the pre-commit hook will block the commit. Stage the entire file with `git add <file>` before committing.

### Binary Files

Only text files are supported. Binary files are rejected by `git-shadow add` because the rebase command relies on text-based 3-way merging.
