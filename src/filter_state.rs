//! Persistent state for the experimental clean/smudge filter backend.
//!
//! This module is self-contained and does not interact with the hook backend
//! state (`shadow/`). It lives under `<git_dir>/shadow-filter/` so every
//! worktree gets an independent view:
//!
//! ```text
//! <git_dir>/shadow-filter/
//!   config.json               registry: { version, files: {path: entry} }
//!   baselines/<enc>           B: last repository content the filter saw
//!   local/<enc>               L: local content (baseline + overlay)
//!   pending_base/<enc>        R: repository blob that caused a smudge conflict
//!   pending_local/<enc>       marker-laden content emitted for that conflict
//! ```
//!
//! The pair (B, L) defines the overlay: `clean` rewrites the worktree content
//! W as `merge3(ours=W, base=L, theirs=B)` and `smudge` rewrites a repository
//! blob R as `merge3(ours=L, base=B, theirs=R)`. See
//! `docs/experiments/clean-smudge-backend.md` for the full design.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fs_util;
use crate::path::encode_path;

pub const FILTER_DIR_NAME: &str = "shadow-filter";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterFileEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_commit: Option<String>,
    /// A smudge-side merge conflict is pending resolution. While set, `clean`
    /// refuses to convert the file (fail closed) until `filter resolve` runs.
    #[serde(default)]
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub conflicted: bool,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterState {
    pub version: u32,
    pub files: BTreeMap<String, FilterFileEntry>,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            version: 1,
            files: BTreeMap::new(),
        }
    }
}

impl FilterState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// File-backed store rooted at `<git_dir>/shadow-filter`.
pub struct FilterStore {
    pub dir: PathBuf,
}

impl FilterStore {
    pub fn new(git_dir: &Path) -> Self {
        Self {
            dir: git_dir.join(FILTER_DIR_NAME),
        }
    }

    pub fn load(&self) -> anyhow::Result<FilterState> {
        let config_path = self.dir.join("config.json");
        if !config_path.exists() {
            return Ok(FilterState::new());
        }
        let content = std::fs::read_to_string(&config_path)
            .context("failed to read shadow-filter/config.json")?;
        serde_json::from_str(&content).context("failed to parse shadow-filter/config.json")
    }

    pub fn save(&self, state: &FilterState) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let content = serde_json::to_string_pretty(state)
            .context("failed to serialize shadow-filter config")?;
        fs_util::atomic_write(&self.dir.join("config.json"), content.as_bytes())
    }

    pub fn exists(&self) -> bool {
        self.dir.is_dir()
    }

    pub fn baseline_path(&self, path: &str) -> PathBuf {
        self.dir.join("baselines").join(encode_path(path))
    }

    pub fn local_path(&self, path: &str) -> PathBuf {
        self.dir.join("local").join(encode_path(path))
    }

    pub fn pending_base_path(&self, path: &str) -> PathBuf {
        self.dir.join("pending_base").join(encode_path(path))
    }

    pub fn pending_local_path(&self, path: &str) -> PathBuf {
        self.dir.join("pending_local").join(encode_path(path))
    }

    /// Write content to a named blob slot, creating parent dirs.
    pub fn write_slot(&self, slot: &Path, content: &[u8]) -> anyhow::Result<()> {
        if let Some(parent) = slot.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fs_util::atomic_write(slot, content)
    }

    pub fn read_slot(&self, slot: &Path) -> anyhow::Result<Vec<u8>> {
        std::fs::read(slot).with_context(|| format!("failed to read {}", slot.display()))
    }

    /// Drop all state files for `path` (keep the registry entry decision to caller).
    pub fn remove_file_state(&self, path: &str) {
        for slot in [
            self.baseline_path(path),
            self.local_path(path),
            self.pending_base_path(path),
            self.pending_local_path(path),
        ] {
            let _ = std::fs::remove_file(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilterStore::new(dir.path());

        let mut state = FilterState::new();
        state.files.insert(
            "config.yml".to_string(),
            FilterFileEntry {
                baseline_commit: Some("abc".to_string()),
                conflicted: false,
                added_at: Utc::now(),
            },
        );
        store.save(&state).unwrap();

        let loaded = store.load().unwrap();
        assert!(loaded.files.contains_key("config.yml"));
        assert_eq!(
            loaded.files["config.yml"].baseline_commit.as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn test_load_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilterStore::new(dir.path());
        let state = store.load().unwrap();
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_slot_paths_encoded() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilterStore::new(dir.path());
        let p = store.baseline_path("src/a%b.yml");
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "src%2Fa%25b.yml");
    }

    #[test]
    fn test_remove_file_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilterStore::new(dir.path());
        let b = store.baseline_path("a.txt");
        store.write_slot(&b, b"x").unwrap();
        assert!(b.exists());
        store.remove_file_state("a.txt");
        assert!(!b.exists());
    }
}
