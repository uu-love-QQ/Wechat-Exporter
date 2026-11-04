fn find_local_key_file() -> Option<String> {
    if let Ok(content) = fs::read_to_string("key.txt") {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            eprintln!("[i] 自动从当前目录下的 key.txt 文件加载数据库密钥");
            return Some(trimmed.to_string());
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let exe_key = parent.join("key.txt");
            if let Ok(content) = fs::read_to_string(&exe_key) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    eprintln!("[i] 自动从程序所在目录的 key.txt 文件加载数据库密钥: {}", exe_key.display());
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::fs;
#[allow(unused_imports)]
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "wechat.chat-export.v1";
pub const GENERATOR: &str = concat!("wechat-exporter/", env!("CARGO_PKG_VERSION"));

pub fn md5_hex(value: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn message_table_name(conversation_id: &str) -> String {
    format!("Msg_{}", md5_hex(conversation_id))
}

#[cfg(target_os = "windows")]
fn stdin_has_piped_data() -> bool {
    use std::ffi::c_void;

    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn GetFileType(hFile: *mut c_void) -> u32;
        fn PeekNamedPipe(
            hNamedPipe: *mut c_void,
            lpBuffer: *mut c_void,
            nBufferSize: u32,
            lpBytesRead: *mut u32,
            lpTotalBytesAvail: *mut u32,
            lpBytesLeftThisMessage: *mut u32,
        ) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6; // -10 as u32
    const FILE_TYPE_PIPE: u32 = 0x0003;

    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if handle.is_null() || handle == -1isize as *mut c_void {
            return false;
        }
        if GetFileType(handle) == FILE_TYPE_PIPE {
            let mut bytes_avail = 0u32;
            if PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut bytes_avail,
                std::ptr::null_mut(),
            ) != 0
            {
                return bytes_avail > 0;
            }
        }
        false
    }
}

#[cfg(not(target_os = "windows"))]
fn stdin_has_piped_data() -> bool {
    !std::io::stdin().is_terminal()
}

pub fn read_key(explicit: Option<&str>, file: Option<&Path>) -> Result<Option<[u8; 32]>> {
    let raw = if let Some(value) = explicit {
        value.trim().to_string()
    } else if let Some(path) = file {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read key file {}", path.display()))?
            .trim()
            .to_string()
    } else if let Some(content) = find_local_key_file() {
        content

    } else if stdin_has_piped_data() {
        let mut stdin = std::io::stdin();
        let mut buf = String::new();
        stdin.read_to_string(&mut buf).ok();
        buf.trim().to_string()
    } else {
        String::new()
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let cleaned = raw
        .strip_prefix("x'")
        .and_then(|v| v.strip_suffix('\''))
        .unwrap_or(&raw);
    if cleaned.len() != 64 || !cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("database key must be exactly 64 hexadecimal characters");
    }
    let bytes = hex::decode(cleaned).expect("validated hex");
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(Some(key))
}

pub fn clean_account_id(value: &str) -> String {
    let trimmed = value.trim();
    if let Some((head, tail)) = trimmed.rsplit_once('_') {
        if head.starts_with("wxid_") && tail.len() == 4 && tail.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return head.to_string();
        }
    }
    trimmed.to_string()
}

pub fn is_group_id(value: &str) -> bool {
    value.ends_with("@chatroom")
}

pub fn format_timestamp(seconds: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(seconds, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => seconds.to_string(),
    }
}

pub fn format_iso_timestamp(seconds: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(seconds, 0).single() {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        None => seconds.to_string(),
    }
}

pub fn sanitize_file_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ' | '(' | ')' | '[' | ']') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim().trim_matches('.').to_string();
    if out.is_empty() {
        "conversation".to_string()
    } else if out.chars().count() > 80 {
        out.chars().take(80).collect()
    } else {
        out
    }
}

#[derive(Serialize, Deserialize)]
struct CursorPayload {
    v: u32,
    k: String,
    t: i64,
    id: String,
    n: u64,
}

pub fn encode_cursor(kind: &str, time: i64, id: &str, ordinal: u64) -> String {
    let payload = CursorPayload {
        v: 1,
        k: kind.to_string(),
        t: time,
        id: id.to_string(),
        n: ordinal,
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("cursor serialization"))
}

pub fn decode_cursor(kind: &str, raw: &str) -> Result<(i64, String, u64)> {
    if raw.len() > 1024 {
        bail!("conversation cursor exceeds limit");
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .with_context(|| "conversation cursor must be canonical base64")?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).with_context(|| "conversation cursor must be canonical base64")?;
    if payload.v != 1 || payload.k != kind {
        bail!("expected conversation page identity");
    }
    Ok((payload.t, payload.id, payload.n))
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn escape_csv(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''").replace('\0', "")
}

pub fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn ensure_output_dir(path: &Path) -> Result<()> {
    if path.is_file() {
        bail!("output directory is an existing file, not a directory");
    }
    if !path.is_absolute() {
        bail!("output directory must be an absolute path");
    }
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            bail!("output directory parent does not exist");
        }
    }
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    Ok(())
}

pub fn copy_media_file(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest)?;
    Ok(())
}
