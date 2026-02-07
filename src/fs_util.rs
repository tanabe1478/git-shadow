use std::io::{Read, Write};
use std::path::Path;

use crate::error::ShadowError;

pub const SIZE_LIMIT: u64 = 1_048_576; // 1 MB
const BINARY_CHECK_BYTES: usize = 8192;

/// Check if file appears to be binary (contains null bytes in first 8KB)
pub fn is_binary(path: &Path) -> anyhow::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; BINARY_CHECK_BYTES];
    let n = file.read(&mut buf)?;
    Ok(buf[..n].contains(&0))
}

/// Check if file exceeds size limit. Returns error if over limit and force is false.
pub fn check_size(path: &Path, force: bool) -> Result<(), ShadowError> {
    let metadata = std::fs::metadata(path)?;
    let size = metadata.len();
    if size > SIZE_LIMIT && !force {
        return Err(ShadowError::FileTooLarge(
            path.display().to_string(),
            size,
            SIZE_LIMIT,
        ));
    }
    Ok(())
}

/// Atomic write: write to temp file in same directory, then rename
pub fn atomic_write(target: &Path, content: &[u8]) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target path has no parent directory"))?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;
    tmp.persist(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_is_binary_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.txt");
        std::fs::write(&path, "Hello, world!\nLine 2\n").unwrap();
        assert!(!is_binary(&path).unwrap());
    }

    #[test]
    fn test_is_binary_with_null_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.bin");
        let mut content = vec![0x48, 0x65, 0x6c, 0x6c, 0x6f]; // "Hello"
        content.push(0x00); // null byte
        content.extend_from_slice(b"world");
        std::fs::write(&path, &content).unwrap();
        assert!(is_binary(&path).unwrap());
    }

    #[test]
    fn test_is_binary_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();
        assert!(!is_binary(&path).unwrap());
    }

    #[test]
    fn test_is_binary_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf8.txt");
        std::fs::write(&path, "UTF-8 test: café résumé 🚀").unwrap();
        assert!(!is_binary(&path).unwrap());
    }

    #[test]
    fn test_check_size_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.txt");
        std::fs::write(&path, "small content").unwrap();
        assert!(check_size(&path, false).is_ok());
    }

    #[test]
    fn test_check_size_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bin");
        let content = vec![0x41u8; (SIZE_LIMIT + 1) as usize];
        std::fs::write(&path, &content).unwrap();

        let result = check_size(&path, false);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ShadowError::FileTooLarge(_, _, _)
        ));
    }

    #[test]
    fn test_check_size_over_limit_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bin");
        let content = vec![0x41u8; (SIZE_LIMIT + 1) as usize];
        std::fs::write(&path, &content).unwrap();

        assert!(check_size(&path, true).is_ok());
    }

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.txt");
        atomic_write(&path, b"test content").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "test content");
    }

    #[test]
    fn test_atomic_write_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.txt");
        std::fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn test_atomic_write_no_partial_on_dir_missing() {
        let path = Path::new("/nonexistent/dir/file.txt");
        assert!(atomic_write(path, b"content").is_err());
        assert!(!path.exists());
    }
}
