use std::io::Write;
use std::path::Path;

use crate::error::ShadowError;

pub const SIZE_LIMIT: u64 = 1_048_576; // 1 MB
const BINARY_CHECK_BYTES: usize = 8192;

/// Check if file appears to be binary (contains null bytes in first 8KB)
pub fn is_binary(path: &Path) -> anyhow::Result<bool> {
    todo!()
}

/// Check if file exceeds size limit. Returns error if over limit and force is false.
pub fn check_size(path: &Path, force: bool) -> Result<(), ShadowError> {
    todo!()
}

/// Atomic write: write to temp file in same directory, then rename
pub fn atomic_write(target: &Path, content: &[u8]) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("対象パスに親ディレクトリがありません"))?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;
    tmp.persist(target)?;
    Ok(())
}
