use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = ".wechat-exporter.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExporterConfig {
    pub account_id: String,
    pub source_database: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_seed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_aes_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_xor_key: Option<u8>,
    /// WeChat login cookies captured by `init` (or supplied with `--cookie`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cookies: String,
    /// C2C CDN download ticket captured from the client (`authkey`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_key: String,
    pub updated_at: i64,
}

impl ExporterConfig {
    pub fn load(search_roots: &[&Path]) -> Option<Self> {
        // 1. Current working directory
        let cwd_file = Path::new(CONFIG_FILE_NAME);
        if cwd_file.exists() {
            if let Ok(config) = Self::read_file(cwd_file) {
                return Some(config);
            }
        }
        // 2. Search roots (account directory, db_storage parent)
        for root in search_roots {
            let candidate = root.join(CONFIG_FILE_NAME);
            if candidate.exists() {
                if let Ok(config) = Self::read_file(&candidate) {
                    return Some(config);
                }
            }
        }
        None
    }

    fn read_file(path: &Path) -> Result<Self> {
        let content = fs::read(path)?;
        let config: Self = serde_json::from_slice(&content)?;
        Ok(config)
    }

    pub fn save(&self, target_dir: Option<&Path>) -> Result<PathBuf> {
        let path = target_dir
            .map(|d| d.join(CONFIG_FILE_NAME))
            .unwrap_or_else(|| PathBuf::from(CONFIG_FILE_NAME));
        let data = serde_json::to_vec_pretty(self)?;
        fs::write(&path, data)
            .with_context(|| format!("failed to write config to {}", path.display()))?;
        Ok(path)
    }

    pub fn db_key_bytes(&self) -> Option<[u8; 32]> {
        let raw = self.key.trim();
        if raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
            let bytes = hex::decode(raw).ok()?;
            if bytes.len() == 32 {
                let mut out = [0u8; 32];
                out.copy_from_slice(&bytes);
                return Some(out);
            }
        }
        None
    }

    pub fn image_aes_key_bytes(&self) -> Option<[u8; 16]> {
        let raw = self.image_aes_key.as_deref()?.trim();
        if raw.len() == 32 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
            let bytes = hex::decode(raw).ok()?;
            if bytes.len() == 16 {
                let mut out = [0u8; 16];
                out.copy_from_slice(&bytes);
                return Some(out);
            }
        } else if raw.len() == 16 {
            let mut out = [0u8; 16];
            out.copy_from_slice(raw.as_bytes());
            return Some(out);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_and_reloads_config() {
        let temp = tempfile::TempDir::new().unwrap();
        let cfg = ExporterConfig {
            account_id: "wxid_test123".into(),
            source_database: "/path/to/db".into(),
            key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            image_seed: Some(3642459924),
            image_aes_key: Some("b866e5aa60cda50c".into()),
            image_xor_key: Some(0x14),
            cookies: String::new(),
            auth_key: String::new(),
            updated_at: 1730000000,
        };
        let saved = cfg.save(Some(temp.path())).unwrap();
        assert!(saved.exists());
        let loaded = ExporterConfig::load(&[temp.path()]).expect("load config");
        assert_eq!(loaded.account_id, "wxid_test123");
        assert_eq!(loaded.db_key_bytes().unwrap()[..4], [0x01, 0x23, 0x45, 0x67]);
        assert_eq!(loaded.image_seed, Some(3642459924));
        assert_eq!(loaded.image_xor_key, Some(0x14));
    }
}
