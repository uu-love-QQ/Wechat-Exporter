//! WeChat session material capture.
//!
//! The private-file CDN (`novac2c.cdn.weixin.qq.com/c2c/download`) rejects
//! anonymous requests: a real `authkey` and the live login cookies are required.
//! Alpha obtains this material from the running client; we read the same values
//! straight out of the `Weixin.exe` private heap without injecting anything.

use base64::Engine as _;
use std::collections::BTreeMap;

/// Login material recovered from the running WeChat client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WechatSession {
    pub wxuin: String,
    pub wxsid: String,
    pub skey: String,
    pub pass_ticket: String,
    pub data_ticket: String,
    /// Best hex-encoded C2C download ticket accepted by novac2c.
    pub auth_key: String,
    /// Every `authkey`-shaped value recovered from memory, best first.
    pub auth_key_candidates: Vec<String>,
}

impl WechatSession {
    pub fn is_empty(&self) -> bool {
        self.cookie_header().is_empty() && self.auth_key.is_empty()
    }

    /// `Cookie:` header value in WeChat's own ordering.
    pub fn cookie_header(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (name, value) in [
            ("wxuin", &self.wxuin),
            ("wxsid", &self.wxsid),
            ("skey", &self.skey),
            ("pass_ticket", &self.pass_ticket),
            ("data_ticket", &self.data_ticket),
        ] {
            if !value.is_empty() {
                parts.push(format!("{name}={value}"));
            }
        }
        parts.join("; ")
    }

    /// Parse a `k=v; k=v` cookie string, ignoring unrelated keys.
    pub fn from_cookie_header(raw: &str) -> Self {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        for part in raw.split(';') {
            let part = part.trim();
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            pairs.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
        let take = |key: &str| pairs.get(key).cloned().unwrap_or_default();
        Self {
            wxuin: take("wxuin"),
            wxsid: take("wxsid"),
            skey: take("skey"),
            pass_ticket: take("pass_ticket"),
            data_ticket: take("data_ticket"),
            auth_key: String::new(),
            auth_key_candidates: Vec::new(),
        }
    }
}

/// Encode an ASCII needle as UTF-16LE bytes.
fn utf16le_needle(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 2);
    for unit in value.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn utf16le_value_after(buffer: &[u8], mut i: usize) -> Option<String> {
    while i + 1 < buffer.len() {
        let unit = u16::from_le_bytes([buffer[i], buffer[i + 1]]);
        if !matches!(unit as u8, b'=' | b':' | b'"' | b'\'' | b' ' | b'\t' | 0) || unit > 0x7f {
            break;
        }
        i += 2;
    }
    let mut out = String::new();
    while i + 1 < buffer.len() {
        let unit = u16::from_le_bytes([buffer[i], buffer[i + 1]]);
        if unit > 0x7f {
            break;
        }
        let byte = unit as u8;
        let allowed = byte.is_ascii_alphanumeric()
            || matches!(byte, b'%' | b'_' | b'-' | b'.' | b'+' | b'/' | b'=' | b'@' | b':');
        if !allowed {
            break;
        }
        out.push(byte as char);
        i += 2;
        if out.len() > 2048 {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// True when the hex string decodes to one complete ASN.1 DER element.
pub fn asn1_der_valid(value: &str) -> bool {
    let Ok(bytes) = hex::decode(value) else {
        return false;
    };
    der_bytes_valid(&bytes)
}

/// Locate the first UUID text embedded in a DER/ticket buffer.
pub fn find_uuid(bytes: &[u8]) -> Option<String> {
    for window in bytes.windows(36) {
        if window[8] == b'-'
            && window[13] == b'-'
            && window[18] == b'-'
            && window[23] == b'-'
            && window.iter().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    *byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        {
            return Some(String::from_utf8_lossy(window).to_string());
        }
    }
    None
}

fn der_bytes_valid(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || bytes[0] != 0x30 {
        return false;
    }
    matches!(der_end(bytes, 0, 0), Some(end) if end == bytes.len())
}

fn der_end(bytes: &[u8], mut offset: usize, depth: u8) -> Option<usize> {
    if depth > 8 || offset + 2 > bytes.len() {
        return None;
    }
    let tag = bytes[offset];
    offset += 1;
    let first = bytes[offset];
    offset += 1;
    let length = if first & 0x80 == 0 {
        first as usize
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 || offset + count > bytes.len() {
            return None;
        }
        let mut value = 0usize;
        for byte in &bytes[offset..offset + count] {
            value = value.checked_mul(256)?.checked_add(*byte as usize)?;
        }
        offset += count;
        value
    };
    let end = offset.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    if tag & 0x20 != 0 {
        let mut cursor = offset;
        while cursor < end {
            cursor = der_end(bytes, cursor, depth + 1)?;
        }
        if cursor != end {
            return None;
        }
    }
    Some(end)
}

fn der_first_integer(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 3 || bytes[0] != 0x30 || bytes[1] & 0x80 != 0 {
        return None;
    }
    let mut offset = 2usize;
    if offset >= bytes.len() || bytes[offset] != 0x02 {
        return None;
    }
    offset += 1;
    let length = *bytes.get(offset)? as usize;
    offset += 1;
    if length == 0 || length > 8 || offset + length > bytes.len() {
        return None;
    }
    let mut value: i64 = 0;
    for byte in &bytes[offset..offset + length] {
        value = value.checked_mul(256)?.checked_add(*byte as i64)?;
    }
    Some(value)
}

fn push_auth_candidate(session: &mut WechatSession, value: String) {
    let value = value.trim().to_string();
    if value.len() < 16 || value.len() > 65_536 || value.len() % 2 != 0 {
        return;
    }
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return;
    }
    // Top-level version 0 envelopes are `cdnattachurl` file tokens, which the
    // CDN rejects as authkeys ("authkey param is empty"); skip them.
    if let Ok(bytes) = hex::decode(&value) {
        if der_first_integer(&bytes) == Some(0) {
            return;
        }
    }
    if !session.auth_key_candidates.iter().any(|item| item == &value)
        && session.auth_key_candidates.len() < 32
    {
        session.auth_key_candidates.push(value);
    }
}

/// Pick the ASN.1-shaped ticket first (that is what novac2c verifies), then the
/// longest remaining candidate.
pub fn finalize_auth_candidates(session: &mut WechatSession) {
    session
        .auth_key_candidates
        .retain(|value| value.len() >= 16 && value.len() % 2 == 0);
    session.auth_key_candidates.sort_by(|a, b| {
        let a_valid = asn1_der_valid(a);
        let b_valid = asn1_der_valid(b);
        b_valid
            .cmp(&a_valid)
            .then_with(|| b.len().cmp(&a.len()))
    });
    session.auth_key_candidates.dedup();
    if !session.auth_key_candidates.is_empty() {
        session.auth_key = session.auth_key_candidates[0].clone();
    }
}

/// Extract a hex token whose key is followed by `=`, `:`, quotes or spaces.
fn hex_after_key(buffer: &[u8], start: usize) -> Option<String> {
    let mut i = start;
    while i < buffer.len() && matches!(buffer[i], b'=' | b':' | b'"' | b'\'' | b' ' | b'\t') {
        i += 1;
    }
    let mut out = String::new();
    while i < buffer.len() && buffer[i].is_ascii_hexdigit() {
        out.push(buffer[i] as char);
        i += 1;
        if out.len() >= 4096 {
            break;
        }
    }
    Some(out)
}

/// Extract the value that follows `needle` in `buffer`, stopping at a delimiter.
fn value_after(buffer: &[u8], start: usize) -> Option<String> {
    let mut out = Vec::new();
    for &byte in buffer.get(start..)? {
        let allowed = byte.is_ascii_alphanumeric()
            || matches!(byte, b'%' | b'_' | b'-' | b'.' | b'+' | b'/' | b'=' | b'@' | b':');
        if !allowed {
            break;
        }
        out.push(byte);
        if out.len() > 2048 {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        String::from_utf8(out).ok()
    }
}

fn is_base64_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
}

/// The CDN expects `authkey=<base64(DER)>`; WeChat also keeps that form in
/// memory. Decode candidate runs and keep those that parse as complete DER.
fn scan_base64_der_runs(buffer: &[u8], session: &mut WechatSession) {
    let mut i = 0usize;
    while i < buffer.len() {
        if is_base64_byte(buffer[i]) {
            let start = i;
            while i < buffer.len() && is_base64_byte(buffer[i]) && i - start < 65_536 {
                i += 1;
            }
            let run = &buffer[start..i];
            if run.len() >= 48 && run.len() % 4 == 0 {
                if let Ok(text) = std::str::from_utf8(run) {
                    for decoded in [
                        base64::engine::general_purpose::STANDARD.decode(text),
                        base64::engine::general_purpose::STANDARD_NO_PAD.decode(text),
                        base64::engine::general_purpose::URL_SAFE.decode(text),
                        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text),
                    ] {
                        if let Ok(bytes) = decoded {
                            if der_bytes_valid(&bytes) {
                                push_auth_candidate(session, hex::encode(&bytes));
                            }
                        }
                    }
                }
            }
        } else {
            i += 1;
        }
    }
}

fn contains_any(buffer: &[u8], needles: &[&[u8]]) -> bool {
    needles
        .iter()
        .any(|needle| buffer.windows(needle.len()).any(|window| window == *needle))
}

fn scan_asn1_hex_runs(buffer: &[u8], session: &mut WechatSession) {
    let mut i = 0usize;
    while i < buffer.len() {
        if buffer[i].is_ascii_hexdigit() {
            let start = i;
            while i < buffer.len() && buffer[i].is_ascii_hexdigit() && i - start < 65_536 {
                i += 1;
            }
            let run = &buffer[start..i];
            if run.len() >= 64 && run.len() % 2 == 0 {
                if let Ok(text) = std::str::from_utf8(run) {
                    if asn1_der_valid(text) {
                        push_auth_candidate(session, text.to_string());
                    }
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Scan one memory buffer for `name=value` occurrences and keep the longest value.
pub fn scan_buffer_for_cookies(buffer: &[u8], session: &mut WechatSession) {
    fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
        let mut hits = Vec::new();
        let mut from = 0usize;
        while from + needle.len() <= haystack.len() {
            match haystack[from..].windows(needle.len()).position(|w| w == needle) {
                Some(offset) => {
                    let at = from + offset;
                    hits.push(at + needle.len());
                    from = at + needle.len();
                }
                None => break,
            }
        }
        hits
    }

    let assign = |slot: &mut String, candidate: String| {
        if candidate.len() > slot.len() && candidate.len() <= 2048 {
            *slot = candidate;
        }
    };

    for (needle, slot) in [
        (&b"wxuin="[..], &mut session.wxuin),
        (&b"wxsid="[..], &mut session.wxsid),
        (&b"skey="[..], &mut session.skey),
        (&b"pass_ticket="[..], &mut session.pass_ticket),
        (&b"data_ticket="[..], &mut session.data_ticket),
    ] {
        for start in find_all(buffer, needle) {
            if let Some(value) = value_after(buffer, start) {
                assign(slot, value);
            }
        }
    }

    // `authkey` is hex-encoded ASN.1 DER. WeChat stores it as `authkey=HEX`,
    // `"authkey":"HEX"`, or inside a protobuf/JSON blob, so accept any of those.
    for start in find_all(buffer, b"authkey") {
        if let Some(value) = hex_after_key(buffer, start) {
            let hex_like = value.len() >= 16
                && value.len() % 2 == 0
                && value.bytes().all(|b| b.is_ascii_hexdigit());
            if hex_like {
                push_auth_candidate(session, value);
            }
        }
    }

    // WeChat stores some session strings as UTF-16LE. Scan those too.
    for (name, slot) in [
        ("wxuin", &mut session.wxuin),
        ("wxsid", &mut session.wxsid),
        ("skey", &mut session.skey),
        ("pass_ticket", &mut session.pass_ticket),
        ("data_ticket", &mut session.data_ticket),
    ] {
        let needle = utf16le_needle(name);
        for start in find_all(buffer, &needle) {
            if let Some(value) = utf16le_value_after(buffer, start) {
                assign(slot, value);
            }
        }
    }
    for start in find_all(buffer, &utf16le_needle("authkey")) {
        if let Some(value) = utf16le_value_after(buffer, start) {
            let hex_like = value.len() >= 16
                && value.len() % 2 == 0
                && value.bytes().all(|b| b.is_ascii_hexdigit());
            if hex_like {
                push_auth_candidate(session, value);
            }
        }
    }

    // The CDN route ticket is a hex-encoded ASN.1 DER blob. Scanning every byte
    // of WeChat's multi-gigabyte heap is far too slow; only inspect chunks that
    // expose relevant labels.
    if contains_any(buffer, &[b"authkey", b"AuthKey", b"cdninfo", b"frontip", b"zoneip"]) {
        scan_asn1_hex_runs(buffer, session);
        scan_base64_der_runs(buffer, session);
    }

    // Cookies may also be stored as `"skey":"..."` inside JSON.
    for (name, slot) in [
        ("skey", &mut session.skey),
        ("wxsid", &mut session.wxsid),
        ("wxuin", &mut session.wxuin),
        ("pass_ticket", &mut session.pass_ticket),
        ("data_ticket", &mut session.data_ticket),
    ] {
        let quoted = format!("\"{name}\"");
        for start in find_all(buffer, quoted.as_bytes()) {
            let mut i = start;
            while i < buffer.len() && matches!(buffer[i], b':' | b' ' | b'\t') {
                i += 1;
            }
            if i < buffer.len() && buffer[i] == b'"' {
                if let Some(value) = value_after(buffer, i + 1) {
                    if value.len() > slot.len() {
                        *slot = value;
                    }
                }
            }
        }
    }

    finalize_auth_candidates(session);
}

/// Merge material captured elsewhere into this session.
pub fn merge_session(base: &mut WechatSession, extra: &WechatSession) {
    for (slot, value) in [
        (&mut base.wxuin, &extra.wxuin),
        (&mut base.wxsid, &extra.wxsid),
        (&mut base.skey, &extra.skey),
        (&mut base.pass_ticket, &extra.pass_ticket),
        (&mut base.data_ticket, &extra.data_ticket),
        (&mut base.auth_key, &extra.auth_key),
    ] {
        if slot.is_empty() && !value.is_empty() {
            *slot = value.clone();
        }
    }
    for candidate in &extra.auth_key_candidates {
        push_auth_candidate(base, candidate.clone());
    }
    finalize_auth_candidates(base);
}

/// Overwrite non-empty fields with explicitly supplied material (CLI wins).
pub fn override_session(base: &mut WechatSession, extra: &WechatSession) {
    for (slot, value) in [
        (&mut base.wxuin, &extra.wxuin),
        (&mut base.wxsid, &extra.wxsid),
        (&mut base.skey, &extra.skey),
        (&mut base.pass_ticket, &extra.pass_ticket),
        (&mut base.data_ticket, &extra.data_ticket),
    ] {
        if !value.is_empty() {
            *slot = value.clone();
        }
    }
    for candidate in &extra.auth_key_candidates {
        push_auth_candidate(base, candidate.clone());
    }
    if !extra.auth_key.is_empty() {
        base.auth_key = extra.auth_key.clone();
        push_auth_candidate(base, extra.auth_key.clone());
        finalize_auth_candidates(base);
        base.auth_key = extra.auth_key.clone();
    }
}

/// Append `authkey` to a C2C download URL when the session carries one.
pub fn with_auth_key(url: &str, wire_auth_key: &str) -> String {
    if wire_auth_key.is_empty() || url.contains("authkey=") || !url.contains("novac2c") {
        return url.to_string();
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    // The server base64-decodes this value; `+`, `/` and `=` must be escaped.
    let encoded: String = wire_auth_key
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("{url}{separator}authkey={encoded}")
}

#[cfg(target_os = "windows")]
pub fn scan_from_process() -> Option<WechatSession> {
    scan_from_process_inner(std::time::Duration::from_secs(10), true)
}

/// Collect every authkey candidate (used by `--dump-session`), even after a
/// complete-looking session was found.
#[cfg(target_os = "windows")]
pub fn scan_from_process_collect() -> Option<WechatSession> {
    scan_from_process_collect_for(10)
}

#[cfg(target_os = "windows")]
pub fn scan_from_process_collect_for(seconds: u64) -> Option<WechatSession> {
    scan_from_process_inner(std::time::Duration::from_secs(seconds.max(2)), false)
}

#[cfg(target_os = "windows")]
fn scan_from_process_inner(
    budget: std::time::Duration,
    stop_when_complete: bool,
) -> Option<WechatSession> {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type DWORD = u32;
    type BOOL = i32;
    #[allow(non_camel_case_types)]
    type SIZE_T = usize;

    const TH32CS_SNAPPROCESS: DWORD = 0x00000002;
    const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
    const PROCESS_VM_READ: DWORD = 0x0010;
    const MEM_COMMIT: DWORD = 0x1000;
    const MEM_PRIVATE: DWORD = 0x20000;
    const MEM_IMAGE: DWORD = 0x1000000;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32W {
        dwSize: DWORD,
        cntUsage: DWORD,
        th32ProcessID: DWORD,
        th32DefaultHeapID: usize,
        th32ModuleID: DWORD,
        cntThreads: DWORD,
        th32ParentProcessID: DWORD,
        pcPriClassBase: i32,
        dwFlags: DWORD,
        szExeFile: [u16; 260],
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct MEMORY_BASIC_INFORMATION {
        BaseAddress: *mut c_void,
        AllocationBase: *mut c_void,
        AllocationProtect: DWORD,
        PartitionId: u16,
        RegionSize: SIZE_T,
        State: DWORD,
        Protect: DWORD,
        Type: DWORD,
    }

    extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: DWORD, th32ProcessID: DWORD) -> HANDLE;
        fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
        fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
        fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
        fn VirtualQueryEx(
            hProcess: HANDLE,
            lpAddress: *const c_void,
            lpBuffer: *mut MEMORY_BASIC_INFORMATION,
            dwLength: SIZE_T,
        ) -> SIZE_T;
        fn ReadProcessMemory(
            hProcess: HANDLE,
            lpBaseAddress: *const c_void,
            lpBuffer: *mut c_void,
            nSize: SIZE_T,
            lpNumberOfBytesRead: *mut SIZE_T,
        ) -> BOOL;
        fn CloseHandle(hObject: HANDLE) -> BOOL;
    }

    fn session_complete(session: &WechatSession) -> bool {
        !session.wxuin.is_empty()
            && !session.wxsid.is_empty()
            && !session.skey.is_empty()
            && !session.pass_ticket.is_empty()
            && !session.auth_key.is_empty()
    }

    let mut session = WechatSession::default();
    const CHUNK: usize = 4 * 1024 * 1024;
    const MAX_REGION: usize = 128 * 1024 * 1024;
    let started = std::time::Instant::now();
    let mut scanned: u64 = 0;
    let mut last_report = std::time::Instant::now();

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot == -1isize as HANDLE {
            return None;
        }
        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as DWORD;

        let mut pids = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile);
                let trimmed = name.trim_matches('\0');
                if trimmed.eq_ignore_ascii_case("Weixin.exe")
                    || trimmed.eq_ignore_ascii_case("WeChat.exe")
                {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        let mut buffer = vec![0u8; CHUNK];
        'processes: for pid in pids {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                continue;
            }
            // Collect readable private regions and scan the smallest ones first:
            // session material lives in heaps, while Chromium reserves are huge.
            let mut regions: Vec<(usize, usize)> = Vec::new();
            let mut address = 0usize;
            let mut mbi = std::mem::zeroed::<MEMORY_BASIC_INFORMATION>();
            let mbi_size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
            while address < 0x7FFF_FFFF_0000 {
                if VirtualQueryEx(handle, address as *const c_void, &mut mbi, mbi_size) == 0 {
                    break;
                }
                let region = mbi.RegionSize;
                if region == 0 {
                    break;
                }
                let readable = mbi.State == MEM_COMMIT
                    && mbi.Type == MEM_PRIVATE
                    && region <= MAX_REGION
                    && (mbi.Protect & 0x01) == 0
                    && (mbi.Protect & 0x100) == 0;
                if readable {
                    regions.push((mbi.BaseAddress as usize, region));
                }
                address = (mbi.BaseAddress as usize).saturating_add(region);
            }
            regions.sort_by_key(|(_, size)| *size);

            let mut stop_all = false;
            for (base, region) in regions {
                let mut offset = 0usize;
                while offset < region {
                    if started.elapsed() >= budget {
                        eprintln!(
                            "[!] 内存扫描达到 {} 秒上限，使用已捕获的凭据",
                            budget.as_secs()
                        );
                        stop_all = true;
                        break;
                    }
                    let want = CHUNK.min(region - offset);
                    let mut read = 0usize;
                    if ReadProcessMemory(
                        handle,
                        (base + offset) as *const c_void,
                        buffer.as_mut_ptr() as *mut c_void,
                        want,
                        &mut read,
                    ) != 0
                        && read >= 8
                    {
                        scan_buffer_for_cookies(&buffer[..read], &mut session);
                        scanned += read as u64;
                        if last_report.elapsed() >= std::time::Duration::from_secs(2) {
                            eprintln!("[i] 已扫描微信内存 {} MB", scanned / (1024 * 1024));
                            last_report = std::time::Instant::now();
                        }
                        if stop_when_complete && session_complete(&session) {
                            stop_all = true;
                            break;
                        }
                    }
                    offset += want;
                    if want < CHUNK {
                        break;
                    }
                }
                if stop_all {
                    break;
                }
            }
            CloseHandle(handle);
            if stop_all {
                break;
            }
        }
    }

    finalize_auth_candidates(&mut session);
    if session.is_empty() {
        None
    } else {
        Some(session)
    }
}

#[cfg(not(target_os = "windows"))]
pub fn scan_from_process() -> Option<WechatSession> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn scan_from_process_collect() -> Option<WechatSession> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn scan_from_process_collect_for(_seconds: u64) -> Option<WechatSession> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_header() {
        let session = WechatSession::from_cookie_header(
            "wxuin=123456; wxsid=abcDEF==; skey=@crypt_1; pass_ticket=PT%2B1; other=x",
        );
        assert_eq!(session.wxuin, "123456");
        assert_eq!(session.wxsid, "abcDEF==");
        assert_eq!(session.skey, "@crypt_1");
        assert_eq!(session.pass_ticket, "PT%2B1");
        assert_eq!(
            session.cookie_header(),
            "wxuin=123456; wxsid=abcDEF==; skey=@crypt_1; pass_ticket=PT%2B1"
        );
    }

    #[test]
    fn extracts_material_from_memory_buffer() {
        let mut buffer = b"junk;wxuin=987654321;skey=@crypt_deadbeef;".to_vec();
        buffer.extend_from_slice(b"\0\0authkey=00112233445566778899aabbccddeeff\0");
        buffer.extend_from_slice(b"wxsid=QwErTy==;pass_ticket=abc%2Fdef;");
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(&buffer, &mut session);
        assert_eq!(session.wxuin, "987654321");
        assert_eq!(session.skey, "@crypt_deadbeef");
        assert_eq!(session.wxsid, "QwErTy==");
        assert_eq!(session.pass_ticket, "abc%2Fdef");
        assert_eq!(session.auth_key, "00112233445566778899aabbccddeeff");
    }


    #[test]
    fn extracts_material_from_utf16_buffer() {
        let mut buffer = Vec::new();
        for ch in "wxuin=123456789;".encode_utf16() {
            buffer.extend_from_slice(&ch.to_le_bytes());
        }
        for ch in "authkey=0011223344556677".encode_utf16() {
            buffer.extend_from_slice(&ch.to_le_bytes());
        }
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(&buffer, &mut session);
        assert_eq!(session.wxuin, "123456789");
        assert_eq!(session.auth_key, "0011223344556677");
    }

    #[test]
    fn ignores_non_hex_auth_key() {
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(b"authkey=not-a-hex-value", &mut session);
        assert!(session.auth_key.is_empty());
    }

    #[test]
    fn validates_asn1_der_auth_tickets() {
        // Inner token of a real c2c `cdnattachurl` (valid DER SEQUENCE).
        let der = "304902010002040500a0be02032e6a6f02041b5b076f02046aea90ba042466663062336363312d393131342d343338302d623063302d393937313136333463633430020405180005020100";
        assert!(asn1_der_valid(der));
        // Login AutoAuthKey protobuf captured from WeChat memory: not DER.
        let protobuf = "0A2B6F4E2D4D77415141414141423462304851773832436B314B456A705356544C327340696D2E7778776F726B10B18CE9EF07";
        assert!(!asn1_der_valid(protobuf));
        assert!(!asn1_der_valid("not-hex"));
        assert!(!asn1_der_valid("30030201ff"));
    }

    #[test]
    fn prefers_the_asn1_authkey_candidate() {
        let der = "306b020102046430620201000204dd837c7702030f424402045f030d0102046ae54ba7042464353363343866332d656133382d346235372d393861392d6661386239393335643731340203102800020303c3f004100906042891c60977b784b0eaefff9d9f0201010201000400";
        let protobuf = "0A2B6F4E2D4D77415141414141423462304851773832436B314B456A705356544C327340696D2E7778776F726B10B18CE9EF07";
        let buffer = format!("authkey={protobuf}\0junk\0{der}\0");
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(buffer.as_bytes(), &mut session);
        assert_eq!(session.auth_key, der);
        assert!(session.auth_key_candidates.iter().any(|item| item == protobuf));
        assert!(session.auth_key_candidates.iter().any(|item| item == der));
    }

    #[test]
    fn appends_auth_key_only_for_c2c_urls() {
        let url = "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_param=AA";
        assert_eq!(
            with_auth_key(url, "0011"),
            "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_param=AA&authkey=0011"
        );
        // Already signed, or unrelated host: leave untouched.
        assert_eq!(with_auth_key(url, ""), url);
        let signed = format!("{url}&authkey=ff");
        assert_eq!(with_auth_key(&signed, "0011"), signed);
        let other = "http://vweixinf.tc.qq.com/x?m=1";
        assert_eq!(with_auth_key(other, "0011"), other);
        assert_eq!(
            with_auth_key(url, "a+b/c="),
            "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_param=AA&authkey=a%2Bb%2Fc%3D"
        );
    }

    #[test]
    fn merges_missing_fields_only() {
        let mut base = WechatSession {
            wxuin: "1".into(),
            ..Default::default()
        };
        let extra = WechatSession {
            wxuin: "2".into(),
            skey: "@k".into(),
            ..Default::default()
        };
        merge_session(&mut base, &extra);
        assert_eq!(base.wxuin, "1");
        assert_eq!(base.skey, "@k");
    }
}

