use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ShadowError;
use crate::fs_util;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Overlay,
    Phantom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeMode {
    GitInfoExclude,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    #[serde(rename = "type")]
    pub file_type: FileType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_commit: Option<String>,
    pub exclude_mode: ExcludeMode,
    #[serde(default)]
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_directory: bool,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowConfig {
    pub version: u32,
    pub files: BTreeMap<String, FileEntry>,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            version: 1,
            files: BTreeMap::new(),
        }
    }
}

impl ShadowConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(shadow_dir: &Path) -> anyhow::Result<Self> {
        let config_path = shadow_dir.join("config.json");
        if !config_path.exists() {
            return Ok(Self::new());
        }
        let content =
            std::fs::read_to_string(&config_path).context("failed to read config.json")?;
        let config: Self = serde_json::from_str(&content).context("failed to parse config.json")?;
        Ok(config)
    }

    pub fn save(&self, shadow_dir: &Path) -> anyhow::Result<()> {
        let config_path = shadow_dir.join("config.json");
        let content =
            serde_json::to_string_pretty(self).context("failed to serialize config.json")?;
        fs_util::atomic_write(&config_path, content.as_bytes())
            .context("failed to write config.json")?;
        Ok(())
    }

    pub fn add_overlay(&mut self, path: String, commit: String) -> Result<(), ShadowError> {
        if self.files.contains_key(&path) {
            return Err(ShadowError::AlreadyManaged(path));
        }
        self.files.insert(
            path,
            FileEntry {
                file_type: FileType::Overlay,
                baseline_commit: Some(commit),
                exclude_mode: ExcludeMode::None,
                is_directory: false,
                added_at: Utc::now(),
            },
        );
        Ok(())
    }

    pub fn add_phantom(
        &mut self,
        path: String,
        exclude: ExcludeMode,
        is_directory: bool,
    ) -> Result<(), ShadowError> {
        if self.files.contains_key(&path) {
            return Err(ShadowError::AlreadyManaged(path));
        }
        self.files.insert(
            path,
            FileEntry {
                file_type: FileType::Phantom,
                baseline_commit: None,
                exclude_mode: exclude,
                is_directory,
                added_at: Utc::now(),
            },
        );
        Ok(())
    }

    pub fn remove(&mut self, path: &str) -> Result<FileEntry, ShadowError> {
        self.files
            .remove(path)
            .ok_or_else(|| ShadowError::NotManaged(path.to_string()))
    }

    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.files.get(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config() {
        let config = ShadowConfig::new();
        assert_eq!(config.version, 1);
        assert!(config.files.is_empty());
    }

    #[test]
    fn test_add_overlay() {
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), "abc1234".to_string())
            .unwrap();

        let entry = config.get("CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, FileType::Overlay);
        assert_eq!(entry.baseline_commit.as_deref(), Some("abc1234"));
        assert_eq!(entry.exclude_mode, ExcludeMode::None);
    }

    #[test]
    fn test_add_phantom_with_exclude() {
        let mut config = ShadowConfig::new();
        config
            .add_phantom(
                "src/components/CLAUDE.md".to_string(),
                ExcludeMode::GitInfoExclude,
                false,
            )
            .unwrap();

        let entry = config.get("src/components/CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, FileType::Phantom);
        assert_eq!(entry.baseline_commit, None);
        assert_eq!(entry.exclude_mode, ExcludeMode::GitInfoExclude);
    }

    #[test]
    fn test_add_phantom_no_exclude() {
        let mut config = ShadowConfig::new();
        config
            .add_phantom("test.md".to_string(), ExcludeMode::None, false)
            .unwrap();

        let entry = config.get("test.md").unwrap();
        assert_eq!(entry.exclude_mode, ExcludeMode::None);
    }

    #[test]
    fn test_add_duplicate_returns_error() {
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), "abc1234".to_string())
            .unwrap();
        let result = config.add_overlay("CLAUDE.md".to_string(), "def5678".to_string());
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ShadowError::AlreadyManaged(_)
        ));
    }

    #[test]
    fn test_remove_existing() {
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), "abc1234".to_string())
            .unwrap();

        let entry = config.remove("CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, FileType::Overlay);
        assert!(config.get("CLAUDE.md").is_none());
    }

    #[test]
    fn test_remove_nonexistent_returns_error() {
        let mut config = ShadowConfig::new();
        let result = config.remove("nonexistent.md");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ShadowError::NotManaged(_)));
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let config = ShadowConfig::new();
        assert!(config.get("nonexistent.md").is_none());
    }

    #[test]
    fn test_serialize_matches_spec() {
        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), "abc1234def5678".to_string())
            .unwrap();
        config
            .add_phantom(
                "src/components/CLAUDE.md".to_string(),
                ExcludeMode::GitInfoExclude,
                false,
            )
            .unwrap();

        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["version"], 1);

        let claude = &json["files"]["CLAUDE.md"];
        assert_eq!(claude["type"], "overlay");
        assert_eq!(claude["baseline_commit"], "abc1234def5678");
        assert_eq!(claude["exclude_mode"], "none");

        let component = &json["files"]["src/components/CLAUDE.md"];
        assert_eq!(component["type"], "phantom");
        assert!(component.get("baseline_commit").is_none());
        assert_eq!(component["exclude_mode"], "git_info_exclude");
    }

    #[test]
    fn test_deserialize_from_spec() {
        let json = r#"{
            "version": 1,
            "files": {
                "CLAUDE.md": {
                    "type": "overlay",
                    "baseline_commit": "abc1234def5678",
                    "exclude_mode": "none",
                    "added_at": "2026-02-07T12:00:00Z"
                },
                "src/components/CLAUDE.md": {
                    "type": "phantom",
                    "exclude_mode": "git_info_exclude",
                    "added_at": "2026-02-07T12:00:00Z"
                }
            }
        }"#;

        let config: ShadowConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, 1);
        assert_eq!(config.files.len(), 2);

        let overlay = config.get("CLAUDE.md").unwrap();
        assert_eq!(overlay.file_type, FileType::Overlay);
        assert_eq!(overlay.baseline_commit.as_deref(), Some("abc1234def5678"));

        let phantom = config.get("src/components/CLAUDE.md").unwrap();
        assert_eq!(phantom.file_type, FileType::Phantom);
        assert_eq!(phantom.baseline_commit, None);
    }

    #[test]
    fn test_add_phantom_directory() {
        let mut config = ShadowConfig::new();
        config
            .add_phantom(".claude".to_string(), ExcludeMode::GitInfoExclude, true)
            .unwrap();

        let entry = config.get(".claude").unwrap();
        assert_eq!(entry.file_type, FileType::Phantom);
        assert!(entry.is_directory);
        assert_eq!(entry.exclude_mode, ExcludeMode::GitInfoExclude);
    }

    #[test]
    fn test_add_phantom_file_is_not_directory() {
        let mut config = ShadowConfig::new();
        config
            .add_phantom("local.md".to_string(), ExcludeMode::None, false)
            .unwrap();

        let entry = config.get("local.md").unwrap();
        assert!(!entry.is_directory);
    }

    #[test]
    fn test_deserialize_without_is_directory() {
        // Old config.json without is_directory field should default to false
        let json = r#"{
            "version": 1,
            "files": {
                "local.md": {
                    "type": "phantom",
                    "exclude_mode": "git_info_exclude",
                    "added_at": "2026-02-07T12:00:00Z"
                }
            }
        }"#;

        let config: ShadowConfig = serde_json::from_str(json).unwrap();
        let entry = config.get("local.md").unwrap();
        assert!(!entry.is_directory);
    }

    #[test]
    fn test_serialize_directory_phantom() {
        let mut config = ShadowConfig::new();
        config
            .add_phantom(".claude".to_string(), ExcludeMode::GitInfoExclude, true)
            .unwrap();

        let json = serde_json::to_value(&config).unwrap();
        let entry = &json["files"][".claude"];
        assert_eq!(entry["is_directory"], true);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let shadow_dir = dir.path().join("shadow");
        std::fs::create_dir_all(&shadow_dir).unwrap();

        let mut config = ShadowConfig::new();
        config
            .add_overlay("CLAUDE.md".to_string(), "abc1234".to_string())
            .unwrap();
        config.save(&shadow_dir).unwrap();

        let loaded = ShadowConfig::load(&shadow_dir).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.files.len(), 1);
        let entry = loaded.get("CLAUDE.md").unwrap();
        assert_eq!(entry.file_type, FileType::Overlay);
    }

    #[test]
    fn test_load_nonexistent_returns_new() {
        let dir = tempfile::tempdir().unwrap();
        let shadow_dir = dir.path().join("shadow");
        std::fs::create_dir_all(&shadow_dir).unwrap();

        let config = ShadowConfig::load(&shadow_dir).unwrap();
        assert_eq!(config.version, 1);
        assert!(config.files.is_empty());
    }
}
