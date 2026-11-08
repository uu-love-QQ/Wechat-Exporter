use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
#[cfg(windows)]
use std::process::Command;

const REPO: &str = "uu-love-QQ/Wechat-Exporter";

fn asset_name() -> &'static str {
    if cfg!(windows) {
        "wechat-exporter.exe"
    } else {
        "wechat-exporter"
    }
}

fn download_url(version: &str, prefix: Option<&str>) -> String {
    let tag = if version == "latest" {
        "latest".to_string()
    } else if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let base = if tag == "latest" {
        format!(
            "https://github.com/{REPO}/releases/latest/download/{}",
            asset_name()
        )
    } else {
        format!(
            "https://github.com/{REPO}/releases/download/{tag}/{}",
            asset_name()
        )
    };
    match prefix {
        Some(prefix) if !prefix.trim().is_empty() => {
            format!("{}/{}", prefix.trim_end_matches('/'), base)
        }
        _ => base,
    }
}

fn download(url: &str) -> Result<Vec<u8>> {
    let response = ureq::get(url)
        .set("User-Agent", "wechat-exporter-updater")
        .timeout(Duration::from_secs(120))
        .call()
        .with_context(|| format!("download failed: {url}"))?;
    let mut data = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut data)
        .context("failed to read upgrade download")?;
    if data.is_empty() {
        bail!("downloaded upgrade is empty");
    }
    Ok(data)
}

pub fn run(version: &str, prefix: Option<&str>) -> Result<()> {
    let current = std::env::current_exe().context("cannot locate current executable")?;
    let url = download_url(version, prefix);
    eprintln!("[i] 当前版本: {}", env!("CARGO_PKG_VERSION"));
    eprintln!("[i] 正在下载: {url}");
    let data = download(&url)?;
    let new_path = PathBuf::from(format!("{}.new", current.display()));
    fs::write(&new_path, &data)
        .with_context(|| format!("failed to write {}", new_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&new_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&new_path, permissions)?;
        fs::rename(&new_path, &current)
            .with_context(|| format!("failed to replace {}", current.display()))?;
        eprintln!("[+] 已升级到 {version}: {}", current.display());
    }

    #[cfg(windows)]
    {
        // A running .exe cannot be overwritten on Windows. Let a detached
        // cmd.exe wait for this process to exit, swap the binary, then start it.
        let script = format!(
            "@echo off\r\ntimeout /t 2 /nobreak >nul\r\nmove /Y \"{}\" \"{}\" >nul\r\nstart \"\" \"{}\"\r\n",
            new_path.display(),
            current.display(),
            current.display()
        );
        Command::new("cmd")
            .args(["/C", &script])
            .spawn()
            .context("failed to launch Windows upgrade helper")?;
        eprintln!("[+] 升级包已下载，程序退出后会自动替换并重启。");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_latest_and_tagged_urls() {
        let latest = download_url("latest", None);
        assert!(latest.contains("/releases/latest/download/"));
        assert!(latest.ends_with(asset_name()));
        let tagged = download_url("0.12.1", Some("https://ghfast.top/"));
        assert!(tagged.starts_with("https://ghfast.top/https://github.com/"));
        assert!(tagged.contains("/releases/download/v0.12.1/"));
        assert!(tagged.ends_with(asset_name()));
    }
}
