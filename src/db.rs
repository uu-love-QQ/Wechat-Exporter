use anyhow::{bail, Context, Result};
use hmac::{Hmac, KeyInit};
use rusqlite::{Connection, OpenFlags};
use sha2::Sha512;
use std::path::Path;

type HmacSha512 = Hmac<Sha512>;

pub struct Db {
    conn: Connection,
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(path, flags)
        .with_context(|| format!("failed to open database {}", path.display()))
}

fn check_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    conn.execute_batch("PRAGMA query_only = ON;")?;
    Ok(())
}

fn open_checked(path: &Path, f: impl Fn(&Connection) -> Result<()>) -> Result<Connection> {
    let conn = match open_connection(path, true) {
        Ok(conn) => conn,
        Err(_) => {
            // A live WAL database may need the -shm file. Opening read-write
            // lets SQLite recover it; query_only prevents accidental writes.
            open_connection(path, false)?
        }
    };
    f(&conn)?;
    Ok(conn)
}

fn is_plain_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    file.read_exact(&mut header).is_ok() && &header == b"SQLite format 3\0"
}

pub fn derive_wechat_db_key(passphrase_bytes: &[u8], salt: &[u8; 16]) -> [u8; 32] {
    let mut derived = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha512>(passphrase_bytes, salt, 256_000, &mut derived);
    derived
}

pub fn verify_db_page1(enc_key: &[u8; 32], page1: &[u8]) -> bool {
    if page1.len() < 4096 {
        return false;
    }
    let salt = &page1[..16];
    let mut mac_salt = [0u8; 16];
    for (i, b) in salt.iter().enumerate() {
        mac_salt[i] = b ^ 0x3a;
    }
    let mut mac_key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha512>(enc_key, &mac_salt, 2, &mut mac_key);

    let hmac_data = &page1[16..4032];
    let stored_hmac = &page1[4032..4096];

    use hmac::Mac;
    if let Ok(mut mac) = HmacSha512::new_from_slice(&mac_key) {
        mac.update(hmac_data);
        mac.update(&1u32.to_le_bytes()); // pgno = 1
        mac.verify_slice(stored_hmac).is_ok()
    } else {
        false
    }
}



#[cfg(target_os = "windows")]
fn scan_weixin_dll_for_internal_keys(dll_bytes: &[u8]) -> Vec<[u8; 32]> {
    let mut candidates = Vec::new();
    if dll_bytes.len() < 0x40 || &dll_bytes[..2] != b"MZ" {
        return candidates;
    }
    let e_lfanew = u32::from_le_bytes(dll_bytes[0x3c..0x40].try_into().unwrap_or([0; 4])) as usize;
    if e_lfanew + 0x18 > dll_bytes.len() || &dll_bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return candidates;
    }
    let num_sections = u16::from_le_bytes(dll_bytes[e_lfanew + 6..e_lfanew + 8].try_into().unwrap_or([0; 2])) as usize;
    let opt_hdr_size = u16::from_le_bytes(dll_bytes[e_lfanew + 0x14..e_lfanew + 0x16].try_into().unwrap_or([0; 2])) as usize;
    let sec_table_offset = e_lfanew + 0x18 + opt_hdr_size;

    for i in 0..num_sections {
        let sec_offset = sec_table_offset + i * 40;
        if sec_offset + 40 > dll_bytes.len() { break; }
        let chars = u32::from_le_bytes(dll_bytes[sec_offset + 36..sec_offset + 40].try_into().unwrap_or([0; 4]));
        // IMAGE_SCN_MEM_EXECUTE (0x20000000)
        if chars & 0x2000_0000 != 0 {
            let virt_size = u32::from_le_bytes(dll_bytes[sec_offset + 8..sec_offset + 12].try_into().unwrap_or([0; 4])) as usize;
            let raw_data_ptr = u32::from_le_bytes(dll_bytes[sec_offset + 20..sec_offset + 24].try_into().unwrap_or([0; 4])) as usize;
            let raw_data_size = u32::from_le_bytes(dll_bytes[sec_offset + 16..sec_offset + 20].try_into().unwrap_or([0; 4])) as usize;
            let sec_size = virt_size.min(raw_data_size);
            if raw_data_ptr + sec_size <= dll_bytes.len() {
                let sec_bytes = &dll_bytes[raw_data_ptr..raw_data_ptr + sec_size];
                let mut idx = 0;
                while idx + 45 <= sec_bytes.len() {
                    if sec_bytes[idx] == 0x48 && sec_bytes[idx + 1] == 0xba {
                        for o2 in 10..=24 {
                            let idx2 = idx + o2;
                            if idx2 + 30 <= sec_bytes.len() && sec_bytes[idx2] == 0x48 && sec_bytes[idx2 + 1] == 0xba {
                                for o3 in 10..=24 {
                                    let idx3 = idx2 + o3;
                                    if idx3 + 20 <= sec_bytes.len() && sec_bytes[idx3] == 0x48 && sec_bytes[idx3 + 1] == 0xba {
                                        for o4 in 10..=24 {
                                            let idx4 = idx3 + o4;
                                            if idx4 + 10 <= sec_bytes.len() && sec_bytes[idx4] == 0x48 && sec_bytes[idx4 + 1] == 0xba {
                                                let mut cand = [0u8; 32];
                                                cand[0..8].copy_from_slice(&sec_bytes[idx + 2..idx + 10]);
                                                cand[8..16].copy_from_slice(&sec_bytes[idx2 + 2..idx2 + 10]);
                                                cand[16..24].copy_from_slice(&sec_bytes[idx3 + 2..idx3 + 10]);
                                                cand[24..32].copy_from_slice(&sec_bytes[idx4 + 2..idx4 + 10]);
                                                candidates.push(cand);
                                                break;
                                            }
                                        }
                                        break;
                                    }
                                }
                                break;
                            }
                        }
                    }
                    idx += 1;
                }
            }
        }
    }
    candidates.dedup();
    candidates
}

#[cfg(target_os = "windows")]
pub fn auto_scan_wechat_key_from_process(page1: Option<&[u8]>) -> Option<[u8; 32]> {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type DWORD = u32;
    type BOOL = i32;
    #[allow(non_camel_case_types)]
    type SIZE_T = usize;

    const TH32CS_SNAPPROCESS: DWORD = 0x00000002;
    const TH32CS_SNAPMODULE: DWORD = 0x00000008;
    const TH32CS_SNAPMODULE32: DWORD = 0x00000010;
    const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
    const PROCESS_VM_READ: DWORD = 0x0010;
    const MEM_COMMIT: DWORD = 0x1000;
    const MEM_PRIVATE: DWORD = 0x20000;
    const PAGE_NOACCESS: DWORD = 0x01;
    const PAGE_GUARD: DWORD = 0x100;

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
    struct MODULEENTRY32W {
        dwSize: DWORD,
        th32ModuleID: DWORD,
        th32ProcessID: DWORD,
        GlblcntUsage: DWORD,
        ProccntUsage: DWORD,
        modBaseAddr: *mut c_void,
        modBaseSize: DWORD,
        hModule: HANDLE,
        szModule: [u16; 256],
        szExePath: [u16; 260],
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
        fn Module32FirstW(hSnapshot: HANDLE, lpme: *mut MODULEENTRY32W) -> BOOL;
        fn Module32NextW(hSnapshot: HANDLE, lpme: *mut MODULEENTRY32W) -> BOOL;
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

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot == -1isize as HANDLE {
            return None;
        }

        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as DWORD;

        let mut weixin_pids = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile);
                let trimmed = name.trim_matches('\0');
                if trimmed.eq_ignore_ascii_case("Weixin.exe")
                    || trimmed.eq_ignore_ascii_case("WeChat.exe")
                {
                    weixin_pids.push((entry.th32ProcessID, trimmed.to_string()));
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        if weixin_pids.is_empty() {
            return None;
        }

        let p1 = match page1 {
            Some(p) if p.len() >= 4096 => p,
            _ => return None,
        };
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&p1[..16]);
        let target_salt_hex = hex::encode(salt);

        // 1. Try to find Weixin.dll module and extract XOR masks (WeChat 4.1.x mechanism)
        let mut weixin_masks: Vec<[u8; 32]> = Vec::new();
        // Always include identity mask [0; 32] (for WeChat <= 4.1.9 without XOR mask)
        weixin_masks.push([0u8; 32]);

        for (pid, _) in &weixin_pids {
            let mod_snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, *pid);
            if !mod_snap.is_null() && mod_snap != -1isize as HANDLE {
                let mut me = std::mem::zeroed::<MODULEENTRY32W>();
                me.dwSize = std::mem::size_of::<MODULEENTRY32W>() as DWORD;
                if Module32FirstW(mod_snap, &mut me) != 0 {
                    loop {
                        let mod_name = String::from_utf16_lossy(&me.szModule);
                        let mod_trim = mod_name.trim_matches('\0');
                        if mod_trim.eq_ignore_ascii_case("Weixin.dll")
                            || mod_trim.eq_ignore_ascii_case("WeChatWin.dll")
                        {
                            let dll_path = String::from_utf16_lossy(&me.szExePath);
                            let dll_path_trim = dll_path.trim_matches('\0');
                            if let Ok(bytes) = std::fs::read(dll_path_trim) {
                                let found = scan_weixin_dll_for_internal_keys(&bytes);
                                if !found.is_empty() {
                                    eprintln!(
                                        "[i] 成功读取 Weixin.dll，提取到 {} 个内部密钥掩码候选值",
                                        found.len()
                                    );
                                    weixin_masks.extend(found);
                                }
                            }
                            break;
                        }
                        if Module32NextW(mod_snap, &mut me) == 0 {
                            break;
                        }
                    }
                }
                CloseHandle(mod_snap);
            }
            if weixin_masks.len() > 1 {
                break;
            }
        }

        let mut chunk = vec![0u8; 1024 * 1024];

        for (pid, proc_name) in weixin_pids {
            eprintln!("[i] 正在扫描 {} (PID: {}) 内存寻找匹配密钥...", proc_name, pid);
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                continue;
            }

            let mut addr = 0usize;
            let mut mbi = std::mem::zeroed::<MEMORY_BASIC_INFORMATION>();
            let mbi_size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
            let mut total_scanned = 0usize;
            let start_time = std::time::Instant::now();

            while addr < 0x7FFF_FFFF_0000 {
                if start_time.elapsed().as_millis() > 2000 || total_scanned > 128 * 1024 * 1024 {
                    break;
                }

                if VirtualQueryEx(handle, addr as *const c_void, &mut mbi, mbi_size) == 0 {
                    break;
                }

                let region_size = mbi.RegionSize;
                let is_heap = mbi.State == MEM_COMMIT
                    && mbi.Type == MEM_PRIVATE
                    && (mbi.Protect & PAGE_GUARD == 0)
                    && (mbi.Protect & PAGE_NOACCESS == 0);

                if is_heap && region_size > 0 {
                    let mut reg_offset = 0usize;
                    while reg_offset < region_size {
                        let to_read = (region_size - reg_offset).min(1024 * 1024);
                        let mut bytes_read = 0usize;
                        let cur_addr = (mbi.BaseAddress as usize) + reg_offset;

                        if ReadProcessMemory(
                            handle,
                            cur_addr as *const c_void,
                            chunk.as_mut_ptr() as *mut c_void,
                            to_read,
                            &mut bytes_read,
                        ) != 0
                            && bytes_read >= 32
                        {
                            total_scanned += bytes_read;
                            let slice = &chunk[..bytes_read];

                            // Strategy A: Scan for pattern b"x'" (WeChat <= 4.1.9)
                            let mut cursor = 0;
                            while let Some(rel) = slice[cursor..].windows(2).position(|w| w == b"x'") {
                                let match_start = cursor + rel + 2;
                                let remaining = &slice[match_start..];
                                let hex_len = remaining
                                    .iter()
                                    .take_while(|b| b.is_ascii_hexdigit())
                                    .count();

                                if (hex_len == 64 || hex_len == 96)
                                    && remaining.get(hex_len) == Some(&b'\'')
                                {
                                    if let Ok(hex_str) = std::str::from_utf8(&remaining[..hex_len]) {
                                        let salt_matches = match hex_len {
                                            96 => hex_str[64..96].eq_ignore_ascii_case(&target_salt_hex),
                                            _ => true,
                                        };

                                        if salt_matches {
                                            if let Ok(key_bytes) = hex::decode(&hex_str[..64]) {
                                                if key_bytes.len() == 32 {
                                                    let mut key_arr = [0u8; 32];
                                                    key_arr.copy_from_slice(&key_bytes);

                                                    for mask in &weixin_masks {
                                                        let mut cand = [0u8; 32];
                                                        for k in 0..32 {
                                                            cand[k] = key_arr[k] ^ mask[k];
                                                        }
                                                        let derived = derive_wechat_db_key(&cand, &salt);
                                                        if verify_db_page1(&derived, p1) || verify_db_page1(&cand, p1) {
                                                            eprintln!(
                                                                "[i] 成功从运行中的 {} (PID: {}) 内存提取到数据库密钥!",
                                                                proc_name, pid
                                                            );
                                                            CloseHandle(handle);
                                                            return Some(cand);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                cursor = match_start;
                                if cursor >= slice.len() {
                                    break;
                                }
                            }

                            // Strategy B: Scan for MSVC std::string (WeChat 4.1.x)
                            let mut j = 0;
                            while j + 32 <= bytes_read {
                                let size = u64::from_le_bytes(slice[j + 16..j + 24].try_into().unwrap_or([0; 8]));
                                let cap = u64::from_le_bytes(slice[j + 24..j + 32].try_into().unwrap_or([0; 8]));
                                if size == 32 && (cap == 47 || cap == 32 || cap == 63) {
                                    let ptr = usize::from_le_bytes(slice[j..j + 8].try_into().unwrap_or([0; 8]));
                                    if ptr >= 0x10000 && ptr <= 0x7FFF_FFFF_0000 {
                                        let mut key_buf = [0u8; 32];
                                        let mut read_sz = 0usize;
                                        if ReadProcessMemory(
                                            handle,
                                            ptr as *const c_void,
                                            key_buf.as_mut_ptr() as *mut c_void,
                                            32,
                                            &mut read_sz,
                                        ) != 0 && read_sz == 32 {
                                            for mask in &weixin_masks {
                                                let mut cand = [0u8; 32];
                                                for k in 0..32 {
                                                    cand[k] = key_buf[k] ^ mask[k];
                                                }
                                                let derived = derive_wechat_db_key(&cand, &salt);
                                                if verify_db_page1(&derived, p1) || verify_db_page1(&cand, p1) {
                                                    eprintln!(
                                                        "[i] 成功通过 Weixin.dll 内部掩码与 MSVC 特征从运行中的 {} (PID: {}) 提取出数据库密钥!",
                                                        proc_name, pid
                                                    );
                                                    CloseHandle(handle);
                                                    return Some(cand);
                                                }
                                            }
                                        }
                                    }
                                }
                                j += 8;
                            }
                        }

                        reg_offset += to_read;
                        if to_read < 1024 * 1024 {
                            break;
                        }
                    }
                }

                let next = addr.saturating_add(region_size);
                if next <= addr {
                    break;
                }
                addr = next;
            }

            CloseHandle(handle);
        }

        eprintln!("[!] 未能在运行中的微信内存中匹配到解密密钥。");
        None
    }
}
#[cfg(not(target_os = "windows"))]
pub fn auto_scan_wechat_key_from_process(_page1: Option<&[u8]>) -> Option<[u8; 32]> {
    None
}

impl Db {
    pub fn open(path: &Path, key: Option<&[u8; 32]>, plaintext: bool) -> Result<Db> {
        if !path.exists() {
            bail!("database does not exist: {}", path.display());
        }
        let is_plain = is_plain_sqlite(path);
        if plaintext {
            if !is_plain {
                bail!(
                    "database {} is not a readable plaintext SQLite database",
                    path.display()
                );
            }
            return Self::try_open_plain(path);
        }
        match key {
            None => {
                if is_plain {
                    Self::try_open_plain(path)
                } else {
                    // Attempt to auto-scan key from running WeChat process on Windows
                    use std::io::Read;
                    let mut file = std::fs::File::open(path)?;
                    let mut page1 = [0u8; 4096];
                    let read_bytes = file.read(&mut page1).unwrap_or(0);
                    let p1_ref = if read_bytes >= 4096 { Some(&page1[..]) } else { None };

                    if let Some(scanned_key) = auto_scan_wechat_key_from_process(p1_ref) {
                        Self::try_open_keyed(path, &scanned_key).map_err(|e| {
                            anyhow::anyhow!(
                                "failed to open database {} with auto-scanned key: {}",
                                path.display(),
                                e
                            )
                        })
                    } else {
                        bail!(
                            "database {} is encrypted; a 64-hex database key is required.
(未能从运行中的微信内存自动捕获密钥。请保持微信登录中，或通过 --key <64位密钥> 手动传入)",
                            path.display()
                        )
                    }
                }
            }
            Some(key) => {
                if is_plain {
                    if let Ok(db) = Self::try_open_plain(path) {
                        return Ok(db);
                    }
                }
                Self::try_open_keyed(path, key).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to open database {}: the database key is wrong or the file is not a SQLCipher database ({})",
                        path.display(),
                        e
                    )
                })
            }
        }
    }

    fn try_open_plain(path: &Path) -> Result<Db> {
        let conn = open_checked(path, check_connection)?;
        Ok(Db { conn })
    }

    fn try_open_keyed(path: &Path, key: &[u8; 32]) -> Result<Db> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut page1 = [0u8; 4096];
        let read_bytes = file.read(&mut page1).unwrap_or(0);

        let final_key = if read_bytes >= 4096 {
            let mut salt = [0u8; 16];
            salt.copy_from_slice(&page1[..16]);

            // 1. Check if the provided key is the WeChat account master passphrase
            // requiring per-database PBKDF2 derivation (WeChat 4.x standard):
            let derived = derive_wechat_db_key(key, &salt);
            if verify_db_page1(&derived, &page1) {
                derived
            } else if verify_db_page1(key, &page1) {
                // 2. The provided key is already the raw per-database key:
                *key
            } else {
                // 3. Fallback: try deriving from hex ASCII representation
                let hex_str = hex::encode(key);
                let derived_ascii = derive_wechat_db_key(hex_str.as_bytes(), &salt);
                if verify_db_page1(&derived_ascii, &page1) {
                    derived_ascii
                } else {
                    // Try derived key first, then raw key
                    derived
                }
            }
        } else {
            *key
        };

        // Try opening with the resolved key
        if let Ok(db) = Self::try_open_with_hex(path, &hex::encode(final_key)) {
            return Ok(db);
        }

        // If the derived attempt failed and differed from input key, try raw input key as fallback
        if final_key != *key {
            if let Ok(db) = Self::try_open_with_hex(path, &hex::encode(key)) {
                return Ok(db);
            }
        }

        bail!("database key verification failed for {}", path.display())
    }

    fn try_open_with_hex(path: &Path, hex_key: &str) -> Result<Db> {
        let check = |conn: &Connection| -> Result<()> {
            conn.busy_timeout(std::time::Duration::from_secs(2))?;
            conn.execute_batch(&format!("PRAGMA key = \"x'{hex_key}'\";"))
                .context("failed to apply SQLCipher raw key")?;
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .context("SQLCipher key check failed")?;
            conn.execute_batch("PRAGMA query_only = ON;")?;
            Ok(())
        };
        let conn = open_checked(path, check)?;
        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn tables(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn columns(&self, table: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", quote_ident(table)))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn count(&self, table: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            &format!("SELECT COUNT(*) FROM {}", quote_ident(table)),
            [],
            |row| row.get(0),
        )?)
    }
}

pub fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_plaintext_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("plain.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(x TEXT); INSERT INTO t VALUES('ok');")
            .unwrap();
        drop(conn);
        let db = Db::open(&path, None, true).unwrap();
        assert_eq!(db.count("t").unwrap(), 1);
    }

    #[test]
    fn opens_raw_key_sqlcipher_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("enc.db");
        let key = [0x42u8; 32];
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(key)))
                .unwrap();
            conn.execute_batch("CREATE TABLE t(x TEXT); INSERT INTO t VALUES('ok');")
                .unwrap();
        }
        let db = Db::open(&path, Some(&key), false).unwrap();
        assert_eq!(db.count("t").unwrap(), 1);
        let wrong = [0x11u8; 32];
        assert!(Db::open(&path, Some(&wrong), false).is_err());
    }

    #[test]
    fn verifies_page1_hmac_calculation() {
        let salt = [0x12u8; 16];
        let passphrase = [0xab; 32];
        let derived = derive_wechat_db_key(&passphrase, &salt);
        assert_ne!(derived, passphrase);
    }
}

    #[test]
    fn opens_wechat_pbkdf2_derived_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("wechat_sim.db");
        let master_passphrase = [0x55u8; 32];
        let salt = [0x77u8; 16];

        // 1. Simulate WeChat WCDB database creation:
        // WeChat derives enc_key from master_passphrase + salt using PBKDF2
        let enc_key = derive_wechat_db_key(&master_passphrase, &salt);
        {
            let conn = Connection::open(&path).unwrap();
            // In SQLCipher raw key mode: pass key + salt in x'...'
            let hex_key_salt = format!("{}{}", hex::encode(enc_key), hex::encode(salt));
            conn.execute_batch(&format!("PRAGMA key = \"x'{hex_key_salt}'\";"))
                .unwrap();
            conn.execute_batch("CREATE TABLE messages (id INTEGER PRIMARY KEY, text TEXT); INSERT INTO messages VALUES(1, 'secret message');")
                .unwrap();
        }

        // 2. Open using Db::open with ONLY the master passphrase!
        let db = Db::open(&path, Some(&master_passphrase), false).expect("must open with master passphrase");
        assert_eq!(db.count("messages").unwrap(), 1);
    }
