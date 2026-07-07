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
    /// Lockfile exists but its contents could not be parsed.
    Corrupt,
}

/// Check current lock status
pub fn check_lock(shadow_dir: &Path) -> anyhow::Result<LockStatus> {
    let lock_path = shadow_dir.join("lock");
    if !lock_path.exists() {
        return Ok(LockStatus::Free);
    }

    let content = std::fs::read_to_string(&lock_path).context("failed to read lockfile")?;
    // A lockfile we cannot parse cannot be attributed to any live process, so
    // report it as Corrupt rather than propagating an error. Callers can then
    // decide to reclaim it (e.g. `restore`).
    let Ok(info) = parse_lock(&content) else {
        return Ok(LockStatus::Corrupt);
    };

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
///
/// Acquisition is atomic: the lockfile is created with `O_CREAT | O_EXCL`
/// (`File::create_new`) so two concurrent processes can never both succeed.
/// If the file already exists we inspect it to classify the holder:
/// - held by us => `Ok(())` (idempotent)
/// - held by a live process => `Err(LockHeld)`
/// - held by a dead process => `Err(StaleLock)`
/// - unparseable/corrupt => `Err(CorruptLock)`, treated like a stale lock: we do
///   NOT silently clobber it (that was the previous bug); the user reclaims it via
///   `git-shadow restore`.
pub fn acquire_lock(shadow_dir: &Path) -> Result<(), ShadowError> {
    let lock_path = shadow_dir.join("lock");

    match std::fs::File::create_new(&lock_path) {
        Ok(mut file) => {
            use std::io::Write;
            let content = format!(
                "pid={}\ntimestamp={}",
                std::process::id(),
                Utc::now().to_rfc3339()
            );
            file.write_all(content.as_bytes())?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let content = std::fs::read_to_string(&lock_path)?;
            match parse_lock(&content) {
                Ok(info) => {
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
                    // Stale lock: held by a process that no longer exists.
                    Err(ShadowError::StaleLock(info.pid))
                }
                // Corrupted lock: cannot be attributed to any process. Treat it as
                // stale but surface a clear, dedicated error instead of overwriting.
                Err(_) => Err(ShadowError::CorruptLock),
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// Release lock (remove file)
pub fn release_lock(shadow_dir: &Path) -> anyhow::Result<()> {
    let lock_path = shadow_dir.join("lock");
    if lock_path.exists() {
        std::fs::remove_file(&lock_path).context("failed to remove lockfile")?;
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
            pid = Some(val.parse().context("failed to parse PID")?);
        } else if let Some(val) = line.strip_prefix("timestamp=") {
            timestamp = Some(
                DateTime::parse_from_rfc3339(val)
                    .context("failed to parse timestamp")?
                    .with_timezone(&Utc),
            );
        }
    }

    Ok(LockInfo {
        pid: pid.context("lockfile missing pid field")?,
        timestamp: timestamp.context("lockfile missing timestamp field")?,
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

    /// Spawn a real, signalable child process so that `is_process_alive` reports
    /// it as live. (Using PID 1 is unreliable: on macOS `kill(1, 0)` returns
    /// EPERM, which our existence check treats as "not alive".) Caller must kill
    /// the returned child.
    fn spawn_live_process() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn helper process")
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
        let mut child = spawn_live_process();
        let lock_path = shadow_dir.join("lock");
        let content = format!("pid={}\ntimestamp={}", child.id(), Utc::now().to_rfc3339());
        std::fs::write(&lock_path, content).unwrap();

        let result = acquire_lock(&shadow_dir);
        let is_held = matches!(result, Err(ShadowError::LockHeld { .. }));
        let _ = child.kill();
        let _ = child.wait();
        assert!(is_held, "expected LockHeld for a live process");
    }

    #[test]
    fn test_acquire_lock_reports_stale_on_dead_process() {
        let (_dir, shadow_dir) = make_shadow_dir();
        let lock_path = shadow_dir.join("lock");
        let content = format!("pid=999999\ntimestamp={}", Utc::now().to_rfc3339());
        std::fs::write(&lock_path, content).unwrap();

        let result = acquire_lock(&shadow_dir);
        assert!(matches!(result, Err(ShadowError::StaleLock(999999))));
    }

    #[test]
    fn test_acquire_lock_is_atomic_second_acquire_by_other_fails() {
        // Simulate a lock already held by another live process, then verify a
        // fresh acquisition does not overwrite it (atomic create_new).
        let (_dir, shadow_dir) = make_shadow_dir();
        let mut child = spawn_live_process();
        let lock_path = shadow_dir.join("lock");
        let existing = format!("pid={}\ntimestamp={}", child.id(), Utc::now().to_rfc3339());
        std::fs::write(&lock_path, &existing).unwrap();

        let result = acquire_lock(&shadow_dir);
        // The original lock content must be untouched.
        let after = std::fs::read_to_string(&lock_path).unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.is_err());
        assert_eq!(after, existing);
    }

    #[test]
    fn test_corrupt_lock_reported_by_check_and_acquire() {
        let (_dir, shadow_dir) = make_shadow_dir();
        let lock_path = shadow_dir.join("lock");
        // Garbage that parse_lock cannot interpret (no pid field).
        std::fs::write(&lock_path, "this is not a valid lockfile").unwrap();

        assert!(matches!(
            check_lock(&shadow_dir).unwrap(),
            LockStatus::Corrupt
        ));
        assert!(matches!(
            acquire_lock(&shadow_dir),
            Err(ShadowError::CorruptLock)
        ));
        // Corrupt lock is not silently clobbered.
        assert!(lock_path.exists());
    }
}
