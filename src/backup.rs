use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::BackupCommand;
use crate::progress::Progress;

pub const BACKUP_CONFIG_FILE: &str = ".wechat-exporter-backup.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupConfig {
    pub provider: String,
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_remote_path")]
    pub remote_path: String,
}

fn default_remote_path() -> String {
    "wechat-exports".to_string()
}

impl BackupConfig {
    pub fn load() -> Option<Self> {
        let path = Path::new(BACKUP_CONFIG_FILE);
        if !path.exists() {
            return None;
        }
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|config| config.provider.eq_ignore_ascii_case("webdav"))
    }

    fn save(&self) -> Result<PathBuf> {
        let data = serde_json::to_vec_pretty(self).context("serialize backup config")?;
        fs::write(BACKUP_CONFIG_FILE, data)
            .with_context(|| format!("failed to write {BACKUP_CONFIG_FILE}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(
                BACKUP_CONFIG_FILE,
                fs::Permissions::from_mode(0o600),
            );
        }
        Ok(PathBuf::from(BACKUP_CONFIG_FILE))
    }

    fn endpoint(&self) -> String {
        join_url(&self.url, &self.remote_path)
    }
}

pub struct UploadSummary {
    pub remote: String,
    pub files: usize,
}

struct DavClient {
    username: String,
    password: String,
}

impl DavClient {
    fn auth(&self) -> String {
        let raw = format!("{}:{}", self.username, self.password);
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))
    }

    fn request(&self, method: &str, url: &str, body: Option<&[u8]>) -> Result<ureq::Response> {
        let request = ureq::request(method, url)
            .set("Authorization", &self.auth())
            .set("User-Agent", "wechat-exporter-backup")
            .timeout(Duration::from_secs(120));
        match body {
            Some(bytes) => request
                .set("Content-Type", "application/octet-stream")
                .send_bytes(bytes)
                .with_context(|| format!("WebDAV {method} {url} failed")),
            None => request
                .call()
                .with_context(|| format!("WebDAV {method} {url} failed")),
        }
    }

    fn mkcol(&self, url: &str) -> Result<()> {
        match self.request("MKCOL", url, None) {
            Ok(_) => Ok(()),
            Err(error) => {
                if let Some(ureq::Error::Status(405, _)) = error.downcast_ref::<ureq::Error>() {
                    return Ok(());
                }
                Err(error)
            }
        }
    }

    fn ensure_collection(&self, base: &str, segments: &[&str]) -> Result<String> {
        let mut url = base.trim_end_matches('/').to_string();
        for segment in segments {
            url.push('/');
            url.push_str(&encode_segment(segment));
            self.mkcol(&url)?;
        }
        Ok(url)
    }

    fn put(&self, url: &str, bytes: &[u8]) -> Result<()> {
        self.request("PUT", url, Some(bytes))?;
        Ok(())
    }

    fn test(&self, url: &str) -> Result<()> {
        let response = ureq::request("PROPFIND", url)
            .set("Authorization", &self.auth())
            .set("Depth", "0")
            .set("User-Agent", "wechat-exporter-backup")
            .timeout(Duration::from_secs(30))
            .call()
            .with_context(|| format!("WebDAV test failed: {url}"))?;
        let status = response.status();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            bail!("WebDAV test returned HTTP {status}");
        }
    }
}

fn encode_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn join_url(base: &str, path: &str) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        url.push('/');
        url.push_str(&encode_segment(segment));
    }
    url
}

fn collect_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn upload_directory(
    output_dir: &Path,
    progress: &mut Progress,
) -> Result<UploadSummary> {
    let config = BackupConfig::load().context(
        "WebDAV backup is not configured; run `wechat-exporter backup webdav --url ... --username ... --password ...`",
    )?;
    let client = DavClient {
        username: config.username.clone(),
        password: config.password.clone(),
    };
    let folder = output_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("export");
    let mut path_parts: Vec<&str> = config
        .remote_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    path_parts.push(folder);
    let collection = client.ensure_collection(&config.url, &path_parts)?;
    let files = collect_files(output_dir)?;
    progress.set_phase(format!("WebDAV 备份 · {} 个文件", files.len()));
    progress.steps(0, files.len(), "文件");
    struct Task {
        path: PathBuf,
        url: String,
    }
    let mut tasks = Vec::with_capacity(files.len());
    let mut parents: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for path in &files {
        let relative = path
            .strip_prefix(output_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let parts: Vec<&str> = relative.split('/').filter(|part| !part.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let (parent_parts, file_name) = parts.split_at(parts.len() - 1);
        let parent_key = parent_parts.join("/");
        let parent_url = if let Some(url) = parents.get(&parent_key) {
            url.clone()
        } else {
            let url = client.ensure_collection(&collection, parent_parts)?;
            parents.insert(parent_key, url.clone());
            url
        };
        tasks.push(Task {
            path: path.clone(),
            url: format!("{}/{}", parent_url, encode_segment(file_name.first().copied().unwrap_or("file"))),
        });
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::atomic::AtomicBool::new(false);
    let errors = std::sync::Mutex::new(Vec::<String>::new());
    let progress = std::sync::Mutex::new(progress);
    let workers = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .min(tasks.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next = &next;
            let completed = &completed;
            let failed = &failed;
            let errors = &errors;
            let progress = &progress;
            let client = &client;
            let tasks = &tasks;
            scope.spawn(move || {
                loop {
                    if failed.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= tasks.len() {
                        break;
                    }
                    let task = &tasks[index];
                    let result = fs::read(&task.path)
                        .with_context(|| format!("failed to read {}", task.path.display()))
                        .and_then(|bytes| client.put(&task.url, &bytes));
                    if let Err(error) = result {
                        failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(mut errors) = errors.lock() {
                            errors.push(format!("{}: {error:#}", task.path.display()));
                        }
                        break;
                    }
                    let done = completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                        + 1;
                    if let Ok(mut progress) = progress.lock() {
                        progress.steps(done, tasks.len(), "文件");
                    }
                }
            });
        }
    });
    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        let message = errors
            .lock()
            .ok()
            .and_then(|errors| errors.first().cloned())
            .unwrap_or_else(|| "WebDAV upload failed".to_string());
        bail!(message);
    }
    Ok(UploadSummary {
        remote: collection,
        files: files.len(),
    })
}

pub fn run(action: &BackupCommand) -> Result<()> {
    match action {
        BackupCommand::Webdav(args) => {
            let url = args.url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                bail!("backup url must start with http:// or https://");
            }
            if args.username.trim().is_empty() {
                bail!("backup username must not be empty");
            }
            if args.password.is_empty() {
                bail!("backup password must not be empty");
            }
            let config = BackupConfig {
                provider: "webdav".to_string(),
                url: url.to_string(),
                username: args.username.clone(),
                password: args.password.clone(),
                remote_path: if args.remote_path.trim().is_empty() {
                    default_remote_path()
                } else {
                    args.remote_path.trim().to_string()
                },
            };
            let path = config.save()?;
            println!("[+] WebDAV 备份配置已保存: {}", path.display());
            println!("    remote: {}", config.endpoint());
            Ok(())
        }
        BackupCommand::Show => {
            let Some(config) = BackupConfig::load() else {
                println!("尚未配置备份提供商。");
                return Ok(());
            };
            println!("provider: {}", config.provider);
            println!("url: {}", config.url);
            println!("username: {}", config.username);
            println!("password: {}", "*".repeat(config.password.chars().count().min(24)));
            println!("remote_path: {}", config.remote_path);
            Ok(())
        }
        BackupCommand::Test => {
            let config = BackupConfig::load().context("尚未配置备份提供商")?;
            let client = DavClient {
                username: config.username.clone(),
                password: config.password.clone(),
            };
            let test_url = format!("{}/", config.url.trim_end_matches('/'));
            client.test(&test_url)?;
            println!("[+] WebDAV 连接正常: {test_url}");
            Ok(())
        }
        BackupCommand::Clear => {
            let path = Path::new(BACKUP_CONFIG_FILE);
            if path.exists() {
                fs::remove_file(path)?;
                println!("[+] 已删除 WebDAV 备份配置");
            } else {
                println!("没有可删除的备份配置。");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_encodes_remote_paths() {
        assert_eq!(
            join_url("https://dav.example.com/root/", "wechat exports/2026"),
            "https://dav.example.com/root/wechat%20exports/2026"
        );
    }
}
