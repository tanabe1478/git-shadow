use std::collections::BTreeMap;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::archive::{self, Manifest, ManifestEntry, ManifestType};
use crate::config::{ExcludeMode, FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::exclude::{self, ExcludeManager};
use crate::fs_util;
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::merge;
use crate::path;
use crate::ui::{self, UiLocale};

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Imported,
    Skipped,
}

pub fn run(archive_path: String, force: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let cwd = std::env::current_dir()?;
    let git = GitRepo::discover(&cwd)?;

    // Must be installed: gives NotInitialized ("Run `git-shadow install`") otherwise.
    git.ensure_initialized()?;

    let mut config = ShadowConfig::load(&git.shadow_dir)?;
    guard_importable(&git, &config)?;

    let (manifest, contents) = archive::read_archive(std::path::Path::new(&archive_path))?;
    if manifest.format_version != archive::FORMAT_VERSION {
        return Err(ShadowError::UnsupportedExportVersion(manifest.format_version).into());
    }

    let (imported, skipped, touched_exclude) =
        apply_manifest(&git, &mut config, &manifest, &contents, force, locale)?;

    config.save(&git.shadow_dir)?;

    // Regenerate the shared exclude section from the union of all worktrees' configs,
    // reusing add's machinery rather than duplicating pattern logic.
    if touched_exclude {
        let manager = ExcludeManager::new(&git.common_dir);
        let patterns = exclude::union_patterns(&git, &config);
        manager
            .set_entries(&patterns)
            .context("failed to update .git/info/exclude")?;
    }

    println!("{}", ui::import_summary(locale, imported, skipped).green());

    if skipped > 0 {
        return Err(ShadowError::ImportSomeSkipped(skipped).into());
    }
    Ok(())
}

/// Refuse to import while a commit cycle is mid-flight or shadow is suspended.
fn guard_importable(git: &GitRepo, config: &ShadowConfig) -> Result<()> {
    if config.suspended {
        return Err(ShadowError::Suspended.into());
    }

    let stash_dir = git.shadow_dir.join("stash");
    if stash_dir.exists() {
        let has_files = std::fs::read_dir(&stash_dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            })
            .unwrap_or(false);
        if has_files {
            return Err(ShadowError::StashRemaining.into());
        }
    }

    if let Ok(LockStatus::HeldByOther(info)) = lock::check_lock(&git.shadow_dir) {
        return Err(ShadowError::LockHeld {
            pid: info.pid,
            timestamp: info.timestamp.to_rfc3339(),
        }
        .into());
    }

    Ok(())
}

/// Apply every manifest entry. Returns (imported, skipped, whether exclude needs a rewrite).
fn apply_manifest(
    git: &GitRepo,
    config: &mut ShadowConfig,
    manifest: &Manifest,
    contents: &BTreeMap<String, Vec<u8>>,
    force: bool,
    locale: UiLocale,
) -> Result<(usize, usize, bool)> {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut touched_exclude = false;

    for entry in &manifest.entries {
        let outcome = match entry.entry_type {
            ManifestType::Overlay => import_overlay(git, config, entry, contents, force, locale)?,
            ManifestType::Phantom => {
                let outcome = import_phantom_file(git, config, entry, contents, force, locale)?;
                if outcome == Outcome::Imported && entry.exclude_mode == ExcludeMode::GitInfoExclude
                {
                    touched_exclude = true;
                }
                outcome
            }
            ManifestType::PhantomDir => {
                let outcome = import_phantom_dir(git, config, entry, contents, force, locale)?;
                if outcome == Outcome::Imported && entry.exclude_mode == ExcludeMode::GitInfoExclude
                {
                    touched_exclude = true;
                }
                outcome
            }
        };

        match outcome {
            Outcome::Imported => imported += 1,
            Outcome::Skipped => skipped += 1,
        }
    }

    Ok((imported, skipped, touched_exclude))
}

fn import_overlay(
    git: &GitRepo,
    config: &mut ShadowConfig,
    entry: &ManifestEntry,
    contents: &BTreeMap<String, Vec<u8>>,
    force: bool,
    locale: UiLocale,
) -> Result<Outcome> {
    let path = &entry.path;

    let (Some(shadow), Some(archived_baseline)) = (
        contents.get(&archive::overlay_shadow_key(path)),
        contents.get(&archive::overlay_baseline_key(path)),
    ) else {
        eprintln!("{}", ui::import_missing_content(locale, path).yellow());
        return Ok(Outcome::Skipped);
    };

    // Already managed as something else: refuse unless forced.
    if let Some(existing) = config.get(path) {
        if existing.file_type != FileType::Overlay && !force {
            eprintln!("{}", ui::import_skip_already_managed(locale, path).yellow());
            return Ok(Outcome::Skipped);
        }
    }

    // Overlay target must be tracked in HEAD; otherwise the repo doesn't match the export.
    let head = match git.show_file("HEAD", path) {
        Ok(content) => content,
        Err(_) => {
            eprintln!(
                "{}",
                ui::import_skip_overlay_untracked(locale, path).yellow()
            );
            return Ok(Outcome::Skipped);
        }
    };

    // Decide the working-tree content to write.
    let mut merged = false;
    let target: Vec<u8> = if head == *archived_baseline {
        shadow.clone()
    } else {
        let base = String::from_utf8_lossy(archived_baseline);
        let ours = String::from_utf8_lossy(shadow);
        let theirs = String::from_utf8_lossy(&head);
        let result = merge::three_way_merge(&base, &ours, &theirs, &git.shadow_dir)?;
        if result.has_conflicts {
            if force {
                // --force: keep the shadow version, discarding the conflicting upstream hunk.
                shadow.clone()
            } else {
                eprintln!(
                    "{}",
                    ui::import_skip_overlay_conflict(locale, path).yellow()
                );
                return Ok(Outcome::Skipped);
            }
        } else {
            merged = true;
            result.content.into_bytes()
        }
    };

    // Safety: don't clobber unrelated local modifications in the working tree.
    let worktree_path = git.root.join(path);
    if let Ok(current) = std::fs::read(&worktree_path) {
        if current != head && current != target && !force {
            eprintln!(
                "{}",
                ui::import_skip_overlay_worktree_modified(locale, path).yellow()
            );
            return Ok(Outcome::Skipped);
        }
    }

    // Baseline must represent the CURRENT HEAD (the invariant pre-commit relies on).
    let encoded = path::encode_path(path);
    let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
    fs_util::atomic_write(&baseline_path, &head)
        .with_context(|| format!("failed to write baseline for {}", path))?;

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory for {}", path))?;
    }
    std::fs::write(&worktree_path, &target).with_context(|| format!("failed to write {}", path))?;

    let commit = git.head_commit()?;
    config.files.remove(path);
    config.add_overlay(path.clone(), commit)?;

    if merged {
        println!("{}", ui::import_merged_overlay(locale, path));
    } else {
        println!("{}", ui::import_imported_overlay(locale, path));
    }
    Ok(Outcome::Imported)
}

fn import_phantom_file(
    git: &GitRepo,
    config: &mut ShadowConfig,
    entry: &ManifestEntry,
    contents: &BTreeMap<String, Vec<u8>>,
    force: bool,
    locale: UiLocale,
) -> Result<Outcome> {
    let path = &entry.path;

    let Some(content) = contents.get(&archive::phantom_key(path)) else {
        eprintln!("{}", ui::import_missing_content(locale, path).yellow());
        return Ok(Outcome::Skipped);
    };

    if let Some(existing) = config.get(path) {
        let same_kind = existing.file_type == FileType::Phantom && !existing.is_directory;
        if !same_kind && !force {
            eprintln!("{}", ui::import_skip_already_managed(locale, path).yellow());
            return Ok(Outcome::Skipped);
        }
    }

    let worktree_path = git.root.join(path);
    if let Ok(current) = std::fs::read(&worktree_path) {
        if current != *content && !force {
            eprintln!(
                "{}",
                ui::import_skip_phantom_conflict(locale, path).yellow()
            );
            return Ok(Outcome::Skipped);
        }
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory for {}", path))?;
    }
    std::fs::write(&worktree_path, content).with_context(|| format!("failed to write {}", path))?;

    config.files.remove(path);
    config.add_phantom(path.clone(), entry.exclude_mode.clone(), false)?;

    println!("{}", ui::import_imported_phantom(locale, path));
    Ok(Outcome::Imported)
}

fn import_phantom_dir(
    git: &GitRepo,
    config: &mut ShadowConfig,
    entry: &ManifestEntry,
    contents: &BTreeMap<String, Vec<u8>>,
    force: bool,
    locale: UiLocale,
) -> Result<Outcome> {
    let path = &entry.path;
    let members = entry.dir_members.clone().unwrap_or_default();

    // First pass: verify content is present and detect conflicts with existing files.
    let mut any_conflict = false;
    for member in &members {
        let Some(content) = contents.get(&archive::phantom_dir_member_key(member)) else {
            eprintln!("{}", ui::import_missing_content(locale, member).yellow());
            return Ok(Outcome::Skipped);
        };
        let member_path = git.root.join(member);
        if let Ok(current) = std::fs::read(&member_path) {
            if current != *content {
                any_conflict = true;
            }
        }
    }

    if any_conflict && !force {
        eprintln!(
            "{}",
            ui::import_skip_phantom_conflict(locale, path).yellow()
        );
        return Ok(Outcome::Skipped);
    }

    if let Some(existing) = config.get(path) {
        let same_kind = existing.file_type == FileType::Phantom && existing.is_directory;
        if !same_kind && !force {
            eprintln!("{}", ui::import_skip_already_managed(locale, path).yellow());
            return Ok(Outcome::Skipped);
        }
    }

    // Second pass: write every member.
    for member in &members {
        let content = contents
            .get(&archive::phantom_dir_member_key(member))
            .expect("member content verified in first pass");
        let member_path = git.root.join(member);
        if let Some(parent) = member_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create parent directory for {}", member))?;
        }
        std::fs::write(&member_path, content)
            .with_context(|| format!("failed to write {}", member))?;
    }

    config.files.remove(path);
    config.add_phantom(path.clone(), entry.exclude_mode.clone(), true)?;

    println!(
        "{}",
        ui::import_imported_phantom_dir(locale, path, members.len())
    );
    Ok(Outcome::Imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "t@t.com"],
        ] {
            std::process::Command::new("git")
                .args(&args)
                .current_dir(&root)
                .output()
                .unwrap();
        }
        std::fs::write(root.join("CLAUDE.md"), "# Team\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "CLAUDE.md"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&root)
            .output()
            .unwrap();

        let repo = GitRepo::discover(&root).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("baselines")).unwrap();
        std::fs::create_dir_all(repo.shadow_dir.join("stash")).unwrap();
        (dir, repo)
    }

    fn overlay_entry() -> ManifestEntry {
        ManifestEntry {
            path: "CLAUDE.md".to_string(),
            entry_type: ManifestType::Overlay,
            exclude_mode: ExcludeMode::None,
            baseline_commit: Some("oldcommit".to_string()),
            dir_members: None,
        }
    }

    #[test]
    fn test_import_overlay_fast_path_writes_shadow() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        // HEAD content is "# Team\n"; archived baseline matches -> fast path.
        contents.insert(
            archive::overlay_baseline_key("CLAUDE.md"),
            b"# Team\n".to_vec(),
        );
        contents.insert(
            archive::overlay_shadow_key("CLAUDE.md"),
            b"# Team\n# shadow\n".to_vec(),
        );

        let outcome = import_overlay(
            &git,
            &mut config,
            &overlay_entry(),
            &contents,
            false,
            UiLocale::En,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Imported);

        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(wt, "# Team\n# shadow\n");

        // Baseline is regenerated from current HEAD, commit set to current HEAD.
        let baseline = std::fs::read_to_string(
            git.shadow_dir
                .join("baselines")
                .join(path::encode_path("CLAUDE.md")),
        )
        .unwrap();
        assert_eq!(baseline, "# Team\n");
        let entry = config.get("CLAUDE.md").unwrap();
        assert_eq!(
            entry.baseline_commit.as_deref(),
            Some(git.head_commit().unwrap().as_str())
        );
    }

    #[test]
    fn test_import_overlay_merges_upstream_change() {
        let (_dir, git) = make_test_repo();
        // Establish a multi-line base so changes can be genuinely non-overlapping.
        let base = "l1\nl2\nl3\nl4\nl5\n";
        std::fs::write(git.root.join("CLAUDE.md"), base).unwrap();
        std::process::Command::new("git")
            .args(["commit", "-am", "base"])
            .current_dir(&git.root)
            .output()
            .unwrap();
        // Upstream changes the last line only.
        std::fs::write(git.root.join("CLAUDE.md"), "l1\nl2\nl3\nl4\nl5 upstream\n").unwrap();
        std::process::Command::new("git")
            .args(["commit", "-am", "upstream"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(
            archive::overlay_baseline_key("CLAUDE.md"),
            base.as_bytes().to_vec(),
        );
        // Shadow changes the first line only.
        contents.insert(
            archive::overlay_shadow_key("CLAUDE.md"),
            b"l1 shadow\nl2\nl3\nl4\nl5\n".to_vec(),
        );

        let outcome = import_overlay(
            &git,
            &mut config,
            &overlay_entry(),
            &contents,
            false,
            UiLocale::En,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Imported);

        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert!(
            wt.contains("l5 upstream"),
            "should keep upstream change: {wt}"
        );
        assert!(wt.contains("l1 shadow"), "should keep shadow change: {wt}");
    }

    #[test]
    fn test_import_overlay_conflict_skips() {
        let (_dir, git) = make_test_repo();
        // Overlapping upstream change on the same line as the shadow edit.
        std::fs::write(git.root.join("CLAUDE.md"), "# Team upstream\n").unwrap();
        std::process::Command::new("git")
            .args(["commit", "-am", "upstream"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(
            archive::overlay_baseline_key("CLAUDE.md"),
            b"# Team\n".to_vec(),
        );
        contents.insert(
            archive::overlay_shadow_key("CLAUDE.md"),
            b"# Team shadow\n".to_vec(),
        );

        let outcome = import_overlay(
            &git,
            &mut config,
            &overlay_entry(),
            &contents,
            false,
            UiLocale::En,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Skipped);
        assert!(config.get("CLAUDE.md").is_none());
        // No conflict markers written to the working tree.
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert!(!wt.contains("<<<<<<<"));
    }

    #[test]
    fn test_import_overlay_conflict_force_keeps_shadow() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("CLAUDE.md"), "# Team upstream\n").unwrap();
        std::process::Command::new("git")
            .args(["commit", "-am", "upstream"])
            .current_dir(&git.root)
            .output()
            .unwrap();

        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(
            archive::overlay_baseline_key("CLAUDE.md"),
            b"# Team\n".to_vec(),
        );
        contents.insert(
            archive::overlay_shadow_key("CLAUDE.md"),
            b"# Team shadow\n".to_vec(),
        );

        let outcome = import_overlay(
            &git,
            &mut config,
            &overlay_entry(),
            &contents,
            true,
            UiLocale::En,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Imported);
        let wt = std::fs::read_to_string(git.root.join("CLAUDE.md")).unwrap();
        assert_eq!(wt, "# Team shadow\n");
    }

    #[test]
    fn test_import_overlay_untracked_skips() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        let entry = ManifestEntry {
            path: "not-in-repo.md".to_string(),
            entry_type: ManifestType::Overlay,
            exclude_mode: ExcludeMode::None,
            baseline_commit: Some("x".to_string()),
            dir_members: None,
        };
        contents.insert(
            archive::overlay_baseline_key("not-in-repo.md"),
            b"a\n".to_vec(),
        );
        contents.insert(
            archive::overlay_shadow_key("not-in-repo.md"),
            b"b\n".to_vec(),
        );

        let outcome =
            import_overlay(&git, &mut config, &entry, &contents, false, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Skipped);
    }

    #[test]
    fn test_import_phantom_file_new_and_idempotent() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(archive::phantom_key("local.md"), b"# Local\n".to_vec());
        let entry = ManifestEntry {
            path: "local.md".to_string(),
            entry_type: ManifestType::Phantom,
            exclude_mode: ExcludeMode::GitInfoExclude,
            baseline_commit: None,
            dir_members: None,
        };

        let outcome =
            import_phantom_file(&git, &mut config, &entry, &contents, false, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Imported);
        assert_eq!(
            std::fs::read_to_string(git.root.join("local.md")).unwrap(),
            "# Local\n"
        );

        // Second import: identical content -> idempotent, still Imported.
        let outcome =
            import_phantom_file(&git, &mut config, &entry, &contents, false, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Imported);
    }

    #[test]
    fn test_import_phantom_file_conflict_skips_without_force() {
        let (_dir, git) = make_test_repo();
        std::fs::write(git.root.join("local.md"), "# Different\n").unwrap();

        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(archive::phantom_key("local.md"), b"# Local\n".to_vec());
        let entry = ManifestEntry {
            path: "local.md".to_string(),
            entry_type: ManifestType::Phantom,
            exclude_mode: ExcludeMode::GitInfoExclude,
            baseline_commit: None,
            dir_members: None,
        };

        let outcome =
            import_phantom_file(&git, &mut config, &entry, &contents, false, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Skipped);
        // Existing file untouched.
        assert_eq!(
            std::fs::read_to_string(git.root.join("local.md")).unwrap(),
            "# Different\n"
        );

        // With --force it overwrites.
        let outcome =
            import_phantom_file(&git, &mut config, &entry, &contents, true, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Imported);
        assert_eq!(
            std::fs::read_to_string(git.root.join("local.md")).unwrap(),
            "# Local\n"
        );
    }

    #[test]
    fn test_import_phantom_dir_writes_members() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        let mut contents = BTreeMap::new();
        contents.insert(
            archive::phantom_dir_member_key(".claude/a.json"),
            b"{}".to_vec(),
        );
        contents.insert(
            archive::phantom_dir_member_key(".claude/n/b.bin"),
            vec![0x00, 0x01],
        );
        let entry = ManifestEntry {
            path: ".claude".to_string(),
            entry_type: ManifestType::PhantomDir,
            exclude_mode: ExcludeMode::GitInfoExclude,
            baseline_commit: None,
            dir_members: Some(vec![
                ".claude/a.json".to_string(),
                ".claude/n/b.bin".to_string(),
            ]),
        };

        let outcome =
            import_phantom_dir(&git, &mut config, &entry, &contents, false, UiLocale::En).unwrap();
        assert_eq!(outcome, Outcome::Imported);
        assert_eq!(
            std::fs::read(git.root.join(".claude/n/b.bin")).unwrap(),
            vec![0x00, 0x01]
        );
        let e = config.get(".claude").unwrap();
        assert!(e.is_directory);
    }

    #[test]
    fn test_guard_rejects_when_suspended() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        config.suspended = true;
        let err = guard_importable(&git, &config).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ShadowError>(),
            Some(ShadowError::Suspended)
        ));
    }
}
