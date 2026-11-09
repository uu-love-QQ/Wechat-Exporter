use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::time::Instant;
use std::path::PathBuf;
use std::time::Duration;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
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

fn resolve_latest(prefix: Option<&str>) -> Option<String> {
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut urls = vec![api.clone()];
    if let Some(prefix) = prefix {
        let prefix = prefix.trim_end_matches('/');
        if !prefix.is_empty() {
            urls.push(format!("{prefix}/{api}"));
        }
    }
    for url in urls {
        let Ok(response) = ureq::get(&url)
            .set("User-Agent", "wechat-exporter-updater")
            .set("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .call()
        else {
            continue;
        };
        let mut body = String::new();
        if response.into_reader().read_to_string(&mut body).is_err() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        if let Some(tag) = value.get("tag_name").and_then(|tag| tag.as_str()) {
            if !tag.trim().is_empty() {
                return Some(tag.trim().to_string());
            }
        }
    }
    None
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn download(url: &str, target: &std::path::Path) -> Result<()> {
    let response = ureq::get(url)
        .set("User-Agent", "wechat-exporter-updater")
        .timeout(Duration::from_secs(600))
        .call()
        .with_context(|| format!("download failed: {url}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let interactive = std::io::stderr().is_terminal();
    let mut reader = response.into_reader();
    let mut file = fs::File::create(target)
        .with_context(|| format!("failed to create {}", target.display()))?;
    let mut buffer = [0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now() - Duration::from_secs(10);
    loop {
        let read = reader.read(&mut buffer).context("failed to read upgrade download")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .with_context(|| format!("failed to write {}", target.display()))?;
        done += read as u64;
        if interactive && last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            let width = 24usize;
            let percent = if total > 0 { (done * 100 / total).min(100) } else { 0 };
            let filled = if total > 0 { width * percent as usize / 100 } else { 0 };
            let bar = format!("{}{}", "#".repeat(filled), "-".repeat(width - filled));
            if total > 0 {
                eprint!(
                    "\r下载 [{bar}] {percent:3}% · {}/{}",
                    format_bytes(done),
                    format_bytes(total)
                );
            } else {
                eprint!("\r下载 {}{}", format_bytes(done), "...");
            }
            let _ = std::io::stderr().flush();
        }
    }
    file.flush().ok();
    if interactive {
        eprintln!();
    }
    if done == 0 {
        bail!("downloaded upgrade is empty");
    }
    Ok(())
}

pub fn run(version: &str, prefix: Option<&str>) -> Result<()> {
    let current = std::env::current_exe().context("cannot locate current executable")?;
    let requested = if version == "latest" {
        resolve_latest(prefix).unwrap_or_else(|| "latest".to_string())
    } else {
        version.to_string()
    };
    let url = download_url(&requested, prefix);
    eprintln!("[i] 当前版本: {}", env!("CARGO_PKG_VERSION"));
    if requested != "latest" {
        eprintln!("[i] 目标版本: {requested}");
    }
    eprintln!("[i] 正在下载: {url}");
    let new_path = PathBuf::from(format!("{}.new", current.display()));
    download(&url, &new_path)?;

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
        let script_path = current.with_extension("upgrade.cmd");
        let log_path = current.with_extension("upgrade.log");
        let script = format!(
            "@echo off\r\nsetlocal\r\nset \"NEW={}\"\r\nset \"PUB={}\"\r\nset \"LOG={}\"\r\necho upgrade %date% %time%>>\"%LOG%\"\r\nfor /l %%i in (1,1,30) do (\r\n  ping 127.0.0.1 -n 2 >nul\r\n  move /Y \"%NEW%\" \"%PUB%\" >nul 2>&1\r\n  if not exist \"%NEW%\" goto done\r\n)\r\necho replace_failed>>\"%LOG%\"\r\ngoto end\r\n:done\r\necho replaced>>\"%LOG%\"\r\nstart \"\" \"%PUB%\"\r\n:end\r\ndel \"%~f0\" >nul 2>&1\r\n",
            new_path.display(),
            current.display(),
            log_path.display()
        );
        fs::write(&script_path, script)
            .with_context(|| format!("failed to write upgrade helper {}", script_path.display()))?;
        Command::new("cmd")
            .args(["/C", &script_path.to_string_lossy()])
            .creation_flags(0x08000000)
            .spawn()
            .context("failed to launch Windows upgrade helper")?;
        eprintln!("[+] 升级包已下载到 {}", new_path.display());
        eprintln!("[+] Windows 将等待退出后替换并重启；如未自动重启，请查看 {}", log_path.display());
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
