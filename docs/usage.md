# git-shadow Usage Guide

> **[日本語版はこちら (Japanese)](usage.ja.md)**

## Installation

```bash
# Build and install
cargo install --path .

# Verify
git-shadow --help
git-shadow --version
```

## Setup

Run `install` once per repository:

```bash
cd your-repo
git-shadow install
```

This creates:
- `.git/shadow/` directory (baselines, stash, config)
- Git hooks: `pre-commit`, `post-commit`, `post-merge`, `post-rewrite`

If hooks already exist, they are renamed to `<hook>.pre-shadow` and chained after git-shadow's processing.

> **`core.hooksPath`**: If your repository sets `core.hooksPath` (e.g., Husky, lefthook, or a custom `dev-hooks/` directory), `install` places its hooks into that effective directory so they actually run, and prints a note such as `note: core.hooksPath (.husky) is set, so hooks were installed into <path>`. `git-shadow doctor` reports an issue if hooks are installed in the default directory while `core.hooksPath` points elsewhere (they would be inert and silently skipped).

> **Worktrees**: If you use `git worktree`, run `git-shadow install` separately in each worktree. If the main repo already has shadow-managed files, `install` automatically inherits the file list (overlay baselines are regenerated from the worktree's HEAD; phantom entries are copied as-is). See [git worktree Support](#git-worktree-support) for details.

## Managing Files

### Adding Files

`git-shadow add` accepts one or more paths and automatically chooses how to manage each one:

- **Tracked files** become **overlays** (local changes layered on top of committed content).
- **Existing untracked paths** become **phantoms** (files or directories that live only on your machine).

```bash
# Add several files at once — each is classified automatically
git-shadow add docker-compose.yml scripts/local-setup.sh .env.local
```

If any path cannot be classified (it is neither tracked nor exists on disk), that path fails with an error and the remaining paths are still processed; the command exits non-zero when at least one path failed.

**Options:**
- `--overlay` — Force overlay registration for all given paths (the file must be tracked)
- `--phantom` — Force phantom registration for all given paths (the path must not be tracked)
- `--no-exclude` — Skip the `.git/info/exclude` entry (phantom only). The file will appear in `git status` as untracked but is still excluded from commits by the pre-commit hook.
- `--force` — Ignore the 1MB overlay file size limit

`--overlay` and `--phantom` are mutually exclusive.

### Overlay: Local Changes on Tracked Files

Use overlays when you want to add personal content to a file that the team already tracks.

```bash
# Register a tracked file (auto-detected as an overlay)
git-shadow add docker-compose.yml

# Edit freely — your changes are "shadow" changes
echo "  # my debug port override" >> docker-compose.yml
```

**What happens on commit:**
1. Your additions are stashed away
2. The original (baseline) content is committed
3. Your additions are restored immediately after

### Phantom: Local-Only Files

Use phantoms for files that should exist only on your machine.

```bash
# Create a new local-only file, then register it (auto-detected as a phantom)
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add scripts/local-setup.sh
```

By default, phantom files are added to `.git/info/exclude` to hide them from `git status`. Use `--no-exclude` to skip that entry.

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

## Uninstalling

To remove git-shadow from a repository entirely:

```bash
git-shadow uninstall
```

This:
- Removes the git-shadow hooks from the effective hooks directory (respecting `core.hooksPath`) and restores any `<hook>.pre-shadow` backups made at install time
- Removes this worktree's entries from the managed section of `.git/info/exclude` (entries owned by other worktrees are preserved)
- Deletes this worktree's shadow state (`.git/shadow/`)

For safety, `uninstall` **refuses** to run in two situations:
- **Files are still managed** — it stops with an error listing the count. Either `git-shadow remove <file>` each file first, or re-run with `--force`.
- **A commit is in progress** — a leftover stash or a lock held by another live process means a commit cycle is mid-flight, so wiping state could lose work.

```bash
# Restore overlay baselines to the working tree and wipe state even if files are still managed
git-shadow uninstall --force
```

With `--force`, overlay files are restored to their baseline content (shadow changes discarded) and the count is reported, e.g. `restored baselines to the working tree for 1 overlay(s)`. Phantom files are left on disk untouched — they are your local-only files. On success you'll see `git-shadow uninstalled (hooks, exclude entries, and state removed)`.

### Manual removal

If the binary is unavailable and you need to remove git-shadow by hand:

1. In the effective hooks directory (`.git/hooks/`, or your `core.hooksPath`), delete the `pre-commit`, `post-commit`, `post-merge`, and `post-rewrite` scripts that call `git-shadow hook`. If a `<hook>.pre-shadow` backup exists, rename it back to `<hook>`.
2. Restore any overlay files you want to reset to their committed content (e.g., `git restore --source=HEAD -- <file>`).
3. Delete `.git/shadow/`.
4. Remove the git-shadow managed section (between its marker comments) from `.git/info/exclude`.

## Viewing Status and Changes

### Status

```bash
git-shadow status
```

Shows all managed files with:
- Overlay: baseline commit hash, diff line counts (+/- lines)
- Overlay: current Git state (`clean`, `modified`, `staged`, or `partially staged`)
- Overlay: a local-only warning when staged changes will be stripped at commit time
- Phantom: exclude mode, file size
- Warnings for stale locks, stash remnants, or baseline drift

For an opt-in combined view with normal Git output:

```bash
git shadow status --git
```

This prints `git status --short --branch` first, then the shadow-managed summary. `git-shadow` does not replace `git status` by default.

For scripting, use `--json`:

```bash
git-shadow status --json
```

This emits a stable, English (non-localized) JSON document and suppresses the human-readable output. Keys are stable identifiers suitable for parsing — for example `git_state` is one of `clean`, `modified`, `staged`, or `partially_staged`, and `warnings` holds tokens such as `stash_remaining` or `stale_lock`:

```json
{
  "suspended": false,
  "warnings": [],
  "files": [
    {
      "path": "docker-compose.yml",
      "type": "overlay",
      "exists": true,
      "baseline_commit": "f5fb751...",
      "shadow_added": 1,
      "shadow_removed": 0,
      "git_state": "modified",
      "baseline_outdated": false
    }
  ]
}
```

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

When the team updates a file you have an overlay on (e.g., after `git pull`), the `post-merge` and `post-rewrite` hooks run automatically:

- **Clean merges** — the baseline and your shadow changes are re-applied automatically (a clean-only auto-rebase). No action is needed.
- **Conflicts** — the auto-rebase is skipped and you are warned to resolve it manually with `git-shadow rebase <file>`.

If a rebase is left to you, run it explicitly:

```bash
# Update your baseline and re-apply shadow changes
git-shadow rebase docker-compose.yml
```

If the lock is held by a live process when the hook fires, the auto-rebase is skipped for safety and you can run `git-shadow rebase` yourself afterward.

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

When a stale lock is found during `git commit`, git-shadow also tries safe auto-recovery first. If restoring would overwrite newer working-tree content, the commit is still blocked and manual `git-shadow restore` is required.

`restore` refuses to run when the lock is held by another **live** process (a real commit or hook is in flight), so it never clobbers work another process is doing. It only cleans up locks whose owning process is gone (stale).

### Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Shadow changes get committed / hooks never run | `core.hooksPath` points somewhere other than where the hooks live, so they are inert | Re-run `git-shadow install` (it installs into the effective hooks directory). `git-shadow doctor` reports this as an issue. |
| `git commit` blocked with "another git-shadow process still holds the lock" | Another live commit or hook is running | Wait for it to finish. If nothing is actually running, the lock is stale — run `git-shadow restore`. |
| `git commit` blocked with "leftover files remain in `.git/shadow/stash/`" | A previous commit was interrupted | Run `git-shadow restore`, then commit again. |
| `git-shadow restore` refuses to run | The lock is held by a live process | Let that process finish; restore only cleans up stale locks. |
| `git-shadow resume` blocked with "was edited in the working tree while suspended" | You edited a file after suspending it, so resuming would overwrite your edits | Review the file, save what you want to keep, reconcile with `.git/shadow/suspended/`, then run `git-shadow resume` again. |
| `git-shadow doctor` exits non-zero | It found one or more issues (broken hooks, missing baselines, inert hooks, ...) | Read the `issues:` list and address each; warnings alone do not cause a non-zero exit. |

## Diagnostics

```bash
git-shadow doctor
```

Checks:
- Hook files exist with correct permissions and content, and are not inert (installed in the default directory while `core.hooksPath` points elsewhere)
- No competing hook managers (Husky, pre-commit, lefthook)
- Config integrity (managed files and baselines exist)
- No stash remnants or stale locks
- Suspended state and worktree initialization

Findings are split into **issues** (red `✗`, things that are broken) and **warnings** (yellow `⚠`, things that need attention).

**Exit codes:** `doctor` exits non-zero when it finds one or more issues (e.g., `Error: doctor found 4 issue(s)`), so you can gate scripts or CI on it. Warnings alone keep a zero exit code.

For scripting, use `--json`:

```bash
git-shadow doctor --json
```

This emits a stable, English (non-localized) JSON document and suppresses the human-readable output. The `ok` field is `false` when there are issues (matching the non-zero exit code):

```json
{
  "ok": true,
  "issues": [],
  "warnings": []
}
```

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

In a `git worktree` setup, storage is split between two directories:

| Location | Scope | Contents |
|----------|-------|----------|
| `git_dir` (per-worktree `.git`) | Per-worktree | `shadow/` (config, baselines, stash, suspended, lock) |
| `common_dir` (shared `.git`) | Shared | `hooks/`, `info/exclude` |

This means each worktree has independent shadow state, while hooks and exclude rules are shared across all worktrees.

### Path Encoding

Nested paths are URL-encoded for flat storage:
- `scripts/local-setup.sh` → `scripts%2Flocal-setup.sh`
- `docs/100%done.md` → `docs%2F100%25done.md`

Encoding order: `%` → `%25` first, then `/` → `%2F`.

## Workflows

### Basic: single repo setup

```bash
git-shadow install
git-shadow add docker-compose.yml     # overlay: tracked file with local overrides
git-shadow add --phantom .env.local  # phantom: local-only config (untracked)

# Normal development — shadow changes are stripped automatically
vim docker-compose.yml
git commit -am "feat: add login"   # local overrides are NOT committed
```

### Adding a worktree

When you create a worktree, run `git-shadow install` once. It inherits the managed file list from the main repo automatically.

```bash
git worktree add ../feature-branch feature/auth
cd ../feature-branch
git-shadow install
# → "inherited 2 file(s) from main worktree"
# → overlay baselines regenerated from HEAD
# → phantom entries copied

# Ready to work immediately
vim .env.local
git commit -am "feat: auth"        # shadow changes still stripped
```

### Per-worktree customization

After inheriting, each worktree can independently add or remove managed files.

```bash
cd ../feature-branch
git-shadow add --phantom TODO.md   # only in this worktree
git-shadow remove notes.md         # only in this worktree
```

### PR review with a temporary worktree

```bash
git worktree add ../review-pr-42 pr/42
cd ../review-pr-42
git-shadow install                 # inherits config, ready to build/test

# After review, remove the worktree (shadow state is cleaned up automatically)
cd ../main-repo
git worktree remove ../review-pr-42
```

### Branch switching without worktrees

If you prefer switching branches in a single working tree, use suspend/resume:

```bash
git-shadow suspend                 # stash shadow changes
git checkout other-branch
git-shadow resume                  # restore with 3-way merge if needed
```

With worktrees, suspend/resume is unnecessary — each worktree has independent state.

### Quick reference

| Task | Command |
|------|---------|
| Initial setup | `git-shadow install` → `git-shadow add <file>` |
| Add a worktree | `git worktree add ...` → `cd` → `git-shadow install` |
| Worktree-specific file | `git-shadow add --phantom <file>` |
| Remove a worktree | `git worktree remove <path>` (shadow state cleaned up) |
| Check status | `git-shadow status` / `git-shadow doctor` |
| Branch switch (no worktree) | `git-shadow suspend` → checkout → `git-shadow resume` |
| Remove git-shadow from a repo | `git-shadow uninstall` (or `--force`) |

## Important Notes

### `git commit --no-verify`

Using `--no-verify` skips the pre-commit hook, so shadow changes will be included in the commit. This is a Git limitation and cannot be prevented. Avoid using `--no-verify` when shadow-managed files have changes.

### Partial Staging

git-shadow does not support partial staging (`git add -p`) of overlay files. If both staged and unstaged changes exist for an overlay file, the pre-commit hook will block the commit. Stage the entire file with `git add <file>` before committing.

### `git add` Guardrails

Git does not provide a general pre-`add` hook, so git-shadow cannot warn before every `git add`. Instead:

- `git-shadow status` shows when overlay-managed files are local-only and currently staged
- `git shadow status --git` gives an opt-in daily wrapper view with normal Git status first
- the pre-commit hook warns when staged local-only overlay changes are about to be stripped

### Binary Files

Only text files are supported. Binary files are rejected by `git-shadow add` because the rebase command relies on text-based 3-way merging.

### git worktree Support

git-shadow works with `git worktree` setups. Each worktree is treated as an independent shadow environment:

- **Per-worktree state**: Config, baselines, stash, suspended state, and lockfiles are stored in each worktree's own `.git` directory.
- **Auto-inherit on install**: When you run `git-shadow install` in a worktree, if the main repo has shadow-managed files and the worktree has no existing config, the file list is automatically inherited. Overlay baselines are regenerated from the worktree's HEAD, and phantom entries are copied as-is. The output message is `inherited N file(s) from main worktree`.
- **Shared resources**: Git hooks and `.git/info/exclude` entries are stored in the common Git directory and shared across all worktrees.
- **Diagnostics**: `git-shadow doctor` detects when you are in a worktree and warns if shadow has not been initialized.
- **Git version**: Git 2.31+ is recommended for full worktree support (`--path-format=absolute`). Older versions (2.20+) are supported via a fallback, but 2.31+ is preferred.

```bash
# Main repository
cd my-repo
git-shadow install
git-shadow add docker-compose.yml

# Create and set up a worktree — install inherits managed files automatically
git worktree add ../my-repo-feature feature-branch
cd ../my-repo-feature
git-shadow install              # inherits docker-compose.yml from main repo
```
