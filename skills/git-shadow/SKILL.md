---
name: git-shadow
description: >-
  Manage local-only changes in a Git repository with the git-shadow CLI so
  personal edits never get committed. Use when the user wants to keep debug
  settings, local config overrides, credentials, or private notes active in
  their working tree but stripped from every commit; when they mention
  "overlay" (a local diff on a tracked file), "phantom" (a local-only file),
  or "phantom dir" (a local-only directory); when git-shadow reports a stash
  remnant, stale lock, suspended state, or rebase/import conflict; or when they
  need to move their local-only state to a new machine via export/import.
  Triggers: "keep this out of commits", "local-only config", "don't commit my
  debug changes", "git-shadow status/doctor/rebase/suspend/resume/restore",
  "migrate my shadow state", "set up git-shadow on a new machine".
---

# git-shadow

`git-shadow` is a Rust CLI that keeps **local-only changes** out of Git history
while leaving them active in the working tree. Shadow state lives under
`.git/shadow/` and is layered on top of the checked-out files. Git hooks
(`pre-commit`, `post-commit`, `post-merge`, `post-rewrite`) stash the shadow
content before each commit and restore it immediately afterward, so personal
edits are never pushed.

Use it when someone needs personal edits (debug flags, local DB hosts,
credentials, private notes) that must stay in the working tree but be excluded
from every commit.

## Three management types

| Type            | Purpose                                             | Example                                        |
|-----------------|-----------------------------------------------------|------------------------------------------------|
| **overlay**     | Local diff layered on top of a tracked file         | Change the DB host in `docker-compose.yml`     |
| **phantom**     | A local-only file that is never committed           | `.env.local`, a personal `NOTES.md`            |
| **phantom dir** | A local-only directory (exclude-only management)    | `.claude/`, a scratch `codemaps/` directory    |

`add` auto-detects the type: a tracked file becomes an **overlay**, an existing
untracked path becomes a **phantom** (file or directory). Force it with
`--overlay` / `--phantom`. Phantoms are added to `.git/info/exclude` unless you
pass `--no-exclude`. Phantom directories are managed via `.git/info/exclude`
only -- no stash/restore is performed; any accidentally staged files under them
are unstaged by the pre-commit hook.

## Prerequisites and detection

Before acting, confirm the tool and repo state. **Prefer the `--json` output**
of `status` and `doctor`: its keys are stable English identifiers and are not
localized, whereas the human-readable output is translated (ja/en) and should
not be parsed.

```bash
git-shadow --version          # tool available? e.g. "git-shadow 0.1.0"
git-shadow status --json      # managed files + warnings, machine-readable
git-shadow doctor --json      # health check, machine-readable
```

`status --json` shape (keys are stable):

```json
{
  "suspended": false,
  "warnings": [],
  "files": [
    {
      "path": ".env.local",
      "type": "phantom",
      "exists": true,
      "exclude_mode": "git_info_exclude",
      "size_bytes": 11
    },
    {
      "path": "app.yml",
      "type": "overlay",
      "exists": true,
      "baseline_commit": "e01888e0048d7f3503943c3a882fbc574c448be5",
      "shadow_added": 1,
      "shadow_removed": 1,
      "git_state": "modified",
      "baseline_outdated": false
    },
    {
      "path": "localdir",
      "type": "phantom",
      "is_directory": true,
      "exists": true,
      "exclude_mode": "git_info_exclude",
      "entry_count": 1
    }
  ]
}
```

- `suspended` -- `true` while shadow changes are suspended for branch switching.
- `warnings` -- tokens such as `stash_remaining`, `stale_lock`, `baseline_drift`.
- overlay entries carry `baseline_commit`, `shadow_added`, `shadow_removed`,
  `git_state` (`clean` / `modified` / `staged` / `partially_staged`), and
  `baseline_outdated`.
- phantom files carry `exclude_mode` (`git_info_exclude` or `none`) and
  `size_bytes`; phantom directories add `is_directory: true` and `entry_count`.

`doctor --json` shape:

```json
{ "ok": true, "issues": [], "warnings": [] }
```

- `ok` is `false` when there are `issues`. **`doctor` exits non-zero when it
  finds issues** (warnings alone keep a zero exit), so it works as a health gate
  in scripts and CI. Issues are things that are broken (missing/inert hooks,
  missing baselines); warnings are things that need attention (suspended state,
  stash remnant, stale lock, uninitialized worktree).

## Command reference

| Command | What it does |
|---------|--------------|
| `git-shadow install` | Set up hooks and `.git/shadow/`. Run once per repository (and once per worktree). Honors `core.hooksPath`; in a worktree it auto-inherits the managed file list from the main repo. |
| `git-shadow add [--overlay\|--phantom] [--no-exclude] [--force] <files>...` | Register one or more paths. Auto-detects overlay (tracked) vs phantom (untracked file/dir). `--overlay`/`--phantom` force the type (mutually exclusive); `--no-exclude` skips the `.git/info/exclude` entry (phantom only); `--force` ignores the 1 MB overlay size limit. Failing paths are reported and skipped; the command exits non-zero if any path failed. |
| `git-shadow remove [--force] <file>` | Unregister a file. Overlay: file is restored to baseline (shadow changes discarded). Phantom: file stays on disk, its exclude entry is removed. Prompts for confirmation; `--force` skips it (needed non-interactively). |
| `git-shadow status [--git] [--json]` | Show managed files and warnings. `--git` prints `git status --short --branch` first. `--json` emits stable English JSON and suppresses human output. |
| `git-shadow diff [<file>]` | Show shadow changes as a colored unified diff (overlay: baseline vs current; phantom: whole file as new-file diff). Omit the file for all. Blocked while suspended. |
| `git-shadow rebase [<file>]` | Update the baseline to current HEAD and re-apply shadow changes via 3-way merge. Run after upstream moves. Omit the file to rebase all overlays. Conflicts write standard `<<<<<<<`/`=======`/`>>>>>>>` markers. Blocked while suspended. |
| `git-shadow suspend` | Save shadow changes to `.git/shadow/suspended/`, restore baselines, and remove phantoms from the working tree so branch switching is clean. |
| `git-shadow resume` | Restore suspended shadow changes; if the baseline moved, re-applies them with a 3-way merge. |
| `git-shadow restore [<file>]` | Recover from an abnormal state: restore stashed files, clean up stale locks and the stash directory. Refuses to run when the lock is held by a **live** process. |
| `git-shadow export [--force] [<output>]` | Bundle all managed state into a portable `.tar.gz` (default `git-shadow-export.tar.gz`). `--force` overwrites an existing archive. Refuses when nothing is managed, while suspended, or mid-commit. |
| `git-shadow import [--force] <archive>` | Restore managed state from an archive into a fresh clone (requires `install` first). Safe by default: continues past conflicts, exits non-zero if any entry was skipped. `--force` overwrites conflicting files and replaces differing entries. Idempotent to re-run. |
| `git-shadow doctor [--json]` | Diagnose hooks, config, stash/lock, suspended and worktree state. **Non-zero exit on issues** -- usable as a health gate. `--json` for stable output. |
| `git-shadow uninstall [--force]` | Remove hooks, this worktree's exclude entries, and `.git/shadow/`. Refuses if files are still managed or a commit is in flight. `--force` restores overlay baselines to the working tree (discarding shadow changes), leaves phantom files on disk, and wipes state. |

## Common workflows

### (a) Daily flow -- add, then commit normally

```bash
git-shadow install                       # once per repo
git-shadow add docker-compose.yml        # tracked -> overlay
git-shadow add --phantom .env.local      # untracked -> phantom
# edit freely, then commit as usual:
git commit -am "feat: something"         # hooks strip shadow content, restore it after
```

No special commit command is needed -- the hooks handle stash/restore. Check
state any time with `git-shadow status` (or `status --json` for scripting).

### (b) Branch switching -- suspend / resume

Overlay edits dirty the working tree and can block `git checkout`.

```bash
git-shadow suspend        # save shadow, restore baselines, drop phantoms
git checkout other-branch
git-shadow resume         # re-apply shadow (3-way merge if the baseline moved)
```

(With `git worktree`, each worktree has independent state, so suspend/resume is
usually unnecessary.)

### (c) Upstream moved -- rebase

After `git pull`, the `post-merge`/`post-rewrite` hooks auto-rebase clean cases.
If a merge conflicts, or a live lock blocks the hook, rebase yourself:

```bash
git-shadow rebase                    # all overlays
git-shadow rebase docker-compose.yml # one file; resolve any conflict markers
```

### (d) Machine migration -- export -> clone -> install -> import

Shadow state is **not** carried by `git clone` (it lives in `.git/shadow/` and
`.git/info/exclude`). To move it:

```bash
# Old machine
git-shadow export ~/shadow.tar.gz      # or bare `export` -> git-shadow-export.tar.gz

# New machine
git clone <repo-url> myrepo && cd myrepo
git-shadow install
git-shadow import ~/shadow.tar.gz
```

Conflict semantics: import is **safe by default** -- it processes every entry,
skips ones that conflict (a phantom that already exists with different content,
or an overlay whose 3-way merge conflicts), prints a per-file message, and
**exits non-zero**. It does not write conflict markers. Re-running import after
resolving the local edit completes the rest; an identical re-import is a no-op
(idempotent). `--force` overwrites conflicting files / replaces differing
entries. Overlay targets must be tracked in HEAD or they are skipped.

If you instead migrate the whole disk (or copy the entire `.git` directory),
shadow state comes along automatically -- no export/import needed.

### (e) Uninstall / cleanup

```bash
git-shadow remove <file>...   # unregister files first (overlays return to baseline)
git-shadow uninstall          # remove hooks, exclude entries, and .git/shadow/
# or, to wipe even with files still managed (restores overlay baselines):
git-shadow uninstall --force
```

## Cautions and troubleshooting

| Symptom | Action |
|---------|--------|
| Shadow changes get committed / hooks never run | `core.hooksPath` points elsewhere so the hooks are inert. Re-run `git-shadow install` (it installs into the effective hooks dir). `git-shadow doctor` flags this as an issue. |
| `git commit` blocked: "another git-shadow process still holds the lock" | A live commit/hook is running -- wait. If nothing is actually running the lock is stale; run `git-shadow restore`. |
| `git commit` blocked: leftover files in `.git/shadow/stash/` | A previous commit was interrupted. Run `git-shadow restore`, then commit again. |
| `git-shadow restore` refuses to run | The lock is held by a live process; let it finish. `restore` only cleans up stale (dead-PID) locks. |
| `git-shadow resume` blocked: file "was edited in the working tree while suspended" | You edited a suspended file. Review it, save what you want to keep, reconcile with `.git/shadow/suspended/`, then `git-shadow resume` again. |
| Import reported skipped files (non-zero exit) | Resolve the conflicting local edit (or accept overwriting with `--force`), then re-run `git-shadow import` -- it completes the remaining entries. |
| `git-shadow doctor` exits non-zero | It found issues. Read the `issues` list and fix each; warnings alone do not fail the exit code. |

## Safety rules for agents

- **Never** run destructive git commands (`git reset --hard`, `git checkout --`,
  deleting `.git/shadow/`) to "fix" shadow state. Use `git-shadow restore` /
  `rebase` / `resume`, which are designed to recover safely.
- **Run `git-shadow doctor` first** when diagnosing; act on its `issues`.
- **Do not commit with `git commit --no-verify`** in a shadow-managed repo -- it
  skips the pre-commit hook, so shadow changes get committed. Stage the whole
  file (`git add <file>`) rather than using `git add -p`; partial staging of an
  overlay blocks the commit by design.
- Prefer `status --json` / `doctor --json` for programmatic decisions; parse
  those stable keys, not the localized human output.
- Only text files are supported for overlays (rebase relies on text 3-way
  merge); binary files are rejected by `add`.

## Canonical help

The CLI itself is the source of truth for flags and messages:

```bash
git-shadow --help
git-shadow <subcommand> --help
```
