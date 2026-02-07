use std::path::Path;

use anyhow::{Context, Result};

/// Result of a 3-way merge
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

    // git merge-file modifies ours_file in place and returns:
    // 0: clean merge
    // >0: number of conflicts
    // <0: error
    let output = std::process::Command::new("git")
        .args([
            "merge-file",
            "-p",      // print to stdout instead of modifying file
            "--diff3", // show base content in conflict markers
        ])
        .arg(ours_file.path())
        .arg(base_file.path())
        .arg(theirs_file.path())
        .output()
        .context("failed to run git merge-file")?;

    let content = String::from_utf8_lossy(&output.stdout).to_string();
    let has_conflicts = output.status.code().unwrap_or(-1) > 0;

    Ok(MergeResult {
        content,
        has_conflicts,
    })
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
}
