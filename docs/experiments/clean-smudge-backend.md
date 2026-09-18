# Technical spike: clean/smudge filter backend

Status: **experimental prototype** on `devin/*-clean-smudge-spike`.
The hook backend is untouched; everything lives behind `git-shadow filter *`
and a new `tests/test_filter_backend.rs` suite (39 E2E tests).

This document is **judgment material, not a recommendation**. The adoption
decision stays with the maintainers.

---

## 1. How Git clean/smudge filters work (verified against git.git docs)

A *filter driver* is configured in `.git/config` (repo-local):

```
[filter "shadow"]
    clean    = git-shadow filter clean %f
    smudge   = git-shadow filter smudge %f
    required = true
```

and bound to paths via attributes — either `.gitattributes` (committed,
affects everyone) or **`$GIT_DIR/info/attributes`** (repo-local, affects only
this clone — the design used here, so teammates see nothing).

- **clean** runs whenever Git reads a worktree file into the index or compares
  it: `git add`, `git commit -a`, `git status`, `git diff`, `git diff-files`,
  `git stash`, `git rm`'s "local modifications" check, …
- **smudge** runs whenever Git writes a worktree file from a blob: `checkout`,
  `switch`, `merge` file updates, `rebase` replay, `stash pop`, `reset --hard`,
  `git worktree add`, `checkout -p` / `restore -p` hunk application.
- `required = true`: a non-zero filter exit aborts the Git command
  (`git add` → exit 128, nothing staged; `checkout` → aborts the switch).
- `required` unset + broken filter command → Git treats it as success and
  commits/passes content **unfiltered** — a silent leak vector.
- `%f` is the path relative to the worktree root; the filter process runs
  with cwd = worktree root (even when invoked from a subdirectory).
- `GIT_DIR` is **not** reliably set for filters — observed set only during
  `git worktree add`. The prototype discovers the repo from cwd via
  `git rev-parse` instead.
- `info/attributes` is read **only from the common dir**
  (`.git/info/attributes`); a per-worktree `worktrees/<name>/info/attributes`
  is created by `git worktree add` for other purposes but its attribute
  entries are **ignored** (verified experimentally, both directions).
- There is a long-running `filter.<driver>.process` protocol (single process,
  many files) used by Git LFS; the prototype uses per-invocation clean/smudge
  for simplicity.

## 2. Prototype design

The overlay model reduces to a content pair per file:

```
B = baseline blob      (clean domain — what the repo stores)
L = local content      (B + overlay edits)
R = repository blob at smudge time
W = worktree bytes
```

- `git-shadow filter add <file>`: requires tracked, non-binary, non-symlink,
  size-capped, **unstaged** file; stores `B := HEAD blob`, `L := current W`,
  adds `<path> filter=shadow` to the managed section of
  `common_dir/info/attributes`, then runs `git add` (which stores
  `clean(L) == B` and refreshes the index stat entry — without this, status
  shows the file as modified until the next add).
- `clean(W)` = merge_lines(base=L, side1=W, side2=B): the worktree diff and
  the overlay-reversion diff are merged in L-coordinates. Disjoint regions
  combine; genuine overlap on differing replacements → **refuse** (fail
  closed). Net effect: `clean(L+T) == B+T`.
- `smudge(R)` = merge_lines(base=B, side1=L, side2=R) with conflict markers:
  reapplies the overlay onto new repo content. `R == B` fast-path emits `L`
  (idempotent checkout). On merge it advances the pair to `(R, merged)` —
  this is the baseline-update step the hook backend does in post-merge.
- `merge_lines` is a precise op-level merge: `diff(base→side1)` and
  `diff(base→side2)` regions are reconciled in base coordinates, overlapping
  clusters recurse on their sub-range. This avoids diff3's adjacency
  conflicts (e.g. overlay on the last line + upstream EOF append merges
  cleanly where `git merge-file` conflicts).
- **Conflicted state**: smudge conflicts (or Git-level merge conflicts —
  detected by `<<<<<<<` markers inside `R`) emit marker output, keep the pair
  untouched, stash `pending_base`/`pending_local`, and set
  `entry.conflicted`. While conflicted, `clean` refuses every write attempt.
- `git-shadow filter resolve <file>`: after the user edits the resolved
  content **keeping the overlay lines**, sets `B := clean(resolved W)` and
  `L := resolved W`.
- `git-shadow filter sync <file>`: sets `L := current W` — the escape hatch
  for "this edit is part of the overlay".
- `git-shadow filter status`: per-file `commit would include +x -y` (B→clean(W))
  and `local overlay +x -y` (clean(W)→W) — a primitive `preview`.
- `filter install` / `uninstall` / `remove`: driver config in `.git/config`,
  managed section in `info/attributes`, state in `<git_dir>/shadow-filter/`
  (**per-worktree**, keyed off the per-worktree `git_dir` — see §5).

## 3. What works (all covered by `tests/test_filter_backend.rs`)

| Scenario | Result |
|---|---|
| Overlay-only change | Invisible to `status`/`diff`/`log`; `git add` stages nothing; commits store B |
| Overlay + team edit (headline case) | `add`/`commit` store `B+T`, worktree keeps `L+T` |
| `git add file` / `-A` / `.` | all strip the overlay |
| `git add -p` | **hunks presented and staged in the clean domain** — overlay lines never appear; staging a subset works |
| `git commit` / `-a` / `pathspec` / `--amend` | strip the overlay |
| **`git commit --no-verify`** | **cannot leak** — stripping happened at add-time; hooks are irrelevant |
| `git switch` (same/changed/absent file) | smudge reapplies the overlay on every checkout — **no suspend/resume needed** |
| `git checkout -- file` / `reset --hard` | restores `smudge(index/HEAD)` = overlay preserved, unstaged T dropped |
| `git checkout -p` / `restore -p` | hunks shown in clean domain; discarding writes back through smudge → overlay preserved |
| `git stash` / `pop` | stash entry stores `B+T` (verified via `stash@{0}:f.txt`); pop restores `L+T` |
| `git merge` disjoint upstream | worktree = `R+L`, commit = `R` — baseline auto-advances |
| `git merge` adjacent lines | merges (op-level merge beats diff3 adjacency) |
| `git merge`/`rebase` conflict | markers visible, staging refused, `filter resolve` → `add` → `commit`/`--continue` completes; pair re-bases correctly |
| `git rebase` clean replay | overlay preserved, HEAD stores clean content |
| `git worktree` | new worktree gets **plain** content (no state) until `filter add` there; per-worktree overlays verified |
| idempotency | `clean(smudge(R)) == R`, `smudge(clean(W)) == W`, clean idempotent, repeated `status` does not drift |

## 4. What does not work / sharp edges

1. **Overlay lines are locked.** Any worktree edit inside an overlay region
   that replaces the overlay text (rather than reverting it to B) makes
   `clean` conflict: it cannot tell "team change" from "new overlay".
   Resolution: keep overlay lines in the file, or `filter sync` (adopt as
   overlay), or `filter remove` → commit → re-add. Consequence: **a same-line
   upstream change to an overlay line can never be committed while the
   overlay exists** — the escape is `remove`+commit+`add`.
2. **`attributes` leftover without state = silent leak.** Delete
   `.git/shadow-filter/` but keep the attributes entry → clean passes
   through → overlay commits (asserted by test, documented). `uninstall`
   must therefore remove attributes and state together.
3. **Reverse leftovers are safe**: state without attributes entry → Git
   never invokes the filter → file is normal → `filter status` shows a stale
   entry (doctor territory).
4. **Smudge failure under `required` is sticky**: checkout aborts mid-switch;
   the user stays on the old branch until the filter works.
5. **`git apply --index` / plumbing bypass clean.** A patch whose context is
   written against *worktree* (overlay) text can stage divergent content.
   Patches must be generated from `git diff` (clean domain).
6. **Non-UTF8 / binary** files are refused at `filter add` and pass through in
   the filters — same limitation class as the hook backend's merge.
7. **State writes are unserialized** across concurrent Git processes
   (atomic_write + last-writer-wins); a concurrent `merge` smudge and
   `status` clean cannot corrupt bytes but could interleave pair updates.
8. **File deleted on one branch**: the state entry lingers; when the file
   reappears on another branch, smudge merges the stale overlay onto the new
   blob. Usually harmless; a `filter gc`/`doctor` pass is needed.
9. **`git rm` / `update-index --refresh` on a just-registered file** fail
   until the next `git add` refreshes the index stat entry — the prototype
   mitigates by running `git add` inside `filter add`.
10. **Filter spawn cost**: one process per clean/smudge per file (state load
    + `git rev-parse`). Fine at 1–10 files; production should use the
    `filter.<driver>.process` protocol or a cached stat fast path.
11. **`git archive`, `git cat-file`, plumbing reads** ignore filters —
    correct (they read blobs), but tools diffing blobs vs worktree
    byte-for-byte (editors' git gutters usually use `git diff` → fine) may
    show phantom diffs.

## 5. Worktree layout — verified, not guessed

| Item | Placement | Why |
|---|---|---|
| `info/attributes` entry | **common** (`common_dir/info/attributes`) | Git only reads it from there; per-worktree `worktrees/*/info/attributes` is ignored (verified) |
| `filter.shadow.*` config | **common** (`.git/config` — shared by all worktrees) | only `config.worktree` / `worktreeConfig` extension is per-worktree |
| state registry (`config.json`) | **per-worktree** (`<git_dir>/shadow-filter/`) | enables independent overlays per worktree — the desired semantics |
| `baselines/`, `local/`, `pending_*` | **per-worktree** | same |

Consequence: **every worktree sees the `filter=shadow` attribute**, so every
worktree invokes the filter for that path — the per-worktree *state* decides
whether the call is a passthrough (unmanaged) or a real transform. This is
what makes different overlays per worktree work, and also means a second
worktree can't opt a managed path out except by staying unregistered.

## 6. Hook backend vs clean/smudge backend

| Aspect | Hook backend (current) | Filter backend (prototype) |
|---|---|---|
| `git add` | worktree keeps overlay; stripping deferred to pre-commit | index gets `B+T` immediately; status/diff clean |
| `git add -p` | **unsupported** — pre-commit aborts (partial staging can't stash safely) | **works** — hunks live in clean domain |
| `git commit` | pre-commit stashes overlay → stages baseline → post-commit restores | nothing special; index already clean |
| `--no-verify` | **leaks the overlay** (pre-commit never runs) | **safe** — hooks skipped but index already holds `B+T` |
| `git status` | shows overlay files as modified | clean; overlay invisible |
| `git switch` | overlay file is "dirty" → blocks/suspend needed | smudge re-applies → suspend/resume unnecessary |
| merge/rebase | post-merge/post-rewrite re-base baselines | smudge advances the pair inline; conflicts get `conflicted` flag |
| conflict handling | 3-way merge to markers in worktree | markers + explicit resolve command; same-line overlay conflicts *cannot* be committed (locked-line) |
| worktree | per-worktree state; hooks shared | per-worktree state; attrs+config shared; passthrough via missing state |
| crash recovery | stash/lock/restore machinery | pair files + `conflicted` flag; smaller state surface |
| implementation complexity | hooks + lock + stash + rollback + suspend/resume + doctor checks | two transforms + small CLI; no locking needed so far |
| user mental model | "my files differ at commit time" | "Git sees a different file than I do" — cleaner, but the clean-domain/worktree split can confuse (`git diff` hides the overlay) |
| data-loss risk | post-commit restore failure paths; lock staleness | state-vs-attrs drift leaks (fail-open) if components desync; fail-closed on conflicts |
| agent friendliness | agents must know suspend/resume | agents use plain git; `filter status`/`preview` gives the clean view |

## 7. Data-loss risk assessment

- **Leak risks** (fail-open): attributes-without-state (§4.2);
  `required` missing (install writes it, but a hand-edit removes the
  guardrail); binary removed → add aborts under `required`, so it's noisy not
  silent.
- **Loss risks**: `checkout -- file`/`reset --hard` on an overlay file drops
  unstaged team edits but restores the overlay — correct; `filter sync` on a
  file with unresolved conflict is refused (guarded); deleting the state dir
  loses the overlay knowledge (recoverable: overlay is in the worktree).
- Net: the dangerous residual is **component desync** (attrs vs state vs
  config). A `doctor` integration should check all three pairwise.

## 8. UX sketches (design only, not implemented)

### `git shadow preview`

```
If you commit now:

Committed:
  src/foo.rs        +15 -3

Excluded by git-shadow:
  CLAUDE.md         +12 -1     (overlay)

Phantoms:
  .claude/
```

Computation (either backend): per managed file, `committed diff` =
`HEAD blob → clean(worktree)`; `excluded diff` = `clean(worktree) → worktree`
(hook backend: `clean()` stands in for "content that would be staged").
`--json`:

```json
{
  "committed": [{"path": "src/foo.rs", "added": 15, "removed": 3}],
  "excluded":  [{"path": "CLAUDE.md", "kind": "overlay", "added": 12, "removed": 1}],
  "phantoms":  [{"path": ".claude/", "kind": "phantom_dir"}],
  "conflicted": []
}
```

The `--json` shape is meant for coding agents deciding whether it is safe to
commit — a stable contract matters more than the exact keys.

### `git shadow doctor --fix`

Auto-fix only provably-safe issues; report the rest:

| Check | Auto-fix? |
|---|---|
| stale `stash/` remnant from an aborted commit | yes (move aside, never delete) |
| stale lock with dead PID | yes |
| attributes entry present but state dir missing (leak vector §4.2) | yes — **remove the attrs entry** and warn |
| state entry present but attrs entry missing | yes — regenerate entry |
| `filter.shadow.required` missing | yes — set it |
| hook scripts missing while managed entries exist | yes — reinstall |
| baseline missing for a managed file | no — report only (ambiguous intent) |
| `conflicted` flag set | no — report only |

## 9. What adoption would require

1. Decide coexistence: hook and filter backends can technically coexist
   (filters on some files, hooks on others) but a file must be managed by
   exactly one; `install` currently just warns.
2. `filter.<driver>.process` long-running protocol for spawn overhead.
3. `phantom` support is orthogonal (untracked files never hit clean/smudge)
   — the hook backend's exclude machinery stays regardless.
4. `doctor`/`preview`/`gc` integration; `status --json` contract.
5. A conflicted-file TUI/UX story: today it's `edit → resolve → add`.
6. Decide the locked-line policy (§4.1): refuse forever vs. a `sync --line`
   style surgical adopt.

## 10. Open questions returned for human judgment

| Question | Evidence gathered |
|---|---|
| Adopt clean/smudge? | §3 shows it works for the core flows; §4 shows the edge cost is component desync + locked lines |
| Coexist with hooks? | Technically compatible per-file; doubles the mental surface |
| Partial staging support? | **Yes** — `add -p` operates in clean domain (§3) |
| Overlay/team fully separable? | For disjoint and adjacent edits yes; same-line replacement is fundamentally ambiguous → refused |
| Per-worktree overlays natural? | **Yes** — common attrs + per-worktree state (§5) |
| Eliminate suspend/resume? | **Yes** — smudge reapplies on every checkout (§3) |
| `--no-verify` fundamentally solved? | **Yes** — stripping is add-time, not commit-time (§3) |
| Data-loss risk vs current? | Lower on commit-time paths (no stash/restore), higher on desync paths (§7); overall probably lower but needs the doctor guardrails |
