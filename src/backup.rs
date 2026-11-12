use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
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
    pub parts: usize,
    pub split: bool,
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

fn create_upload_archive(output_dir: &Path, staging: &Path, archive_name: &str) -> Result<PathBuf> {
    let archive_path = staging.join(archive_name);
    let file = fs::File::create(&archive_path)
        .with_context(|| format!("failed to create {}", archive_path.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let files = collect_files(output_dir)?;
    for path in files {
        let relative = path
            .strip_prefix(output_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        writer
            .start_file(relative, options)
            .with_context(|| format!("failed to add {} to upload archive", path.display()))?;
        let mut input = fs::File::open(&path)?;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read])?;
        }
    }
    writer.finish()?;
    Ok(archive_path)
}

fn split_archive(
    archive: &Path,
    output_dir: &Path,
    archive_name: &str,
    part_size: u64,
) -> Result<Vec<(PathBuf, u64)>> {
    fs::create_dir_all(output_dir)?;
    let total = fs::metadata(archive)?.len();
    if part_size == 0 || total <= part_size {
        let destination = output_dir.join(archive_name);
        fs::rename(archive, &destination)
            .or_else(|_| fs::copy(archive, &destination).map(|_| ()))?;
        let size = fs::metadata(&destination)?.len();
        return Ok(vec![(destination, size)]);
    }
    let mut input = BufReader::new(fs::File::open(archive)?);
    let mut parts = Vec::new();
    let mut index = 1u32;
    loop {
        let part_name = format!("{archive_name}.part{index:03}");
        let path = output_dir.join(&part_name);
        let mut output = BufWriter::new(fs::File::create(&path)?);
        let mut remaining = part_size;
        let mut written = 0u64;
        let mut buffer = [0u8; 128 * 1024];
        while remaining > 0 {
            let take = remaining.min(buffer.len() as u64) as usize;
            let read = input.read(&mut buffer[..take])?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            written += read as u64;
            remaining -= read as u64;
        }
        output.flush()?;
        if written == 0 {
            let _ = fs::remove_file(&path);
            break;
        }
        parts.push((path, written));
        index += 1;
        if written < part_size {
            break;
        }
    }
    let _ = fs::remove_file(archive);
    Ok(parts)
}

fn write_parts_manifest(
    output_dir: &Path,
    archive_name: &str,
    part_size: u64,
    parts: &[(PathBuf, u64)],
) -> Result<PathBuf> {
    let manifest_name = format!("{archive_name}.parts.json");
    let manifest_path = output_dir.join(&manifest_name);
    let entries: Vec<serde_json::Value> = parts
        .iter()
        .map(|(path, size)| {
            serde_json::json!({
                "name": path.file_name().and_then(|value| value.to_str()).unwrap_or(""),
                "size": size,
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "format": "wechat-exporter-split-archive-v1",
        "archive": archive_name,
        "part_size": part_size,
        "parts": entries,
        "join": "按 parts 顺序拼接所有分卷，再重命名为 archive",
    });
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest_path)
}

/// Upload a directory as one ZIP archive. If `part_size_mb` is nonzero and the
/// archive is larger, it is split into fixed-size parts plus a parts manifest.
pub fn upload_directory(
    output_dir: &Path,
    progress: &mut Progress,
    part_size_mb: u32,
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
    let archive_name = format!("{folder}.zip");
    let staging = tempfile::tempdir().context("failed to create upload staging directory")?;
    progress.set_phase("打包上传文件");
    let archive = create_upload_archive(output_dir, staging.path(), &archive_name)?;
    let part_size = part_size_mb as u64 * 1024 * 1024;
    let upload_dir = staging.path().join("upload");
    fs::create_dir_all(&upload_dir)?;
    let parts = split_archive(&archive, &upload_dir, &archive_name, part_size)?;
    let split = parts.len() > 1;
    if split {
        write_parts_manifest(&upload_dir, &archive_name, part_size, &parts)?;
    }
    let files = collect_files(&upload_dir)?;
    let mut path_parts: Vec<&str> = config
        .remote_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    path_parts.push(folder);
    let collection = client.ensure_collection(&config.url, &path_parts)?;
    progress.set_phase(format!(
        "WebDAV 备份 · 打包为 {} 个{}",
        files.len(),
        if split { "分卷" } else { "文件" }
    ));
    progress.steps(0, files.len(), if split { "分卷" } else { "文件" });
    struct Task {
        path: PathBuf,
        url: String,
    }
    let mut tasks = Vec::with_capacity(files.len());
    let manifest_name = format!("{archive_name}.parts.json");
    let mut manifest_task: Option<Task> = None;
    for path in &files {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("archive");
        let task = Task {
            path: path.clone(),
            url: format!("{}/{}", collection, encode_segment(name)),
        };
        if split && name == manifest_name {
            manifest_task = Some(task);
        } else {
            tasks.push(task);
        }
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
                        progress.steps(done, tasks.len(), if split { "分卷" } else { "文件" });
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
    if let Some(task) = manifest_task {
        let bytes = fs::read(&task.path)
            .with_context(|| format!("failed to read {}", task.path.display()))?;
        client
            .put(&task.url, &bytes)
            .with_context(|| format!("failed to upload manifest {}", task.path.display()))?;
    }
    Ok(UploadSummary {
        remote: collection,
        files: files.len(),
        parts: parts.len(),
        split,
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
    fn archives_and_splits_upload_parts() {
        let temp = tempfile::TempDir::new().unwrap();
        let output = temp.path().join("export");
        std::fs::create_dir_all(output.join("media")).unwrap();
        std::fs::write(output.join("conversation.json"), vec![b'a'; 8192]).unwrap();
        std::fs::write(output.join("media").join("image.png"), vec![b'b'; 8192]).unwrap();
        let staging = tempfile::TempDir::new().unwrap();
        let archive = create_upload_archive(&output, staging.path(), "export.zip").unwrap();
        let original = std::fs::read(&archive).unwrap();
        let parts_dir = staging.path().join("parts");
        let parts = split_archive(&archive, &parts_dir, "export.zip", 128).unwrap();
        assert!(parts.len() > 1, "archive should be split");
        assert!(parts.iter().all(|(_, size)| *size <= 128));
        let manifest_path = write_parts_manifest(&parts_dir, "export.zip", 128, &parts).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["parts"].as_array().unwrap().len(), parts.len());
        let mut joined = Vec::new();
        for (path, _) in &parts {
            joined.extend(std::fs::read(path).unwrap());
        }
        assert_eq!(joined, original);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(joined)).unwrap();
        assert!(zip.by_name("conversation.json").is_ok());
        assert!(zip.by_name("media/image.png").is_ok());
    }

    #[test]
    fn joins_and_encodes_remote_paths() {
        assert_eq!(
            join_url("https://dav.example.com/root/", "wechat exports/2026"),
            "https://dav.example.com/root/wechat%20exports/2026"
        );
    }
}
