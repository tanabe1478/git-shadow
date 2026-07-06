# src/hooks/

Git hook handlers. These are called via `git-shadow hook <name>` from shell scripts installed in the effective hooks dir (`.git/hooks/`, or `core.hooksPath` when set).

## Hook Lifecycle

```
git commit
  -> .git/hooks/pre-commit
       -> git-shadow hook pre-commit    [pre_commit.rs]
  -> git writes the commit object
  -> .git/hooks/post-commit
       -> git-shadow hook post-commit   [post_commit.rs]

git pull / git merge
  -> .git/hooks/post-merge
       -> git-shadow hook post-merge    [post_merge.rs]

git commit --amend / git rebase / git filter-branch
  -> .git/hooks/post-rewrite
       -> git-shadow hook post-rewrite  [post_rewrite.rs]
```

## Design Notes

### pre_commit.rs: Transaction Pattern

The core of the tool. Uses `PreCommitTransaction` to track what has been modified for rollback capability:

```
1. Acquire lock
2. Hard checks (stash remnants, missing files, missing baselines)
3. Soft checks (baseline drift warning -- does not abort)
4. Partial staging detection (index != worktree for overlay files -> abort)
5. For each overlay:
   a. Stash current content (shadow) to .git/shadow/stash/
   b. Write baseline content to working tree
   c. git add (stage the baseline)
6. For each phantom:
   a. Stash current content
   b. git rm --cached / git restore --staged / git reset (unstage)
```

On any error in step 5-6, `tx.rollback()` restores all stashed files and re-stages overwritten files. The lock is NOT released on success -- post-commit handles that.

**Unstaging strategy for phantoms** (`git.unstage_phantom()`): Three strategies are tried in order because git behavior varies by version and state:
1. `git rm --cached --ignore-unmatch`
2. `git restore --staged`
3. `git reset -- <file>`

### post_commit.rs: Best-Effort Restore

Reads all files from `stash/`, writes them back to the working tree, and releases the lock. Failures are logged but do not abort -- partial restoration is better than losing everything. If any file fails, the lock is kept so `restore` can retry.

### post_merge.rs / post_rewrite.rs: Clean-Only Auto-Rebase

Both delegate to `rebase::auto_rebase_all(git, trigger)` (post-merge fires after `git pull`/`git merge`; post-rewrite after `git commit --amend`/`git rebase`). `auto_rebase_all` acquires the shadow lock -- if another process holds it (a commit in progress), or it is stale/corrupt, it skips with a warning rather than failing the git operation. Under the lock it 3-way-merges each overlay baseline up to the new HEAD in `RebaseMode::Auto`: clean merges update the baseline and working tree; conflicting ones are **deferred** (no conflict markers written, baseline left untouched) with a warning telling the user to run `git-shadow rebase` manually. Non-overlay entries and baselines already at HEAD are skipped.

## Critical Invariants

1. **Lock ownership**: pre-commit acquires, post-commit releases. If post-commit never runs (e.g., `--no-verify` or commit aborted), the lock becomes stale. `restore` and `doctor` handle this.
2. **Stash must be empty before pre-commit**: If stash has files, a previous commit was interrupted. Pre-commit refuses to run to prevent data loss.
3. **No partial staging**: If an overlay file has both staged and unstaged changes (`git add -p`), pre-commit aborts because it cannot safely determine which content to stash.
