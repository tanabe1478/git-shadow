//! Portable archive format for `git-shadow export` / `import`.
//!
//! The archive is a gzipped tar (`.tar.gz`) containing a `manifest.json` plus
//! one content file per managed entry. Tar member names are flat (path
//! separators are URL-encoded via [`crate::path::encode_path`]) so nested repo
//! paths never create nested tar directories:
//!
//! - `overlay/<encoded-path>`   -- overlay shadow (working-tree) content
//! - `baseline/<encoded-path>`  -- overlay pristine baseline content
//! - `phantom/<encoded-path>`   -- phantom file content
//! - `phantomdir/<encoded-member-path>` -- one entry per file under a phantom dir
//!
//! Contents are stored as raw bytes (`Vec<u8>`), so binary phantom/overlay
//! files round-trip byte-for-byte with no UTF-8 assumption.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::ExcludeMode;
use crate::path;

/// Current archive format version. Bumped on incompatible changes.
pub const FORMAT_VERSION: u32 = 1;

/// Name of the manifest member inside the archive.
pub const MANIFEST_NAME: &str = "manifest.json";

/// Type of a managed entry as recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestType {
    Overlay,
    Phantom,
    PhantomDir,
}

/// One managed entry described in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Repository-relative path of the entry.
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: ManifestType,
    pub exclude_mode: ExcludeMode,
    /// Overlay only: the commit the archived baseline was captured from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_commit: Option<String>,
    /// Phantom dir only: repo-relative paths of every member file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir_members: Option<Vec<String>>,
}

/// Top-level archive manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub tool_version: String,
    pub entries: Vec<ManifestEntry>,
}

/// Tar member name for an overlay's shadow (working-tree) content.
pub fn overlay_shadow_key(path: &str) -> String {
    format!("overlay/{}", path::encode_path(path))
}

/// Tar member name for an overlay's pristine baseline content.
pub fn overlay_baseline_key(path: &str) -> String {
    format!("baseline/{}", path::encode_path(path))
}

/// Tar member name for a phantom file's content.
pub fn phantom_key(path: &str) -> String {
    format!("phantom/{}", path::encode_path(path))
}

/// Tar member name for a single file under a phantom directory.
pub fn phantom_dir_member_key(member_path: &str) -> String {
    format!("phantomdir/{}", path::encode_path(member_path))
}

/// Write a gzipped tar archive containing the manifest and content files.
///
/// The write is atomic: the archive is built in a temporary file in the target's
/// parent directory and renamed into place, so a crash mid-write cannot leave a
/// truncated archive at `output`.
pub fn write_archive(
    output: &Path,
    manifest: &Manifest,
    contents: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let tmp = tempfile::Builder::new()
        .prefix(".git-shadow-export-")
        .suffix(".tmp")
        .tempfile_in(&parent)
        .context("failed to create temporary archive file")?;

    {
        let encoder = flate2::write::GzEncoder::new(
            std::io::BufWriter::new(tmp.as_file()),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);

        let manifest_json =
            serde_json::to_vec_pretty(manifest).context("failed to serialize manifest")?;
        append_bytes(&mut builder, MANIFEST_NAME, &manifest_json)?;

        for (name, data) in contents {
            append_bytes(&mut builder, name, data)?;
        }

        let encoder = builder.into_inner().context("failed to finalize tar")?;
        let writer = encoder.finish().context("failed to finish gzip stream")?;
        writer
            .into_inner()
            .map_err(|e| anyhow::anyhow!("failed to flush archive: {}", e.into_error()))?;
    }

    tmp.persist(output)
        .map_err(|e| anyhow::anyhow!("failed to write archive to {}: {}", output.display(), e))?;
    Ok(())
}

fn append_bytes<W: Write>(builder: &mut tar::Builder<W>, name: &str, data: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, name, data)
        .with_context(|| format!("failed to append {} to archive", name))?;
    Ok(())
}

/// Read a gzipped tar archive, returning its manifest and all content members.
pub fn read_archive(input: &Path) -> Result<(Manifest, BTreeMap<String, Vec<u8>>)> {
    let file = std::fs::File::open(input)
        .with_context(|| format!("failed to open archive {}", input.display()))?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);

    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut manifest: Option<Manifest> = None;

    for entry in archive
        .entries()
        .context("failed to read archive entries")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let name = entry
            .path()
            .context("archive entry has an invalid path")?
            .to_string_lossy()
            .to_string();

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .with_context(|| format!("failed to read archive member {}", name))?;

        if name == MANIFEST_NAME {
            manifest = Some(serde_json::from_slice(&buf).context("failed to parse manifest.json")?);
        } else {
            contents.insert(name, buf);
        }
    }

    let manifest = manifest.context("archive is missing manifest.json")?;
    Ok((manifest, contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        Manifest {
            format_version: FORMAT_VERSION,
            tool_version: "0.0.0-test".to_string(),
            entries: vec![
                ManifestEntry {
                    path: "CLAUDE.md".to_string(),
                    entry_type: ManifestType::Overlay,
                    exclude_mode: ExcludeMode::None,
                    baseline_commit: Some("abc1234".to_string()),
                    dir_members: None,
                },
                ManifestEntry {
                    path: ".claude".to_string(),
                    entry_type: ManifestType::PhantomDir,
                    exclude_mode: ExcludeMode::GitInfoExclude,
                    baseline_commit: None,
                    dir_members: Some(vec![".claude/settings.json".to_string()]),
                },
            ],
        }
    }

    #[test]
    fn test_key_encoding_is_flat() {
        assert_eq!(overlay_shadow_key("a/b.md"), "overlay/a%2Fb.md");
        assert_eq!(overlay_baseline_key("a/b.md"), "baseline/a%2Fb.md");
        assert_eq!(phantom_key("dir/x"), "phantom/dir%2Fx");
        assert_eq!(
            phantom_dir_member_key(".claude/s.json"),
            "phantomdir/.claude%2Fs.json"
        );
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("export.tar.gz");

        let mut contents = BTreeMap::new();
        contents.insert(overlay_shadow_key("CLAUDE.md"), b"# shadow\n".to_vec());
        contents.insert(overlay_baseline_key("CLAUDE.md"), b"# base\n".to_vec());
        // Binary content with a null byte must survive intact.
        contents.insert(
            phantom_dir_member_key(".claude/settings.json"),
            vec![0x00, 0x01, 0xff, b'x'],
        );

        write_archive(&output, &sample_manifest(), &contents).unwrap();
        assert!(output.exists());

        let (manifest, read) = read_archive(&output).unwrap();
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(
            read.get(&overlay_shadow_key("CLAUDE.md")).unwrap(),
            b"# shadow\n"
        );
        assert_eq!(
            read.get(&phantom_dir_member_key(".claude/settings.json"))
                .unwrap(),
            &vec![0x00, 0x01, 0xff, b'x']
        );
    }

    #[test]
    fn test_read_missing_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("bad.tar.gz");

        // Build an archive with no manifest.
        let file = std::fs::File::create(&output).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append_bytes(&mut builder, "phantom/x", b"data").unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let err = read_archive(&output).unwrap_err();
        assert!(err.to_string().contains("manifest"));
    }
}
