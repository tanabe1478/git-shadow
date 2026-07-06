use std::path::Path;

use anyhow::{Context, Result};

use crate::error::ShadowError;

/// Result of a 3-way merge
#[derive(Debug)]
pub struct MergeResult {
    /// The merged content
    pub content: String,
    /// Whether there were conflicts
    pub has_conflicts: bool,
}

/// Perform a 3-way merge using `git merge-file`
///
/// - base: the common ancestor (old baseline)
/// - ours: the version with our changes (current working tree content)
/// - theirs: the version from the other side (new HEAD content = new baseline)
///
/// Returns merged content with conflict markers if applicable
pub fn three_way_merge(
    base: &str,
    ours: &str,
    theirs: &str,
    work_dir: &Path,
) -> Result<MergeResult> {
    let base_file = tempfile::Builder::new()
        .prefix("shadow-base-")
        .tempfile_in(work_dir)
        .context("failed to create temp file")?;
    let ours_file = tempfile::Builder::new()
        .prefix("shadow-ours-")
        .tempfile_in(work_dir)
        .context("failed to create temp file")?;
    let theirs_file = tempfile::Builder::new()
        .prefix("shadow-theirs-")
        .tempfile_in(work_dir)
        .context("failed to create temp file")?;

    std::fs::write(base_file.path(), base)?;
    std::fs::write(ours_file.path(), ours)?;
    std::fs::write(theirs_file.path(), theirs)?;

    run_merge_file(ours_file.path(), base_file.path(), theirs_file.path())
}

/// Run `git merge-file -p --diff3 <ours> <base> <theirs>` and interpret the result.
///
/// `git merge-file` exit codes:
/// - `0` => clean merge (no conflicts)
/// - `1..=127` => number of conflicts (capped at 127); merge succeeded with markers
/// - otherwise => the merge itself failed (git returns a negative value on error, which
///   surfaces as an exit code in the 128..=255 range), or the process was killed by a
///   signal (`code()` is `None`). In these cases stdout is unusable (typically empty), so
///   writing it to the working tree would truncate the user's file. Report an error instead.
fn run_merge_file(ours: &Path, base: &Path, theirs: &Path) -> Result<MergeResult> {
    let output = std::process::Command::new("git")
        .args([
            "merge-file",
            "-p",      // print to stdout instead of modifying file
            "--diff3", // show base content in conflict markers
        ])
        .arg(ours)
        .arg(base)
        .arg(theirs)
        .output()
        .context("failed to run git merge-file")?;

    match output.status.code() {
        Some(0) => Ok(MergeResult {
            content: String::from_utf8_lossy(&output.stdout).to_string(),
            has_conflicts: false,
        }),
        Some(n) if (1..=127).contains(&n) => Ok(MergeResult {
            content: String::from_utf8_lossy(&output.stdout).to_string(),
            has_conflicts: true,
        }),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(ShadowError::MergeFailed(stderr).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_merge() {
        let dir = tempfile::tempdir().unwrap();
        let base = "line1\nline2\nline3\n";
        let ours = "line1\nline2 modified\nline3\n";
        let theirs = "line1\nline2\nline3\nline4\n";

        let result = three_way_merge(base, ours, theirs, dir.path()).unwrap();
        assert!(!result.has_conflicts);
        assert!(result.content.contains("line2 modified"));
        assert!(result.content.contains("line4"));
    }

    #[test]
    fn test_conflict_merge() {
        let dir = tempfile::tempdir().unwrap();
        let base = "line1\n";
        let ours = "ours change\n";
        let theirs = "theirs change\n";

        let result = three_way_merge(base, ours, theirs, dir.path()).unwrap();
        assert!(result.has_conflicts);
        assert!(result.content.contains("<<<<<<<"));
        assert!(result.content.contains(">>>>>>>"));
    }

    #[test]
    fn test_no_changes() {
        let dir = tempfile::tempdir().unwrap();
        let content = "unchanged\n";

        let result = three_way_merge(content, content, content, dir.path()).unwrap();
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "unchanged\n");
    }

    #[test]
    fn test_only_ours_changed() {
        let dir = tempfile::tempdir().unwrap();
        let base = "original\n";
        let ours = "original\nour addition\n";
        let theirs = "original\n";

        let result = three_way_merge(base, ours, theirs, dir.path()).unwrap();
        assert!(!result.has_conflicts);
        assert!(result.content.contains("our addition"));
    }

    #[test]
    fn test_only_theirs_changed() {
        let dir = tempfile::tempdir().unwrap();
        let base = "original\n";
        let ours = "original\n";
        let theirs = "original\ntheir addition\n";

        let result = three_way_merge(base, ours, theirs, dir.path()).unwrap();
        assert!(!result.has_conflicts);
        assert!(result.content.contains("their addition"));
    }

    #[test]
    fn test_merge_error_is_reported_not_treated_as_conflict() {
        // Force a real `git merge-file` error by passing a directory as the
        // `ours` path. On error, git returns a negative value (surfacing as a
        // high exit code) with empty stdout — this must be an Err, never a
        // MergeResult with empty content that would truncate the file.
        let dir = tempfile::tempdir().unwrap();
        let sub_dir = dir.path().join("is-a-directory");
        std::fs::create_dir(&sub_dir).unwrap();

        let base = dir.path().join("base");
        let theirs = dir.path().join("theirs");
        std::fs::write(&base, "a\n").unwrap();
        std::fs::write(&theirs, "a\n").unwrap();

        let result = run_merge_file(&sub_dir, &base, &theirs);
        assert!(result.is_err(), "merge error must surface as Err");
        let err = result.unwrap_err();
        assert!(
            err.downcast_ref::<ShadowError>()
                .map(|e| matches!(e, ShadowError::MergeFailed(_)))
                .unwrap_or(false),
            "error should be ShadowError::MergeFailed, got: {err}"
        );
    }

    #[test]
    fn test_run_merge_file_clean_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        // Clean merge
        let base = dir.path().join("base");
        let ours = dir.path().join("ours");
        let theirs = dir.path().join("theirs");
        std::fs::write(&base, "line1\nline2\n").unwrap();
        std::fs::write(&ours, "line1\nline2\nline3\n").unwrap();
        std::fs::write(&theirs, "line1\nline2\n").unwrap();
        let clean = run_merge_file(&ours, &base, &theirs).unwrap();
        assert!(!clean.has_conflicts);
        assert!(clean.content.contains("line3"));

        // Conflicting merge
        std::fs::write(&base, "line1\n").unwrap();
        std::fs::write(&ours, "ours\n").unwrap();
        std::fs::write(&theirs, "theirs\n").unwrap();
        let conflict = run_merge_file(&ours, &base, &theirs).unwrap();
        assert!(conflict.has_conflicts);
        assert!(conflict.content.contains("<<<<<<<"));
    }
}
