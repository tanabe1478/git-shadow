use std::path::Path;

use anyhow::{bail, Result};

/// Normalize a user-provided path to repository-relative format:
/// - Convert to repo-relative path (using / separator)
/// - Strip leading ./
pub fn normalize_path(input: &str, repo_root: &Path) -> Result<String> {
    // Convert backslashes to forward slashes
    let input = input.replace('\\', "/");

    // If absolute, try to strip repo_root prefix
    let relative = if input.starts_with('/') {
        let root_str = repo_root.to_string_lossy().replace('\\', "/");
        let root_str = root_str.trim_end_matches('/');
        if let Some(stripped) = input.strip_prefix(root_str) {
            stripped.trim_start_matches('/').to_string()
        } else {
            bail!(
                "path '{}' is not inside repository '{}'",
                input,
                repo_root.display()
            );
        }
    } else {
        input.to_string()
    };

    // Strip leading ./ (possibly repeated)
    let mut result = relative.as_str();
    while let Some(stripped) = result.strip_prefix("./") {
        result = stripped;
    }

    Ok(result.to_string())
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
        assert_eq!(normalize_path("./CLAUDE.md", &repo).unwrap(), "CLAUDE.md");
    }

    #[test]
    fn test_normalize_already_relative() {
        let repo = PathBuf::from("/repo");
        assert_eq!(normalize_path("CLAUDE.md", &repo).unwrap(), "CLAUDE.md");
    }

    #[test]
    fn test_normalize_nested_path() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("src/components/CLAUDE.md", &repo).unwrap(),
            "src/components/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_backslash_to_forward_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("src\\components\\CLAUDE.md", &repo).unwrap(),
            "src/components/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_absolute_path_within_repo() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            normalize_path("/repo/src/CLAUDE.md", &repo).unwrap(),
            "src/CLAUDE.md"
        );
    }

    #[test]
    fn test_normalize_strips_multiple_leading_dot_slash() {
        let repo = PathBuf::from("/repo");
        assert_eq!(normalize_path("././CLAUDE.md", &repo).unwrap(), "CLAUDE.md");
    }
}
