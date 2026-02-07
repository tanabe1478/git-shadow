use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::error::ShadowError;

#[derive(Debug)]
pub struct LockInfo {
    pub pid: u32,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub enum LockStatus {
    Free,
    HeldByUs,
    HeldByOther(LockInfo),
    Stale(LockInfo),
}

/// Check current lock status
pub fn check_lock(shadow_dir: &Path) -> anyhow::Result<LockStatus> {
    let lock_path = shadow_dir.join("lock");
    if !lock_path.exists() {
        return Ok(LockStatus::Free);
    }

    let content = std::fs::read_to_string(&lock_path).context("lockfile の読み込みに失敗")?;
    let info = parse_lock(&content)?;

    let my_pid = std::process::id();
    if info.pid == my_pid {
        return Ok(LockStatus::HeldByUs);
    }

    if is_process_alive(info.pid) {
        Ok(LockStatus::HeldByOther(info))
    } else {
        Ok(LockStatus::Stale(info))
    }
}

/// Acquire lock (write PID + timestamp). Fails if locked by another live process.
pub fn acquire_lock(shadow_dir: &Path) -> Result<(), ShadowError> {
    let lock_path = shadow_dir.join("lock");

    if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)?;
        if let Ok(info) = parse_lock(&content) {
            let my_pid = std::process::id();
            if info.pid == my_pid {
                return Ok(()); // Already held by us
            }
            if is_process_alive(info.pid) {
                return Err(ShadowError::LockHeld {
                    pid: info.pid,
                    timestamp: info.timestamp.to_rfc3339(),
                });
            }
            // Stale lock
            return Err(ShadowError::StaleLock(info.pid));
        }
    }

    let content = format!(
        "pid={}\ntimestamp={}",
        std::process::id(),
        Utc::now().to_rfc3339()
    );
    std::fs::write(&lock_path, content)?;
    Ok(())
}

/// Release lock (remove file)
pub fn release_lock(shadow_dir: &Path) -> anyhow::Result<()> {
    let lock_path = shadow_dir.join("lock");
    if lock_path.exists() {
        std::fs::remove_file(&lock_path).context("lockfile の削除に失敗")?;
    }
    Ok(())
}

/// Check if a process with the given PID is alive
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Parse lock file content
fn parse_lock(content: &str) -> anyhow::Result<LockInfo> {
    let mut pid: Option<u32> = None;
    let mut timestamp: Option<DateTime<Utc>> = None;

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("pid=") {
            pid = Some(val.parse().context("PID のパースに失敗")?);
        } else if let Some(val) = line.strip_prefix("timestamp=") {
            timestamp = Some(
                DateTime::parse_from_rfc3339(val)
                    .context("タイムスタンプのパースに失敗")?
                    .with_timezone(&Utc),
            );
        }
    }

    Ok(LockInfo {
        pid: pid.context("lockfile に pid がありません")?,
        timestamp: timestamp.context("lockfile に timestamp がありません")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shadow_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let shadow_dir = dir.path().join("shadow");
        std::fs::create_dir_all(&shadow_dir).unwrap();
        (dir, shadow_dir)
    }

    #[test]
    fn test_check_lock_free() {
        let (_dir, shadow_dir) = make_shadow_dir();
        let status = check_lock(&shadow_dir).unwrap();
        assert!(matches!(status, LockStatus::Free));
    }

    #[test]
    fn test_acquire_and_check_held_by_us() {
        let (_dir, shadow_dir) = make_shadow_dir();
        acquire_lock(&shadow_dir).unwrap();
        let status = check_lock(&shadow_dir).unwrap();
        assert!(matches!(status, LockStatus::HeldByUs));
    }

    #[test]
    fn test_release_lock() {
        let (_dir, shadow_dir) = make_shadow_dir();
        acquire_lock(&shadow_dir).unwrap();
        release_lock(&shadow_dir).unwrap();
        let status = check_lock(&shadow_dir).unwrap();
        assert!(matches!(status, LockStatus::Free));
    }

    #[test]
    fn test_stale_lock_detection() {
        let (_dir, shadow_dir) = make_shadow_dir();
        let lock_path = shadow_dir.join("lock");
        // Write a lock with a PID that definitely doesn't exist
        let content = format!("pid=999999\ntimestamp={}", Utc::now().to_rfc3339());
        std::fs::write(&lock_path, content).unwrap();

        let status = check_lock(&shadow_dir).unwrap();
        assert!(matches!(status, LockStatus::Stale(_)));
    }

    #[test]
    fn test_lock_file_format() {
        let (_dir, shadow_dir) = make_shadow_dir();
        acquire_lock(&shadow_dir).unwrap();

        let lock_path = shadow_dir.join("lock");
        let content = std::fs::read_to_string(&lock_path).unwrap();
        assert!(content.contains("pid="));
        assert!(content.contains("timestamp="));
    }

    #[test]
    fn test_parse_lock_content() {
        let content = "pid=12345\ntimestamp=2026-02-07T12:00:00+00:00";
        let info = parse_lock(content).unwrap();
        assert_eq!(info.pid, 12345);
    }

    #[test]
    fn test_release_nonexistent_lock_is_ok() {
        let (_dir, shadow_dir) = make_shadow_dir();
        assert!(release_lock(&shadow_dir).is_ok());
    }

    #[test]
    fn test_acquire_lock_fails_on_live_other_process() {
        let (_dir, shadow_dir) = make_shadow_dir();
        // Write a lock with PID 1 (init/launchd - always alive)
        let lock_path = shadow_dir.join("lock");
        let content = format!("pid=1\ntimestamp={}", Utc::now().to_rfc3339());
        std::fs::write(&lock_path, content).unwrap();

        let result = acquire_lock(&shadow_dir);
        assert!(result.is_err());
    }
}
