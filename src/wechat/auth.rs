//! WeChat login cookie capture for public CDN downloads.

use std::collections::BTreeMap;

/// Login cookies recovered from the running WeChat client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WechatSession {
    pub wxuin: String,
    pub wxsid: String,
    pub skey: String,
    pub pass_ticket: String,
    pub data_ticket: String,
}

impl WechatSession {
    pub fn is_empty(&self) -> bool {
        self.cookie_header().is_empty()
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

/// Extract the value that follows `needle` in `buffer`, stopping at a delimiter.
fn value_after(buffer: &[u8], mut i: usize) -> Option<String> {
    let mut out = String::new();
    while i < buffer.len() {
        let byte = buffer[i];
        let allowed = byte.is_ascii_alphanumeric()
            || matches!(byte, b'%' | b'_' | b'-' | b'.' | b'+' | b'/' | b'=' | b'@' | b':');
        if !allowed {
            break;
        }
        out.push(byte as char);
        i += 1;
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

/// Scan one memory buffer for `name=value` cookie occurrences.
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

    // WeChat stores some session strings as UTF-16LE.
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
}

/// Merge material captured elsewhere into this session.
pub fn merge_session(base: &mut WechatSession, extra: &WechatSession) {
    for (slot, value) in [
        (&mut base.wxuin, &extra.wxuin),
        (&mut base.wxsid, &extra.wxsid),
        (&mut base.skey, &extra.skey),
        (&mut base.pass_ticket, &extra.pass_ticket),
        (&mut base.data_ticket, &extra.data_ticket),
    ] {
        if slot.is_empty() && !value.is_empty() {
            *slot = value.clone();
        }
    }
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
}

#[cfg(target_os = "windows")]
pub fn scan_from_process() -> Option<WechatSession> {
    scan_from_process_inner(std::time::Duration::from_secs(10))
}

#[cfg(target_os = "windows")]
fn scan_from_process_inner(budget: std::time::Duration) -> Option<WechatSession> {
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
        !session.skey.is_empty() && !session.pass_ticket.is_empty()
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
                            "[!] 内存扫描达到 {} 秒上限，使用已捕获的 Cookie",
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
                        if last_report.elapsed() >= std::time::Duration::from_secs(1) {
                            eprintln!("[i] 已扫描微信内存 {} MB", scanned / (1024 * 1024));
                            last_report = std::time::Instant::now();
                        }
                        if session_complete(&session) {
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
                break 'processes;
            }
        }
    }

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
    fn extracts_cookies_from_memory_buffer() {
        let mut buffer = b"junk;wxuin=987654321;skey=@crypt_deadbeef;".to_vec();
        buffer.extend_from_slice(b"\0\0wxsid=QwErTy==;pass_ticket=abc%2Fdef;\0");
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(&buffer, &mut session);
        assert_eq!(session.wxuin, "987654321");
        assert_eq!(session.skey, "@crypt_deadbeef");
        assert_eq!(session.wxsid, "QwErTy==");
        assert_eq!(session.pass_ticket, "abc%2Fdef");
    }

    #[test]
    fn extracts_cookies_from_utf16_buffer() {
        let mut buffer = Vec::new();
        for ch in "wxuin=123456789;pass_ticket=abc%2Fdef".encode_utf16() {
            buffer.extend_from_slice(&ch.to_le_bytes());
        }
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(&buffer, &mut session);
        assert_eq!(session.wxuin, "123456789");
        assert_eq!(session.pass_ticket, "abc%2Fdef");
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
            pass_ticket: "PT".into(),
            ..Default::default()
        };
        merge_session(&mut base, &extra);
        assert_eq!(base.wxuin, "1");
        assert_eq!(base.skey, "@k");
        assert_eq!(base.pass_ticket, "PT");
    }
}
