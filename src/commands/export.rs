use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::archive::{self, Manifest, ManifestEntry, ManifestType};
use crate::config::{FileType, ShadowConfig};
use crate::error::ShadowError;
use crate::git::GitRepo;
use crate::lock::{self, LockStatus};
use crate::path;
use crate::ui;

const DEFAULT_OUTPUT: &str = "git-shadow-export.tar.gz";

pub fn run(output: Option<String>, force: bool) -> Result<()> {
    let locale = ui::detect_locale();
    let cwd = std::env::current_dir()?;
    let git = GitRepo::discover(&cwd)?;
    git.ensure_initialized()?;
    let config = ShadowConfig::load(&git.shadow_dir)?;

    guard_exportable(&git, &config)?;

    let output_path = resolve_output(output, &cwd);
    if output_path.exists() && !force {
        return Err(ShadowError::ExportFileExists(output_path.display().to_string()).into());
    }

    let (manifest, contents) = collect_export(&git, &config)?;
    archive::write_archive(&output_path, &manifest, &contents)?;

    println!(
        "{}",
        ui::export_success(
            locale,
            manifest.entries.len(),
            &output_path.display().to_string()
        )
        .green()
    );
    Ok(())
}

fn resolve_output(output: Option<String>, cwd: &Path) -> PathBuf {
    match output {
        Some(o) => {
            let p = PathBuf::from(&o);
            if p.is_absolute() {
                p
            } else {
                cwd.join(p)
            }
        }
        None => cwd.join(DEFAULT_OUTPUT),
    }
}

/// Refuse to export in states where the exported content would be wrong or where a
/// commit cycle is mid-flight. Mirrors the guard patterns used by uninstall/suspend.
fn guard_exportable(git: &GitRepo, config: &ShadowConfig) -> Result<()> {
    if config.files.is_empty() {
        return Err(ShadowError::NothingToExport.into());
    }

    // Suspended: worktree files hold baseline content, so "shadow content" would be
    // empty/wrong. Tell the user to resume first.
    if config.suspended {
        return Err(ShadowError::Suspended.into());
    }

    // Leftover stash means a commit cycle was interrupted.
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

    // A live lock means another git-shadow process is committing right now.
    if let Ok(LockStatus::HeldByOther(info)) = lock::check_lock(&git.shadow_dir) {
        return Err(ShadowError::LockHeld {
            pid: info.pid,
            timestamp: info.timestamp.to_rfc3339(),
        }
        .into());
    }

    Ok(())
}

/// Gather the manifest and content files for every managed entry.
fn collect_export(
    git: &GitRepo,
    config: &ShadowConfig,
) -> Result<(Manifest, BTreeMap<String, Vec<u8>>)> {
    let mut entries = Vec::new();
    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for (file_path, entry) in &config.files {
        match entry.file_type {
            FileType::Overlay => {
                let worktree_path = git.root.join(file_path);
                let shadow = std::fs::read(&worktree_path).with_context(|| {
                    format!(
                        "failed to read working-tree content for overlay {}",
                        file_path
                    )
                })?;

                let encoded = path::encode_path(file_path);
                let baseline_path = git.shadow_dir.join("baselines").join(&encoded);
                if !baseline_path.exists() {
                    return Err(ShadowError::BaselineMissing(file_path.clone()).into());
                }
                let baseline = std::fs::read(&baseline_path)
                    .with_context(|| format!("failed to read baseline for {}", file_path))?;

                contents.insert(archive::overlay_shadow_key(file_path), shadow);
                contents.insert(archive::overlay_baseline_key(file_path), baseline);

                entries.push(ManifestEntry {
                    path: file_path.clone(),
                    entry_type: ManifestType::Overlay,
                    exclude_mode: entry.exclude_mode.clone(),
                    baseline_commit: entry.baseline_commit.clone(),
                    dir_members: None,
                });
            }
            FileType::Phantom if !entry.is_directory => {
                let worktree_path = git.root.join(file_path);
                let content = std::fs::read(&worktree_path).with_context(|| {
                    format!("failed to read phantom file {} for export", file_path)
                })?;
                contents.insert(archive::phantom_key(file_path), content);

                entries.push(ManifestEntry {
                    path: file_path.clone(),
                    entry_type: ManifestType::Phantom,
                    exclude_mode: entry.exclude_mode.clone(),
                    baseline_commit: None,
                    dir_members: None,
                });
            }
            FileType::Phantom => {
                // Phantom directory: archive every file under it recursively.
                let dir_path = git.root.join(file_path);
                if !dir_path.is_dir() {
                    anyhow::bail!(
                        "phantom directory {} does not exist in the working tree",
                        file_path
                    );
                }
                let mut members = Vec::new();
                collect_dir_files(&git.root, &dir_path, &mut members)?;
                members.sort();

                for member in &members {
                    let data = std::fs::read(git.root.join(member))
                        .with_context(|| format!("failed to read phantom dir member {}", member))?;
                    contents.insert(archive::phantom_dir_member_key(member), data);
                }

                entries.push(ManifestEntry {
                    path: file_path.clone(),
                    entry_type: ManifestType::PhantomDir,
                    exclude_mode: entry.exclude_mode.clone(),
                    baseline_commit: None,
                    dir_members: Some(members),
                });
            }
        }
    }

    let manifest = Manifest {
        format_version: archive::FORMAT_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        entries,
    };

    Ok((manifest, contents))
}

/// Recursively collect repo-relative paths of every regular file under `dir`.
fn collect_dir_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_dir_files(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("path {} is outside repo root", path.display()))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExcludeMode;

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

    fn overlay_config(git: &GitRepo) -> ShadowConfig {
        let mut config = ShadowConfig::new();
        let commit = git.head_commit().unwrap();
        let baseline = git.show_file("HEAD", "CLAUDE.md").unwrap();
        let encoded = path::encode_path("CLAUDE.md");
        crate::fs_util::atomic_write(&git.shadow_dir.join("baselines").join(&encoded), &baseline)
            .unwrap();
        config.add_overlay("CLAUDE.md".to_string(), commit).unwrap();
        // Local shadow edit.
        std::fs::write(git.root.join("CLAUDE.md"), "# Team\n# shadow\n").unwrap();
        config
    }

    #[test]
    fn test_guard_rejects_empty_config() {
        let (_dir, git) = make_test_repo();
        let config = ShadowConfig::new();
        let err = guard_exportable(&git, &config).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ShadowError>(),
            Some(ShadowError::NothingToExport)
        ));
    }

    #[test]
    fn test_guard_rejects_when_suspended() {
        let (_dir, git) = make_test_repo();
        let mut config = overlay_config(&git);
        config.suspended = true;
        let err = guard_exportable(&git, &config).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ShadowError>(),
            Some(ShadowError::Suspended)
        ));
    }

    #[test]
    fn test_guard_rejects_on_stash_remnant() {
        let (_dir, git) = make_test_repo();
        let config = overlay_config(&git);
        std::fs::write(git.shadow_dir.join("stash").join("old.md"), "x").unwrap();
        let err = guard_exportable(&git, &config).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ShadowError>(),
            Some(ShadowError::StashRemaining)
        ));
    }

    #[test]
    fn test_collect_export_overlay_includes_shadow_and_baseline() {
        let (_dir, git) = make_test_repo();
        let config = overlay_config(&git);

        let (manifest, contents) = collect_export(&git, &config).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].entry_type, ManifestType::Overlay);
        assert!(manifest.entries[0].baseline_commit.is_some());
        assert_eq!(
            contents
                .get(&archive::overlay_shadow_key("CLAUDE.md"))
                .unwrap(),
            b"# Team\n# shadow\n"
        );
        assert_eq!(
            contents
                .get(&archive::overlay_baseline_key("CLAUDE.md"))
                .unwrap(),
            b"# Team\n"
        );
    }

    #[test]
    fn test_collect_export_phantom_dir_recurses() {
        let (_dir, git) = make_test_repo();
        let mut config = ShadowConfig::new();
        std::fs::create_dir_all(git.root.join(".claude/nested")).unwrap();
        std::fs::write(git.root.join(".claude/a.json"), "{}").unwrap();
        std::fs::write(git.root.join(".claude/nested/b.txt"), b"\x00\x01bin").unwrap();
        config
            .add_phantom(".claude".to_string(), ExcludeMode::GitInfoExclude, true)
            .unwrap();

        let (manifest, contents) = collect_export(&git, &config).unwrap();
        assert_eq!(manifest.entries[0].entry_type, ManifestType::PhantomDir);
        let members = manifest.entries[0].dir_members.clone().unwrap();
        assert!(members.contains(&".claude/a.json".to_string()));
        assert!(members.contains(&".claude/nested/b.txt".to_string()));
        assert_eq!(
            contents
                .get(&archive::phantom_dir_member_key(".claude/nested/b.txt"))
                .unwrap(),
            b"\x00\x01bin"
        );
    }

    #[test]
    fn test_resolve_output_defaults_to_cwd() {
        let cwd = Path::new("/tmp/repo");
        assert_eq!(
            resolve_output(None, cwd),
            cwd.join("git-shadow-export.tar.gz")
        );
        assert_eq!(
            resolve_output(Some("out.tar.gz".to_string()), cwd),
            cwd.join("out.tar.gz")
        );
        assert_eq!(
            resolve_output(Some("/abs/out.tar.gz".to_string()), cwd),
            PathBuf::from("/abs/out.tar.gz")
        );
    }
}
