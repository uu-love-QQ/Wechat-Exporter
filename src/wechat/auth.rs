//! WeChat session material capture.
//!
//! The private-file CDN (`novac2c.cdn.weixin.qq.com/c2c/download`) rejects
//! anonymous requests: a real `authkey` and the live login cookies are required.
//! Alpha obtains this material from the running client; we read the same values
//! straight out of the `Weixin.exe` private heap without injecting anything.

use std::collections::BTreeMap;

/// Login material recovered from the running WeChat client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WechatSession {
    pub wxuin: String,
    pub wxsid: String,
    pub skey: String,
    pub pass_ticket: String,
    pub data_ticket: String,
    /// Hex-encoded C2C download ticket accepted by novac2c.
    pub auth_key: String,
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
        }
    }
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

    // `authkey` is hex-encoded ASN.1 DER; keep a syntactically valid candidate.
    for start in find_all(buffer, b"authkey=") {
        if let Some(value) = value_after(buffer, start) {
            let hex_like = value.len() >= 16
                && value.len() % 2 == 0
                && value.bytes().all(|b| b.is_ascii_hexdigit());
            if hex_like {
                if value.len() > session.auth_key.len() {
                    session.auth_key = value;
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
        (&mut base.auth_key, &extra.auth_key),
    ] {
        if slot.is_empty() && !value.is_empty() {
            *slot = value.clone();
        }
    }
}

/// Append `authkey` to a C2C download URL when the session carries one.
pub fn with_auth_key(url: &str, auth_key: &str) -> String {
    if auth_key.is_empty() || url.contains("authkey=") || !url.contains("novac2c") {
        return url.to_string();
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}authkey={auth_key}")
}

#[cfg(target_os = "windows")]
pub fn scan_from_process() -> Option<WechatSession> {
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

    let mut session = WechatSession::default();
    const CHUNK: usize = 4 * 1024 * 1024;
    const OVERLAP: usize = 96;

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

        let mut buffer = vec![0u8; CHUNK + OVERLAP];
        for pid in pids {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                continue;
            }
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
                let readable = mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE;
                if readable {
                    let mut offset = 0usize;
                    while offset < region {
                        let want = CHUNK.min(region - offset);
                        let mut read = 0usize;
                        if ReadProcessMemory(
                            handle,
                            (mbi.BaseAddress as usize + offset) as *const c_void,
                            buffer.as_mut_ptr() as *mut c_void,
                            want,
                            &mut read,
                        ) != 0
                            && read >= 8
                        {
                            let end = (read + OVERLAP).min(buffer.len());
                            scan_buffer_for_cookies(&buffer[..end], &mut session);
                        }
                        offset += want;
                        if want < CHUNK {
                            break;
                        }
                    }
                }
                address = (mbi.BaseAddress as usize).saturating_add(region);
            }
            CloseHandle(handle);
            if !session.is_empty() {
                break;
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
    fn ignores_non_hex_auth_key() {
        let mut session = WechatSession::default();
        scan_buffer_for_cookies(b"authkey=not-a-hex-value", &mut session);
        assert!(session.auth_key.is_empty());
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
