use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Normalize a user-provided path to a repository-relative path:
/// - Resolve relative inputs against the caller's current working directory
/// - Reject paths that resolve outside the repository root
/// - Return a forward-slash relative path without "." or ".." segments
pub fn normalize_path(input: &str, cwd: &Path, repo_root: &Path) -> Result<String> {
    let input = input.replace('\\', "/");
    if input.trim().is_empty() {
        bail!("path cannot be empty");
    }

    let candidate = if Path::new(&input).is_absolute() {
        PathBuf::from(&input)
    } else {
        cwd.join(&input)
    };
    let normalized = normalize_lexical(&candidate);

    if !normalized.starts_with(repo_root) {
        bail!(
            "path '{}' resolves outside repository '{}'",
            input,
            repo_root.display()
        );
    }

    let relative = normalized
        .strip_prefix(repo_root)
        .with_context(|| format!("failed to strip repository prefix from '{}'", input))?;

    if relative.as_os_str().is_empty() {
        bail!("path '{}' resolves to the repository root", input);
    }

    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// URL-encode a normalized path for use as filename in baselines/ and stash/:
/// 1. % -> %25 (escape the escape char first)
/// 2. / -> %2F
pub fn encode_path(normalized: &str) -> String {
    normalized.replace('%', "%25").replace('/', "%2F")
}

/// Decode a URL-encoded filename back to a normalized path:
/// 1. %2F -> /
/// 2. %25 -> %
pub fn decode_path(encoded: &str) -> String {
    encoded.replace("%2F", "/").replace("%25", "%")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- encode_path tests ---

    #[test]
    fn test_encode_simple_filename() {
        assert_eq!(encode_path("CLAUDE.md"), "CLAUDE.md");
    }

    #[test]
    fn test_encode_path_with_slashes() {
        assert_eq!(
            encode_path("src/components/CLAUDE.md"),
            "src%2Fcomponents%2FCLAUDE.md"
        );
    }

    #[test]
    fn test_encode_path_with_percent() {
        assert_eq!(encode_path("docs/100%done.md"), "docs%2F100%25done.md");
    }

    #[test]
    fn test_encode_path_with_percent_and_slash() {
        assert_eq!(encode_path("a%b/c"), "a%25b%2Fc");
    }

    // --- decode_path tests ---

    #[test]
    fn test_decode_simple_filename() {
        assert_eq!(decode_path("CLAUDE.md"), "CLAUDE.md");
    }

    #[test]
    fn test_decode_path_with_slashes() {
        assert_eq!(
            decode_path("src%2Fcomponents%2FCLAUDE.md"),
            "src/components/CLAUDE.md"
        );
    }

    #[test]
    fn test_decode_path_with_percent() {
        assert_eq!(decode_path("docs%2F100%25done.md"), "docs/100%done.md");
    }

    // --- roundtrip tests ---

    #[test]
    fn test_roundtrip_simple() {
        let path = "CLAUDE.md";
        assert_eq!(decode_path(&encode_path(path)), path);
    }

    #[test]
    fn test_roundtrip_nested() {
        let path = "src/components/CLAUDE.md";
        assert_eq!(decode_path(&encode_path(path)), path);
    }

    #[test]
    fn test_roundtrip_with_percent() {
        let path = "docs/100%done.md";
        assert_eq!(decode_path(&encode_path(path)), path);
    }

    #[test]
    fn test_roundtrip_complex() {
        let path = "a%b/c%d/e";
        assert_eq!(decode_path(&encode_path(path)), path);
    }

    #[test]
    fn test_roundtrip_double_percent() {
        let path = "%%/%%";
        assert_eq!(decode_path(&encode_path(path)), path);
    }

    // --- normalize_path tests ---

    #[test]
    fn test_normalize_strips_leading_dot_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("./CLAUDE.md", &repo, &repo).unwrap(),
            "CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_already_relative() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("CLAUDE.md", &repo, &repo).unwrap(),
            "CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_nested_path() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("src/components/CLAUDE.md", &repo, &repo).unwrap(),
            "src/components/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_backslash_to_forward_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("src\\components\\CLAUDE.md", &repo, &repo).unwrap(),
            "src/components/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_absolute_path_within_repo() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("/repo/src/CLAUDE.md", &repo, &repo).unwrap(),
            "src/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_strips_trailing_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(normalize_path(".claude/", &repo, &repo).unwrap(), ".claude");
    }

    #[test]
    fn test_normalize_strips_trailing_slash_nested() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("src/components/", &repo, &repo).unwrap(),
            "src/components"
        );
    }

    #[test]
    fn test_normalize_dir_with_leading_dot_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("./.claude/", &repo, &repo).unwrap(),
            ".claude"
        );
    }

    #[test]
    fn test_normalize_strips_multiple_leading_dot_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("././CLAUDE.md", &repo, &repo).unwrap(),
            "CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_resolves_parent_dir_inside_repo() {
        let repo = PathBuf::from("/repo");
        let cwd = repo.join("src");
        assert_eq!(
            normalize_path("../CLAUDE.md", &cwd, &repo).unwrap(),
            "CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_rejects_path_outside_repo() {
        let repo = PathBuf::from("/repo");
        let cwd = repo.join("src");
        let err = normalize_path("../../outside.txt", &cwd, &repo).unwrap_err();
        assert!(err.to_string().contains("resolves outside repository"));
    }

    #[test]
    fn test_normalize_rejects_repo_root() {
        let repo = PathBuf::from("/repo");
        let err = normalize_path(".", &repo, &repo).unwrap_err();
        assert!(err.to_string().contains("repository root"));
    }
}
