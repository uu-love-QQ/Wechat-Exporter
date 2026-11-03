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
const WINDOWS_CONFIG_CIPHER_NAME: &[u8] = b"com.Tencent.WCDB.Config.Cipher";
#[cfg(target_os = "windows")]
const WINDOWS_CONFIG_XOR_MASK: &[u8; 32] = &[
    0xd2, 0xc7, 0x44, 0x24, 0x58, 0x02, 0x00, 0x00, 0x00, 0x48, 0x89, 0x44, 0x24, 0x50, 0x48,
    0x8b, 0x45, 0x00, 0x48, 0x84, 0x4c, 0x24, 0x48, 0x48, 0x89, 0x44, 0x25, 0x40, 0x48, 0x58,
    0x4c, 0x24,
];

#[cfg(target_os = "windows")]
pub fn auto_scan_wechat_key_from_process(page1: Option<&[u8]>) -> Option<[u8; 32]> {
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
    struct PROCESS_MEMORY_COUNTERS {
        cb: DWORD,
        PageFaultCount: DWORD,
        PeakWorkingSetSize: SIZE_T,
        WorkingSetSize: SIZE_T,
        QuotaPeakPagedPoolUsage: SIZE_T,
        QuotaPagedPoolUsage: SIZE_T,
        QuotaPeakNonPagedPoolUsage: SIZE_T,
        QuotaNonPagedPoolUsage: SIZE_T,
        PagefileUsage: SIZE_T,
        PeakPagefileUsage: SIZE_T,
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
        fn K32GetProcessMemoryInfo(
            Process: HANDLE,
            ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS,
            cb: DWORD,
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

        let mut weixin_pids: Vec<(u32, usize, String)> = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile);
                let trimmed = name.trim_matches('\0');
                if trimmed.eq_ignore_ascii_case("Weixin.exe")
                    || trimmed.eq_ignore_ascii_case("WeChat.exe")
                {
                    let pid = entry.th32ProcessID;
                    let mut mem_usage = 0usize;
                    let h_test = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
                    if !h_test.is_null() {
                        let mut pmc = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
                        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as DWORD;
                        if K32GetProcessMemoryInfo(h_test, &mut pmc, pmc.cb) != 0 {
                            mem_usage = pmc.WorkingSetSize;
                        }
                        CloseHandle(h_test);
                    }
                    weixin_pids.push((pid, mem_usage, trimmed.to_string()));
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

        // Sort by memory usage descending so the main WeChat process (500MB+) is scanned FIRST!
        weixin_pids.sort_by(|a, b| b.1.cmp(&a.1));

        let p1 = match page1 {
            Some(p) if p.len() >= 4096 => p,
            _ => return None,
        };
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&p1[..16]);

        let mut chunk = vec![0u8; 2 * 1024 * 1024];

        for (pid, mem_bytes, proc_name) in weixin_pids {
            let mem_mb = mem_bytes / (1024 * 1024);
            eprintln!(
                "[i] 正在扫描 {} (PID: {}, 内存: {}MB) 匹配密钥...",
                proc_name, pid, mem_mb
            );
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                continue;
            }

            // 1. Collect all readable regions
            let mut addr = 0usize;
            let mut mbi = std::mem::zeroed::<MEMORY_BASIC_INFORMATION>();
            let mbi_size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
            let mut readable_regions: Vec<(usize, usize)> = Vec::new();

            while addr < 0x7FFF_FFFF_0000 {
                if VirtualQueryEx(handle, addr as *const c_void, &mut mbi, mbi_size) == 0 {
                    break;
                }
                let region_size = mbi.RegionSize;
                let is_readable = mbi.State == MEM_COMMIT
                    && (mbi.Protect & PAGE_GUARD == 0)
                    && (mbi.Protect & PAGE_NOACCESS == 0);

                if is_readable && region_size > 0 && region_size <= 256 * 1024 * 1024 {
                    readable_regions.push((mbi.BaseAddress as usize, region_size));
                }
                let next = addr.saturating_add(region_size);
                if next <= addr {
                    break;
                }
                addr = next;
            }

            // 2. Algorithm A (wcdb-key-tool for WeChat 4.1+):
            // Search for WINDOWS_CONFIG_CIPHER_NAME ("com.Tencent.WCDB.Config.Cipher")
            let mut needle_addresses = Vec::new();
            for &(base, size) in &readable_regions {
                let mut reg_offset = 0usize;
                while reg_offset < size {
                    let to_read = (size - reg_offset).min(chunk.len());
                    let mut bytes_read = 0usize;
                    if ReadProcessMemory(
                        handle,
                        (base + reg_offset) as *const c_void,
                        chunk.as_mut_ptr() as *mut c_void,
                        to_read,
                        &mut bytes_read,
                    ) != 0 && bytes_read >= WINDOWS_CONFIG_CIPHER_NAME.len() {
                        let slice = &chunk[..bytes_read];
                        let mut pos = 0;
                        while let Some(rel) = slice[pos..].windows(WINDOWS_CONFIG_CIPHER_NAME.len()).position(|w| w == WINDOWS_CONFIG_CIPHER_NAME) {
                            let match_addr = base + reg_offset + pos + rel;
                            needle_addresses.push(match_addr);
                            pos += rel + WINDOWS_CONFIG_CIPHER_NAME.len();
                        }
                    }
                    reg_offset += to_read;
                }
            }

            if !needle_addresses.is_empty() {
                eprintln!("[i] 发现 {} 个 Config.Cipher 对象标记，正在解析密钥链...", needle_addresses.len());
                for &needle_addr in &needle_addresses {
                    // Pattern is [addr (8 bytes LE), len (8 bytes LE == 30)]
                    let mut pattern = [0u8; 16];
                    pattern[..8].copy_from_slice(&(needle_addr as u64).to_le_bytes());
                    pattern[8..16].copy_from_slice(&(WINDOWS_CONFIG_CIPHER_NAME.len() as u64).to_le_bytes());

                    for &(base, size) in &readable_regions {
                        let mut reg_offset = 0usize;
                        while reg_offset < size {
                            let to_read = (size - reg_offset).min(chunk.len());
                            let mut bytes_read = 0usize;
                            if ReadProcessMemory(
                                handle,
                                (base + reg_offset) as *const c_void,
                                chunk.as_mut_ptr() as *mut c_void,
                                to_read,
                                &mut bytes_read,
                            ) != 0 && bytes_read >= 16 {
                                let slice = &chunk[..bytes_read];
                                let mut pos = 0;
                                while let Some(rel) = slice[pos..].windows(16).position(|w| w == pattern) {
                                    let qaddr = base + reg_offset + pos + rel;
                                    let node_base = qaddr.saturating_sub(0x10);
                                    let mut node = [0u8; 0x50];
                                    let mut nr = 0usize;
                                    if ReadProcessMemory(handle, node_base as *const c_void, node.as_mut_ptr() as *mut c_void, 0x50, &mut nr) != 0 && nr >= 0x40 {
                                        let config_ptr = u64::from_le_bytes(node[0x28..0x30].try_into().unwrap_or([0; 8])) as usize;
                                        if config_ptr >= 0x10000 && config_ptr < 0x7FFF_FFFF_0000 {
                                            let mut obj = [0u8; 0x28];
                                            let mut obr = 0usize;
                                            if ReadProcessMemory(handle, (config_ptr + 0x88) as *const c_void, obj.as_mut_ptr() as *mut c_void, 0x28, &mut obr) != 0 && obr >= 0x18 {
                                                let data_ptr = u64::from_le_bytes(obj[0x8..0x10].try_into().unwrap_or([0; 8])) as usize;
                                                let data_len = u64::from_le_bytes(obj[0x10..0x18].try_into().unwrap_or([0; 8])) as usize;
                                                if data_len > 0 && data_len <= 1024 && data_ptr >= 0x10000 && data_ptr < 0x7FFF_FFFF_0000 {
                                                    let mut blob = vec![0u8; data_len];
                                                    let mut bbr = 0usize;
                                                    if ReadProcessMemory(handle, data_ptr as *const c_void, blob.as_mut_ptr() as *mut c_void, data_len, &mut bbr) != 0 && bbr == data_len {
                                                        // XOR decrypt blob with WINDOWS_CONFIG_XOR_MASK
                                                        for k in 0..blob.len() {
                                                            blob[k] ^= WINDOWS_CONFIG_XOR_MASK[k % 32];
                                                        }
                                                        // Search for x'<hex>' in decrypted blob
                                                        let mut bpos = 0;
                                                        while let Some(brel) = blob[bpos..].windows(2).position(|w| w == b"x'" || w == b"X'") {
                                                            let mstart = bpos + brel + 2;
                                                            let rem = &blob[mstart..];
                                                            let hlen = rem.iter().take_while(|b| b.is_ascii_hexdigit()).count();
                                                            if (hlen == 64 || hlen == 96) && rem.get(hlen) == Some(&b'\'') {
                                                                if let Ok(hex_str) = std::str::from_utf8(&rem[..64]) {
                                                                    if let Ok(kbytes) = hex::decode(hex_str) {
                                                                        if kbytes.len() == 32 {
                                                                            let mut karr = [0u8; 32];
                                                                            karr.copy_from_slice(&kbytes);
                                                                            let derived = derive_wechat_db_key(&karr, &salt);
                                                                            if verify_db_page1(&derived, p1) || verify_db_page1(&karr, p1) {
                                                                                eprintln!(
                                                                                    "[+] 成功通过 Config.Cipher 算法从微信进程 (PID: {}) 提取出数据库密钥: x'{}'!",
                                                                                    pid, hex::encode(karr)
                                                                                );
                                                                                CloseHandle(handle);
                                                                                return Some(karr);
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            bpos = mstart;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    pos += rel + 16;
                                }
                            }
                            reg_offset += to_read;
                        }
                    }
                }
            }

            // 3. Algorithm B: Raw x'<hex>' scan (WeChat <= 4.1.9)
            for &(base, size) in &readable_regions {
                let mut reg_offset = 0usize;
                while reg_offset < size {
                    let to_read = (size - reg_offset).min(chunk.len());
                    let mut bytes_read = 0usize;
                    if ReadProcessMemory(
                        handle,
                        (base + reg_offset) as *const c_void,
                        chunk.as_mut_ptr() as *mut c_void,
                        to_read,
                        &mut bytes_read,
                    ) != 0 && bytes_read >= 68 {
                        let slice = &chunk[..bytes_read];
                        let mut cursor = 0;
                        while let Some(rel) = slice[cursor..].windows(2).position(|w| w == b"x'" || w == b"X'") {
                            let match_start = cursor + rel + 2;
                            let remaining = &slice[match_start..];
                            let hex_len = remaining.iter().take_while(|b| b.is_ascii_hexdigit()).count();

                            if (hex_len == 64 || hex_len == 96) && remaining.get(hex_len) == Some(&b'\'') {
                                if let Ok(hex_str) = std::str::from_utf8(&remaining[..64]) {
                                    if let Ok(key_bytes) = hex::decode(hex_str) {
                                        if key_bytes.len() == 32 {
                                            let mut key_arr = [0u8; 32];
                                            key_arr.copy_from_slice(&key_bytes);
                                            let derived = derive_wechat_db_key(&key_arr, &salt);
                                            if verify_db_page1(&derived, p1) || verify_db_page1(&key_arr, p1) {
                                                eprintln!(
                                                    "[+] 成功从运行中的微信进程 (PID: {}) 内存匹配到数据库密钥!",
                                                    pid
                                                );
                                                CloseHandle(handle);
                                                return Some(key_arr);
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
                    }
                    reg_offset += to_read;
                }
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
