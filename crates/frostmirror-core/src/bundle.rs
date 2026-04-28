use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::manifest::Manifest;

/// Section kinds in a `.pkg` bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Manifest,
    Crate,
    Index,
    Rustup,
    /// Toolchain distribution files (channel manifests + component archives).
    Dist,
    Config,
}

impl SectionKind {
    pub fn prefix(&self) -> &'static str {
        match self {
            SectionKind::Manifest => "manifest.json",
            SectionKind::Crate => "crates/",
            SectionKind::Index => "index/",
            SectionKind::Rustup => "rustup/",
            SectionKind::Dist => "dist/",
            SectionKind::Config => "config.toml",
        }
    }

    fn from_path(path: &str) -> Self {
        if path == "manifest.json" {
            SectionKind::Manifest
        } else if path.starts_with("crates/") {
            SectionKind::Crate
        } else if path.starts_with("index/") {
            SectionKind::Index
        } else if path.starts_with("rustup/") {
            SectionKind::Rustup
        } else if path.starts_with("dist/") {
            SectionKind::Dist
        } else if path == "config.toml" || path.starts_with("config/") {
            SectionKind::Config
        } else {
            SectionKind::Crate // fallback
        }
    }
}

/// A section within the bundle.
#[derive(Debug, Clone)]
pub struct Section {
    pub kind: SectionKind,
    pub path: String,
    pub data: Vec<u8>,
}

/// High-level `.pkg` bundle representation.
pub struct Bundle {
    pub manifest: Manifest,
    pub sections: Vec<Section>,
}

/// Builds a `.pkg` bundle file.
pub struct BundleBuilder {
    sections: Vec<Section>,
}

impl BundleBuilder {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    pub fn add_section(&mut self, kind: SectionKind, path: String, data: Vec<u8>) {
        self.sections.push(Section { kind, path, data });
    }

    pub fn add_manifest(&mut self, manifest: &Manifest) -> Result<()> {
        let json = manifest.to_json()?;
        self.add_section(
            SectionKind::Manifest,
            "manifest.json".to_string(),
            json.into_bytes(),
        );
        Ok(())
    }

    pub fn add_crate_file(&mut self, name: &str, version: &str, data: Vec<u8>) {
        let path = format!("crates/{}/{}/download", name, version);
        self.add_section(SectionKind::Crate, path, data);
    }

    pub fn add_index_entry(&mut self, name: &str, data: Vec<u8>) {
        let index_path = crate_index_path(name);
        self.add_section(SectionKind::Index, format!("index/{}", index_path), data);
    }

    pub fn add_rustup_file(&mut self, target: &str, filename: &str, data: Vec<u8>) {
        let path = format!("rustup/dist/{}/{}", target, filename);
        self.add_section(SectionKind::Rustup, path, data);
    }

    pub fn add_dist_file(&mut self, relative_path: &str, data: Vec<u8>) {
        let path = format!("dist/{}", relative_path);
        self.add_section(SectionKind::Dist, path, data);
    }

    pub fn add_config(&mut self, config_toml: &str) {
        self.add_section(
            SectionKind::Config,
            "config.toml".to_string(),
            config_toml.as_bytes().to_vec(),
        );
    }

    /// Add a named config file under the `config/` prefix. Used by snapshot
    /// exports to bundle multiple files (e.g. `frostmirror.toml`,
    /// `depends.toml`) so the importer can place them back into the config dir.
    pub fn add_config_file(&mut self, filename: &str, data: Vec<u8>) {
        self.add_section(
            SectionKind::Config,
            format!("config/{}", filename),
            data,
        );
    }

    /// Write the bundle to a file, compressed with brotli.
    ///
    /// Bundle format (before brotli compression):
    /// - 8 bytes: magic "FMPKG\x00\x01\x00"
    /// - 4 bytes: number of sections (u32 LE)
    /// - For each section in the header table:
    ///   - 4 bytes: path length (u32 LE)
    ///   - N bytes: path (UTF-8)
    ///   - 8 bytes: data offset (u64 LE)
    ///   - 8 bytes: data length (u64 LE)
    /// - Raw section data concatenated
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let raw = self.build_raw()?;

        let file = std::fs::File::create(path)
            .with_context(|| format!("failed to create bundle at {}", path.display()))?;
        let mut encoder = brotli::CompressorWriter::new(file, 4096, 6, 22);
        encoder.write_all(&raw)?;
        encoder.flush()?;
        drop(encoder);

        Ok(())
    }

    fn build_raw(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Magic
        buf.extend_from_slice(b"FMPKG\x00\x01\x00");

        // Section count
        let count = self.sections.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());

        // Calculate offsets: header comes first, then data
        let mut header_size = 0u64;
        for s in &self.sections {
            header_size += 4 + s.path.len() as u64 + 8 + 8;
        }

        let mut data_offset = 8 + 4 + header_size; // magic + count + header
        let mut offsets = Vec::new();
        for s in &self.sections {
            offsets.push((data_offset, s.data.len() as u64));
            data_offset += s.data.len() as u64;
        }

        // Write header table
        for (i, s) in self.sections.iter().enumerate() {
            let path_bytes = s.path.as_bytes();
            buf.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(path_bytes);
            buf.extend_from_slice(&offsets[i].0.to_le_bytes());
            buf.extend_from_slice(&offsets[i].1.to_le_bytes());
        }

        // Write section data
        for s in &self.sections {
            buf.extend_from_slice(&s.data);
        }

        Ok(buf)
    }
}

/// Reads a `.pkg` bundle file.
pub struct BundleReader;

impl BundleReader {
    /// Read and decompress a `.pkg` file, returning all sections.
    pub fn read_file(path: &Path) -> Result<Bundle> {
        let compressed = std::fs::read(path)
            .with_context(|| format!("failed to read bundle at {}", path.display()))?;
        Self::read_bytes(&compressed)
    }

    /// Read from compressed bytes.
    pub fn read_bytes(compressed: &[u8]) -> Result<Bundle> {
        let mut decompressed = Vec::new();
        let mut decoder = brotli::Decompressor::new(compressed, 4096);
        decoder
            .read_to_end(&mut decompressed)
            .context("failed to decompress bundle")?;

        Self::parse_raw(&decompressed)
    }

    fn parse_raw(data: &[u8]) -> Result<Bundle> {
        if data.len() < 12 {
            bail!("bundle too small");
        }

        // Verify magic
        if &data[0..8] != b"FMPKG\x00\x01\x00" {
            bail!("invalid bundle magic");
        }

        let section_count = u32::from_le_bytes(data[8..12].try_into()?) as usize;
        let mut pos = 12;

        // Read header table
        let mut entries = Vec::new();
        for _ in 0..section_count {
            if pos + 4 > data.len() {
                bail!("truncated header");
            }
            let path_len = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
            pos += 4;

            if pos + path_len > data.len() {
                bail!("truncated header path");
            }
            let path = std::str::from_utf8(&data[pos..pos + path_len])
                .context("invalid UTF-8 in section path")?
                .to_string();
            pos += path_len;

            if pos + 16 > data.len() {
                bail!("truncated header offsets");
            }
            let offset = u64::from_le_bytes(data[pos..pos + 8].try_into()?) as usize;
            pos += 8;
            let length = u64::from_le_bytes(data[pos..pos + 8].try_into()?) as usize;
            pos += 8;

            entries.push((path, offset, length));
        }

        // Read sections
        let mut sections = Vec::new();
        for (path, offset, length) in &entries {
            if offset + length > data.len() {
                bail!(
                    "section '{}' extends past end of bundle (offset={}, len={}, total={})",
                    path,
                    offset,
                    length,
                    data.len()
                );
            }
            let section_data = data[*offset..*offset + *length].to_vec();
            let kind = SectionKind::from_path(path);
            sections.push(Section {
                kind,
                path: path.clone(),
                data: section_data,
            });
        }

        // Extract manifest
        let manifest_section = sections
            .iter()
            .find(|s| s.kind == SectionKind::Manifest)
            .context("bundle has no manifest")?;
        let manifest_json =
            std::str::from_utf8(&manifest_section.data).context("manifest is not valid UTF-8")?;
        let manifest = Manifest::from_json(manifest_json)?;

        Ok(Bundle { manifest, sections })
    }

    /// Verify the integrity of a bundle: check manifest hash and per-crate SHA-256.
    pub fn verify(bundle: &Bundle) -> Result<()> {
        // Verify manifest hash
        bundle.manifest.verify_hash()?;

        // Build a map of crate path → data for SHA-256 checking
        let crate_data: BTreeMap<String, &[u8]> = bundle
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Crate)
            .map(|s| (s.path.clone(), s.data.as_slice()))
            .collect();

        for entry in bundle.manifest.crates.values() {
            let path = format!("crates/{}/{}/download", entry.name, entry.version);
            let data = crate_data
                .get(&path)
                .ok_or_else(|| anyhow::anyhow!("missing crate file: {}", path))?;

            let mut hasher = Sha256::new();
            hasher.update(data);
            let hash = hex::encode(hasher.finalize());

            if hash != entry.sha256 {
                bail!(
                    "SHA-256 mismatch for {}-{}: expected {}, got {}",
                    entry.name,
                    entry.version,
                    entry.sha256,
                    hash
                );
            }
        }

        Ok(())
    }
}

/// Compute the sparse index path for a crate name.
/// See: https://doc.rust-lang.org/cargo/reference/registry-index.html
pub fn crate_index_path(name: &str) -> String {
    match name.len() {
        1 => format!("1/{}", name),
        2 => format!("2/{}", name),
        3 => format!("3/{}/{}", &name[..1], name),
        _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
    }
}

/// Generate the pkg filename from the current timestamp.
pub fn pkg_filename() -> String {
    let now = chrono::Utc::now();
    format!("{}-crates.pkg", now.format("%Y%m%d-%H%M"))
}

/// Compute SHA-256 of arbitrary data.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BundleType, Manifest};

    #[test]
    fn test_crate_index_path() {
        assert_eq!(crate_index_path("a"), "1/a");
        assert_eq!(crate_index_path("ab"), "2/ab");
        assert_eq!(crate_index_path("abc"), "3/a/abc");
        assert_eq!(crate_index_path("tokio"), "to/ki/tokio");
        assert_eq!(crate_index_path("serde"), "se/rd/serde");
    }

    #[test]
    fn test_bundle_roundtrip() {
        let mut manifest = Manifest::new(
            BundleType::Full,
            None,
            vec!["x86_64-unknown-linux-gnu".into()],
            "stable".into(),
        );
        let crate_data = b"fake crate data";
        let hash = sha256_hex(crate_data);
        manifest.add_crate("test-crate".into(), "1.0.0".into(), hash, crate_data.len() as u64);
        manifest.seal();

        let mut builder = BundleBuilder::new();
        builder.add_manifest(&manifest).unwrap();
        builder.add_crate_file("test-crate", "1.0.0", crate_data.to_vec());
        builder.add_index_entry("test-crate", b"index data".to_vec());
        builder.add_config("# config");

        let tmp = tempfile::NamedTempFile::new().unwrap();
        builder.write_to_file(tmp.path()).unwrap();

        let bundle = BundleReader::read_file(tmp.path()).unwrap();
        assert_eq!(bundle.manifest.crates.len(), 1);
        assert_eq!(bundle.sections.len(), 4);

        BundleReader::verify(&bundle).unwrap();
    }

    #[test]
    fn test_config_file_roundtrip() {
        let mut manifest = Manifest::new(
            BundleType::Full,
            None,
            vec!["x86_64-unknown-linux-gnu".into()],
            "stable".into(),
        );
        manifest.seal();

        let mut builder = BundleBuilder::new();
        builder.add_manifest(&manifest).unwrap();
        builder.add_config_file("frostmirror.toml", b"base_url = \"x\"".to_vec());
        builder.add_config_file("depends.toml", b"[dependencies]\n".to_vec());

        let tmp = tempfile::NamedTempFile::new().unwrap();
        builder.write_to_file(tmp.path()).unwrap();

        let bundle = BundleReader::read_file(tmp.path()).unwrap();
        let config_sections: Vec<_> = bundle
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Config)
            .collect();
        assert_eq!(config_sections.len(), 2);
        let paths: std::collections::BTreeSet<_> =
            config_sections.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains("config/frostmirror.toml"));
        assert!(paths.contains("config/depends.toml"));
    }
}
