//! Experimental clean/smudge filter backend (prototype — does not replace the
//! hook backend).
//!
//! Instead of stashing local changes around `git commit`, this backend uses a
//! Git content filter: `info/attributes` marks managed files with
//! `filter=shadow`, and `filter.shadow.clean`/`filter.shadow.smudge` rewrite
//! blobs as they move between the worktree and the index/objects.
//!
//! Per managed file the backend stores a pair `(B, L)` under
//! `<git_dir>/shadow-filter/` (per-worktree):
//!
//! - `clean(W) = merge3(ours=W, base=L, theirs=B)` — strips the local overlay,
//!   keeps team edits. The index/commit only ever sees `B + T`.
//! - `smudge(R) = merge3(ours=L, base=B, theirs=R)` — re-applies the overlay on
//!   top of any checked-out repository version.
//!
//! Properties: `clean(smudge(R)) == R` and `smudge(clean(W)) == W` when the
//! overlay and upstream changes touch disjoint lines. `required = true` makes
//! filter failures abort the Git command instead of silently passing content
//! through (which would leak overlays into the index).

use std::io::{Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context};
use chrono::Utc;

use crate::cli::FilterCommands;
use crate::error::ShadowError;
use crate::exclude::to_exclude_pattern;
use crate::filter_state::{FilterFileEntry, FilterState, FilterStore};
use crate::fs_util;
use crate::git::GitRepo;
use crate::path::normalize_path;

const DRIVER: &str = "shadow";
const ATTRS_SECTION_START: &str = "# >>> git-shadow filter managed (DO NOT EDIT) >>>";
const ATTRS_SECTION_END: &str = "# <<< git-shadow filter managed <<<";

pub fn run(command: &FilterCommands) -> anyhow::Result<()> {
    match command {
        FilterCommands::Install => install(),
        FilterCommands::Uninstall { force } => uninstall(*force),
        FilterCommands::Add { files, force } => add(files, *force),
        FilterCommands::Remove { file } => remove(file),
        FilterCommands::Status => status(),
        FilterCommands::Sync { file } => sync(file),
        FilterCommands::Resolve { file } => resolve(file),
        FilterCommands::Clean { file } => clean(file),
        FilterCommands::Smudge { file } => smudge(file),
    }
}

// ---------------------------------------------------------------------------
// Managed section inside <common_dir>/info/attributes
// ---------------------------------------------------------------------------

struct AttributesSection {
    path: PathBuf,
}

impl AttributesSection {
    fn new(common_dir: &Path) -> Self {
        Self {
            path: common_dir.join("info").join("attributes"),
        }
    }

    fn entries(&self) -> Vec<String> {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut in_section = false;
        let mut entries = Vec::new();
        for line in content.lines() {
            if line == ATTRS_SECTION_START {
                in_section = true;
            } else if line == ATTRS_SECTION_END {
                in_section = false;
            } else if in_section && !line.trim().is_empty() {
                entries.push(line.trim().to_string());
            }
        }
        entries
    }

    fn set_entries(&self, entries: &[String]) -> anyhow::Result<()> {
        let original = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut in_section = false;
        let mut past_section = false;
        for line in original.lines() {
            if line == ATTRS_SECTION_START {
                in_section = true;
                continue;
            }
            if line == ATTRS_SECTION_END {
                in_section = false;
                past_section = true;
                continue;
            }
            if in_section {
                continue;
            }
            if past_section {
                after.push(line.to_string());
            } else {
                before.push(line.to_string());
            }
        }

        let mut result = before.join("\n");
        if !entries.is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(ATTRS_SECTION_START);
            result.push('\n');
            for entry in entries {
                result.push_str(entry);
                result.push('\n');
            }
            result.push_str(ATTRS_SECTION_END);
            result.push('\n');
        }
        if !after.is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&after.join("\n"));
            if !result.ends_with('\n') {
                result.push('\n');
            }
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fs_util::atomic_write(&self.path, result.as_bytes())
    }

    fn add(&self, entry: &str) -> anyhow::Result<()> {
        let mut entries = self.entries();
        if !entries.iter().any(|e| e == entry) {
            entries.push(entry.to_string());
        }
        self.set_entries(&entries)
    }

    fn remove(&self, entry: &str) -> anyhow::Result<()> {
        let mut entries = self.entries();
        entries.retain(|e| e != entry);
        self.set_entries(&entries)
    }
}

fn attr_entry(rel_path: &str) -> String {
    format!("{} filter={}", to_exclude_pattern(rel_path, false), DRIVER)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn repo() -> anyhow::Result<GitRepo> {
    GitRepo::discover(&std::env::current_dir().context("failed to get cwd")?)
}

fn git_config_set(git: &GitRepo, key: &str, value: &str) -> anyhow::Result<()> {
    let out = Command::new("git")
        .args(["config", key, value])
        .current_dir(&git.root)
        .output()
        .context("failed to run git config")?;
    if !out.status.success() {
        bail!(
            "git config {} failed: {}",
            key,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn git_config_unset_section(git: &GitRepo, section: &str) -> anyhow::Result<()> {
    let out = Command::new("git")
        .args(["config", "--remove-section", section])
        .current_dir(&git.root)
        .output()
        .context("failed to run git config")?;
    // Exit 5 = section does not exist; fine for idempotent uninstall.
    if !out.status.success() && out.status.code() != Some(5) {
        bail!(
            "git config --remove-section {} failed: {}",
            section,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// Read all of stdin (filter input: the blob or worktree content).
fn read_stdin() -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf)?;
    Ok(buf)
}

fn write_stdout(content: &[u8]) -> anyhow::Result<()> {
    let mut out = std::io::stdout();
    out.write_all(content)?;
    out.flush()?;
    Ok(())
}

fn to_text(content: &[u8], what: &str, path: &str) -> anyhow::Result<String> {
    String::from_utf8(content.to_vec()).with_context(|| {
        format!("{what} for '{path}' is not valid UTF-8 (filter prototype handles text only)")
    })
}

/// Resolve the filter-scoped view of this checkout: the per-worktree store plus
/// the entry for `path` if this worktree manages it.
fn load_entry(
    git: &GitRepo,
    path: &str,
) -> anyhow::Result<(FilterStore, FilterState, Option<FilterFileEntry>)> {
    let store = FilterStore::new(&git.git_dir);
    let state = store.load()?;
    let entry = state.files.get(path).cloned();
    Ok((store, state, entry))
}

// ---------------------------------------------------------------------------
// user-facing commands
// ---------------------------------------------------------------------------

fn install() -> anyhow::Result<()> {
    let git = repo()?;

    if git.hooks_installed() {
        eprintln!(
            "warning: git-shadow hooks are installed; the filter backend is independent. \
             Do not register the same file in both backends."
        );
    }

    // `current_exe` bakes an absolute path so filters do not depend on PATH.
    // `required = true` is load-bearing: without it Git silently passes content
    // through when the filter binary is missing or fails, which would leak
    // local overlay content into commits.
    let exe = std::env::current_exe().context("failed to resolve git-shadow path")?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    git_config_set(
        &git,
        "filter.shadow.clean",
        &format!("\"{exe}\" filter clean %f"),
    )?;
    git_config_set(
        &git,
        "filter.shadow.smudge",
        &format!("\"{exe}\" filter smudge %f"),
    )?;
    git_config_set(&git, "filter.shadow.required", "true")?;

    let store = FilterStore::new(&git.git_dir);
    std::fs::create_dir_all(&store.dir)?;
    store.save(&store.load()?)?;

    println!("git-shadow filter backend installed (experimental)");
    println!(
        "  driver : filter.{DRIVER}.* in {}",
        git.common_dir.join("config").display()
    );
    println!("  state  : {}", store.dir.display());
    println!("Next: git-shadow filter add <file>");
    Ok(())
}

fn uninstall(force: bool) -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    let state = store.load()?;

    if !state.files.is_empty() && !force {
        return Err(ShadowError::UninstallHasEntries(state.files.len()).into());
    }

    git_config_unset_section(&git, "filter.shadow")?;
    AttributesSection::new(&git.common_dir).set_entries(&[])?;
    for path in state.files.keys() {
        store.remove_file_state(path);
    }
    if store.exists() {
        std::fs::remove_dir_all(&store.dir)?;
    }

    println!("git-shadow filter backend removed");
    println!(
        "note: other worktrees may still have filter state under their own shadow-filter/ dirs"
    );
    Ok(())
}

fn add(files: &[String], force: bool) -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    if !store.exists() {
        bail!("filter backend not installed; run `git-shadow filter install` first");
    }

    let cwd = std::env::current_dir()?;
    let attrs = AttributesSection::new(&git.common_dir);
    let mut state = store.load()?;

    for file in files {
        let rel = normalize_path(file, &cwd, &git.root)?;

        if state.files.contains_key(&rel) {
            return Err(ShadowError::AlreadyManaged(rel).into());
        }
        if !git.is_tracked(&rel)? {
            bail!("'{rel}' is not tracked by git; filter backend only manages tracked files");
        }
        let worktree_path = git.root.join(&rel);
        if worktree_path.is_symlink() {
            bail!("'{rel}' is a symlink; the filter backend cannot manage symlinks");
        }
        fs_util::check_size(&worktree_path, force)?;
        if fs_util::is_binary(&worktree_path)? {
            bail!("'{rel}' looks binary; the filter backend handles text files only");
        }

        // The index must match HEAD so B (HEAD content) is what the next
        // `git add` reproduces. Staged-but-uncommitted content would be
        // silently reinterpreted by the filter.
        let (index_changed, _) = git.staging_status(&rel)?;
        if index_changed {
            bail!(
                "'{rel}' has staged changes; commit or unstage them before `filter add` \
                 (staged content cannot be cleanly separated from the overlay)"
            );
        }

        let baseline = git.show_file("HEAD", &rel)?;
        to_text(&baseline, "HEAD content", &rel)?;
        let local = std::fs::read(&worktree_path)?;
        to_text(&local, "worktree content", &rel)?;

        let enc = store.baseline_path(&rel);
        store.write_slot(&enc, &baseline)?;
        store.write_slot(&store.local_path(&rel), &local)?;

        state.files.insert(
            rel.clone(),
            FilterFileEntry {
                baseline_commit: git.head_commit().ok(),
                conflicted: false,
                added_at: Utc::now(),
            },
        );
        attrs.add(&attr_entry(&rel))?;
        store.save(&state)?;

        // Refresh the index stat entry: without this the worktree file's stat
        // no longer matches the index and porcelain commands (status/rm/diff)
        // report the file as modified until the next add.
        git.add(&rel)?;

        println!("registered '{rel}' (filter backend)");
        println!("  baseline B = HEAD blob; local overlay L = current worktree content");
        println!("  hint: run `git-shadow filter sync {rel}` after editing overlay lines");
    }
    Ok(())
}

fn remove(file: &str) -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    let cwd = std::env::current_dir()?;
    let rel = normalize_path(file, &cwd, &git.root)?;

    let mut state = store.load()?;
    if state.files.remove(&rel).is_none() {
        bail!("'{rel}' is not managed by the filter backend");
    }
    store.save(&state)?;
    store.remove_file_state(&rel);
    AttributesSection::new(&git.common_dir).remove(&attr_entry(&rel))?;

    println!("unregistered '{rel}' (filter backend)");
    println!("warning: the worktree file still contains the local overlay;");
    println!("         run `git checkout -- {rel}` to restore the repository version");
    Ok(())
}

/// Re-snapshot the overlay: set L to the current worktree content.
/// Use after editing lines that belong to the overlay — otherwise `clean`
/// treats edits touching the overlay region as ambiguous (and may fail).
fn sync(file: &str) -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    let cwd = std::env::current_dir()?;
    let rel = normalize_path(file, &cwd, &git.root)?;

    let mut state = store.load()?;
    let Some(entry) = state.files.get_mut(&rel) else {
        bail!("'{rel}' is not managed by the filter backend");
    };

    let local = std::fs::read(git.root.join(&rel))?;
    to_text(&local, "worktree content", &rel)?;
    store.write_slot(&store.local_path(&rel), &local)?;
    entry.conflicted = false;
    let _ = std::fs::remove_file(store.pending_base_path(&rel));
    let _ = std::fs::remove_file(store.pending_local_path(&rel));
    store.save(&state)?;
    println!("re-snapshotted overlay for '{rel}' (L := current worktree content)");
    Ok(())
}

/// Resolve a conflicted file: the worktree must now hold the resolved content
/// *keeping the local overlay lines where you want them*.
///
/// The new pair is (B := clean(resolved W) computed against the old pair,
/// L := resolved W). If the resolution touched the overlay region itself,
/// clean() cannot decide what belongs to the overlay and refuses — drop the
/// overlay (`filter remove`), commit, and re-add, or re-edit keeping the
/// overlay lines.
fn resolve(file: &str) -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    let cwd = std::env::current_dir()?;
    let rel = normalize_path(file, &cwd, &git.root)?;

    let mut state = store.load()?;
    let Some(entry) = state.files.get_mut(&rel) else {
        bail!("'{rel}' is not managed by the filter backend");
    };
    if !entry.conflicted {
        bail!("'{rel}' has no pending filter conflict");
    }

    let resolved = std::fs::read(git.root.join(&rel))?;
    to_text(&resolved, "resolved content", &rel)?;
    let l_old = store.read_slot(&store.local_path(&rel))?;
    to_text(&l_old, "overlay", &rel)?;
    let b_old = store.read_slot(&store.baseline_path(&rel))?;
    to_text(&b_old, "baseline", &rel)?;

    let new_b = clean_view(&l_old, &resolved, &b_old, &store).map_err(|_| {
        anyhow::anyhow!(
            "git-shadow filter: resolution for '{rel}' touched the overlay region — clean \
             cannot tell overlay from resolution. Keep the local overlay lines in the \
             resolved file, or `git-shadow filter remove {rel}` then commit and re-add."
        )
    })?;

    store.write_slot(&store.baseline_path(&rel), &new_b)?;
    store.write_slot(&store.local_path(&rel), &resolved)?;
    let _ = std::fs::remove_file(store.pending_base_path(&rel));
    let _ = std::fs::remove_file(store.pending_local_path(&rel));
    entry.conflicted = false;
    store.save(&state)?;
    println!("resolved '{rel}': pair re-based onto resolved worktree");
    Ok(())
}

fn status() -> anyhow::Result<()> {
    let git = repo()?;
    let store = FilterStore::new(&git.git_dir);
    let state = store.load()?;

    if state.files.is_empty() {
        println!("no files managed by the filter backend in this worktree");
        return Ok(());
    }

    for (rel, entry) in &state.files {
        println!("{rel}");
        if entry.conflicted {
            println!("  STATUS: conflicted — run `git-shadow filter resolve {rel}` after editing");
            continue;
        }
        let b = store.read_slot(&store.baseline_path(rel))?;
        let l = store.read_slot(&store.local_path(rel))?;
        let w = std::fs::read(git.root.join(rel)).unwrap_or_default();

        // "would commit" = clean(W) vs B ; "overlay" = W vs clean(W)
        let cleaned = clean_view(&l, &w, &b, &store)?;
        print_counts(&b, &cleaned, "commit would include");
        print_counts(&cleaned, &w, "local overlay       ");

        match git.show_file("HEAD", rel) {
            Ok(head) if head != b => println!("  note: HEAD moved past stored baseline"),
            Err(_) => println!("  note: file not present in HEAD"),
            _ => {}
        }
    }
    Ok(())
}

fn print_counts(old: &[u8], new: &[u8], label: &str) {
    let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    let diff = similar::TextDiff::from_lines(&old, &new);
    let mut adds = 0;
    let mut dels = 0;
    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Insert => adds += op.new_range().len(),
            similar::DiffTag::Delete => dels += op.old_range().len(),
            _ => {}
        }
    }
    println!("  {label}: +{adds} -{dels}");
}

// ---------------------------------------------------------------------------
// filter entry points (invoked by Git; stdin -> stdout)
// ---------------------------------------------------------------------------

/// clean(W): strip the overlay — merge3(ours=W, base=L, theirs=B).
///
/// Fail-closed: any ambiguity (conflicted state, unreadable state, non-UTF8,
/// merge conflict) exits non-zero. With `filter.shadow.required = true` Git
/// then aborts the operation instead of committing overlay content.
fn clean(file: &str) -> anyhow::Result<()> {
    let input = read_stdin()?;
    let git = match repo() {
        Ok(g) => g,
        Err(_) => return write_stdout(&input), // not a repo: pass through
    };
    let (store, _state, entry) = load_entry(&git, file)?;
    let Some(entry) = entry else {
        return write_stdout(&input); // not managed in this worktree
    };

    if entry.conflicted {
        bail!(
            "git-shadow filter: '{file}' has an unresolved overlay conflict. \
             Edit the file, then run `git-shadow filter resolve {file}`"
        );
    }

    let b = store.read_slot(&store.baseline_path(file))?;
    let l = store.read_slot(&store.local_path(file))?;
    let cleaned = clean_view(&l, &input, &b, &store)?;
    write_stdout(&cleaned)
}

/// A single changed region, expressed in coordinates of the common base.
/// `new` holds the replacement lines taken from the other file.
#[derive(Debug)]
struct Region {
    /// [start, end) line range in the base
    old: std::ops::Range<usize>,
    /// replacement lines taken from the other file
    new: Vec<String>,
    /// which side produced this region (0 = side1, 1 = side2)
    side: u8,
}

fn diff_regions(base_lines: &[&str], other_lines: &[&str], side: u8) -> Vec<Region> {
    let diff = similar::TextDiff::from_slices(base_lines, other_lines);
    let mut regions = Vec::new();
    for op in diff.ops() {
        if op.tag() == similar::DiffTag::Equal {
            continue;
        }
        regions.push(Region {
            old: op.old_range(),
            new: other_lines[op.new_range()]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            side,
        });
    }
    regions
}

/// Do two regions overlap in base coordinates? A zero-width insertion overlaps
/// an enclosing region only when strictly inside it (boundary is fine).
fn regions_overlap(r1: &Region, r2: &Region) -> bool {
    let (a1, b1) = (r1.old.start, r1.old.end);
    let (a2, b2) = (r2.old.start, r2.old.end);
    if a1 == b1 || a2 == b2 {
        let (p, a, b) = if a1 == b1 { (a1, a2, b2) } else { (a2, a1, b1) };
        return a < p && p < b;
    }
    a1 < b2 && a2 < b1
}

/// Apply one side's (sorted, disjoint) regions to a base line slice.
fn apply_regions(base: &[&str], regions: &[&Region]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in regions {
        out.extend(base[pos..r.old.start].iter().map(|s| (*s).to_string()));
        out.extend(r.new.iter().cloned());
        pos = r.old.end;
    }
    out.extend(base[pos..].iter().map(|s| (*s).to_string()));
    out
}

const MAX_MERGE_DEPTH: u32 = 16;

/// Precise line-level 3-way merge.
///
/// Instead of diff3-style hunk merging (which conservatively conflicts on
/// *adjacent* hunks), this computes `diff(base -> side1)` and
/// `diff(base -> side2)` independently, then reconciles the changed regions in
/// base coordinates:
///
/// - regions from one side apply verbatim;
/// - regions from both sides that *genuinely overlap* (share base lines) are
///   resolved recursively — the recursion re-diffs the smaller base range,
///   so adjacent edits like "overlay touches last line, team appends after it"
///   merge cleanly;
/// - a recursion that makes no progress is a true conflict.
///
/// When `markers` is Some(("LEFT", "RIGHT")), conflicts emit conflict-marker
/// blocks and the returned flag is true; when None, the first conflict returns
/// Err.
fn merge_lines(
    base: &[&str],
    side1: &[&str],
    side2: &[&str],
    markers: Option<(&str, &str)>,
    depth: u32,
) -> Result<(Vec<String>, bool), ()> {
    let mut regions: Vec<Region> = diff_regions(base, side1, 0);
    regions.extend(diff_regions(base, side2, 1));
    regions.sort_by_key(|r| (r.old.start, r.old.end, r.side));

    // Cluster transitively-overlapping regions.
    let mut clusters: Vec<Vec<Region>> = Vec::new();
    for r in regions {
        if let Some(last) = clusters.last_mut() {
            if last.iter().any(|e| regions_overlap(e, &r)) {
                last.push(r);
                continue;
            }
        }
        clusters.push(vec![r]);
    }

    let mut out: Vec<String> = Vec::new();
    let mut pos = 0;
    let mut had_conflict = false;

    for cluster in clusters {
        let cstart = cluster.first().unwrap().old.start;
        let cend = cluster.iter().map(|r| r.old.end).max().unwrap();
        out.extend(base[pos..cstart].iter().map(|s| (*s).to_string()));
        pos = cend;

        let single_side =
            cluster.iter().all(|r| r.side == 0) || cluster.iter().all(|r| r.side == 1);
        if single_side {
            let slice = &base[cstart..cend];
            let shifted: Vec<Region> = cluster
                .iter()
                .map(|r| Region {
                    old: (r.old.start - cstart)..(r.old.end - cstart),
                    new: r.new.clone(),
                    side: r.side,
                })
                .collect();
            let refs: Vec<&Region> = shifted.iter().collect();
            out.extend(apply_regions(slice, &refs));
            continue;
        }

        // Both sides touched overlapping base lines. Recurse on the cluster's
        // base range: each side's view is its ops applied to that range.
        let slice = &base[cstart..cend];
        let side_view = |want: u8| -> Vec<String> {
            let shifted: Vec<Region> = cluster
                .iter()
                .filter(|r| r.side == want)
                .map(|r| Region {
                    old: (r.old.start - cstart)..(r.old.end - cstart),
                    new: r.new.clone(),
                    side: want,
                })
                .collect();
            let refs: Vec<&Region> = shifted.iter().collect();
            apply_regions(slice, &refs)
        };
        let v1 = side_view(0);
        let v2 = side_view(1);

        if v1 == v2 {
            out.extend(v1);
            continue;
        }

        // Progress check: if the cluster spans the whole base and both sides'
        // views equal their inputs, recursion cannot shrink the problem.
        let sub_base: Vec<&str> = slice.to_vec();
        let v1_refs: Vec<&str> = v1.iter().map(|s| s.as_str()).collect();
        let v2_refs: Vec<&str> = v2.iter().map(|s| s.as_str()).collect();
        let no_progress = cstart == 0 && cend == base.len() && v1_refs == side1 && v2_refs == side2;
        if depth >= MAX_MERGE_DEPTH || no_progress {
            match markers {
                Some((l1, l2)) => {
                    had_conflict = true;
                    out.push(format!("<<<<<<< {l1}"));
                    out.extend(v1);
                    out.push("=======".to_string());
                    out.extend(v2);
                    out.push(format!(">>>>>>> {l2}"));
                }
                None => return Err(()),
            }
            continue;
        }

        match merge_lines(&sub_base, &v1_refs, &v2_refs, markers, depth + 1) {
            Ok((lines, conflicted)) => {
                had_conflict |= conflicted;
                out.extend(lines);
            }
            Err(()) => return Err(()),
        }
    }
    out.extend(base[pos..].iter().map(|s| (*s).to_string()));
    Ok((out, had_conflict))
}

fn join_lines(lines: &[String], trailing_nl: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_nl && !s.is_empty() {
        s.push('\n');
    }
    s
}

/// clean transform: strip the overlay while keeping every other edit.
/// Base = L; side1 = W (user edits to keep); side2 = B (overlay reversion).
fn clean_lines(l: &str, w: &str, b: &str) -> Result<String, ()> {
    let l_lines: Vec<&str> = l.lines().collect();
    let w_lines: Vec<&str> = w.lines().collect();
    let b_lines: Vec<&str> = b.lines().collect();

    let (lines, conflicted) = merge_lines(&l_lines, &w_lines, &b_lines, None, 0)?;
    debug_assert!(!conflicted);
    // Trailing-newline shape follows B unless the worktree agrees otherwise.
    let nl = if w.ends_with('\n') == b.ends_with('\n') {
        w.ends_with('\n')
    } else {
        b.ends_with('\n')
    };
    Ok(join_lines(&lines, nl))
}

/// The clean transform shared by `clean` and `status`: W -> B + T.
fn clean_view(l: &[u8], w: &[u8], b: &[u8], _store: &FilterStore) -> anyhow::Result<Vec<u8>> {
    let l = String::from_utf8(l.to_vec()).context("stored local content is not UTF-8")?;
    let w = String::from_utf8(w.to_vec())
        .context("worktree content is not valid UTF-8; cannot clean safely")?;
    let b = String::from_utf8(b.to_vec()).context("stored baseline is not UTF-8")?;
    match clean_lines(&l, &w, &b) {
        Ok(out) => Ok(out.into_bytes()),
        Err(()) => bail!(
            "git-shadow filter: clean conflict — the edit overlaps the local overlay region. \
             Either `git-shadow filter sync` to grow the overlay, or undo the edit"
        ),
    }
}

/// smudge(R): re-apply the overlay — merge3(ours=L, base=B, theirs=R).
///
/// On conflict the marker-laden merge result is written to the worktree and the
/// file is marked `conflicted`; `clean` refuses until `filter resolve` runs.
/// This keeps every Git operation honest (nothing is silently dropped) at the
/// cost of surfacing non-Git conflicts inside the file.
fn smudge(file: &str) -> anyhow::Result<()> {
    let input = read_stdin()?;
    let git = match repo() {
        Ok(g) => g,
        Err(_) => return write_stdout(&input),
    };
    let (store, mut state, entry) = load_entry(&git, file)?;
    let Some(entry) = entry else {
        return write_stdout(&input);
    };

    let b = store.read_slot(&store.baseline_path(file))?;
    let l = store.read_slot(&store.local_path(file))?;

    if entry.conflicted {
        let pending = store
            .read_slot(&store.pending_base_path(file))
            .unwrap_or_default();
        if pending == input {
            // Same conflicted blob again: re-emit the recorded conflicted content.
            if let Ok(content) = store.read_slot(&store.pending_local_path(file)) {
                return write_stdout(&content);
            }
        }
        // Blob moved on (e.g. re-checkout of a different commit): retry the merge.
        return smudge_merge(&store, &mut state, file, &l, &b, &input);
    }

    if input == b {
        return write_stdout(&l); // fast path: same repo content -> local view
    }
    smudge_merge(&store, &mut state, file, &l, &b, &input)
}

/// merge_lines(base=B, side1=L, side2=R): apply the overlay onto repo
/// content R. On clean merge the stored pair advances to (R, merged). On
/// conflict the marker-laden result is emitted, the pair stays put, and the
/// file is marked conflicted until `filter resolve` re-bases it.
fn smudge_merge(
    store: &FilterStore,
    state: &mut FilterState,
    file: &str,
    l: &[u8],
    b: &[u8],
    r: &[u8],
) -> anyhow::Result<()> {
    let Ok(l) = String::from_utf8(l.to_vec()) else {
        return write_stdout(r); // cannot merge non-UTF8: keep repo content
    };
    let Ok(b) = String::from_utf8(b.to_vec()) else {
        return write_stdout(r);
    };
    let Ok(r_text) = String::from_utf8(r.to_vec()) else {
        return write_stdout(r);
    };

    // If R itself is a merge-conflict product (Git merging a managed file
    // hit a content conflict), do not merge or advance the pair: emit R
    // verbatim so standard markers stay visible, and mark the file conflicted
    // so `filter resolve` re-bases it once the user fixes the file.
    let r_has_markers = r_text
        .lines()
        .any(|line| line.starts_with("<<<<<<<") || line.starts_with(">>>>>>>"));
    let (lines, had_conflict) = if r_has_markers {
        (r_text.lines().map(str::to_string).collect(), true)
    } else {
        let b_lines: Vec<&str> = b.lines().collect();
        let l_lines: Vec<&str> = l.lines().collect();
        let r_lines: Vec<&str> = r_text.lines().collect();
        merge_lines(
            &b_lines,
            &l_lines,
            &r_lines,
            Some(("local overlay", "repository")),
            0,
        )
        .expect("marker mode never returns Err")
    };
    let content = join_lines(&lines, r_text.ends_with('\n')).into_bytes();

    if let Some(entry) = state.files.get_mut(file) {
        if had_conflict {
            entry.conflicted = true;
            store.write_slot(&store.pending_base_path(file), r)?;
            store.write_slot(&store.pending_local_path(file), &content)?;
            eprintln!(
                "git-shadow filter: overlay conflict in '{file}' — worktree contains merge \
                 markers; resolve them and run `git-shadow filter resolve {file}`"
            );
        } else {
            entry.conflicted = false;
            store.write_slot(&store.baseline_path(file), r)?;
            store.write_slot(&store.local_path(file), &content)?;
            let _ = std::fs::remove_file(store.pending_base_path(file));
            let _ = std::fs::remove_file(store.pending_local_path(file));
        }
        store.save(state)?;
    }
    write_stdout(&content)
}
