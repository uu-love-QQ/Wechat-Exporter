use aes::cipher::{BlockDecrypt, KeyInit};
use aes::Aes128;
use anyhow::{bail, Result};
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{ExportMessage, MediaKind};

const V1_MAGIC: &[u8] = b"\x07\x08V1\x08\x07";
const V2_MAGIC: &[u8] = b"\x07\x08V2\x08\x07";

static IMAGE_SEED_CACHE: std::sync::Mutex<Option<(u32, [u8; 16], u8)>> =
    std::sync::Mutex::new(None);

pub fn set_image_seed_cache(seed: u32, key: [u8; 16], xor_key: u8) {
    if let Ok(mut lock) = IMAGE_SEED_CACHE.lock() {
        *lock = Some((seed, key, xor_key));
    }
}

#[derive(Clone, Debug)]
struct IndexedFile {
    path: PathBuf,
}

pub struct MediaIndex {
    files: Vec<IndexedFile>,
    by_name: HashMap<String, Vec<usize>>,
    by_md5: HashMap<String, Vec<usize>>,
    persist_roots: Vec<PathBuf>,
}

fn is_hash_token(value: &str) -> bool {
    (value.len() == 32 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl MediaIndex {
    fn add_file(&mut self, path: PathBuf, lower_name: String) {
        let stem = Path::new(&lower_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let idx = self.files.len();
        self.by_name.entry(lower_name).or_default().push(idx);
        let mut tokens = Vec::new();
        for token in stem.split(['_', '-', '.']) {
            if is_hash_token(token) {
                tokens.push(token.to_string());
            }
        }
        if is_hash_token(&stem) {
            tokens.push(stem);
        }
        for token in tokens {
            self.by_md5.entry(token).or_default().push(idx);
        }
        self.files.push(IndexedFile { path });
    }

    pub fn build(account_root: Option<&Path>) -> MediaIndex {
        let mut index = MediaIndex {
            files: Vec::new(),
            by_name: HashMap::new(),
            by_md5: HashMap::new(),
            persist_roots: Vec::new(),
        };
        if let Some(root) = account_root {
            for base in [root.to_path_buf()] {
                let persist = base.join("business").join("emoticon").join("Persist");
                if persist.is_dir() {
                    index.persist_roots.push(persist);
                }
            }
            if let Some(parent) = root.parent() {
                let persist = parent.join("business").join("emoticon").join("Persist");
                if persist.is_dir() {
                    index.persist_roots.push(persist);
                }
            }
            let mut stack = vec![(root.to_path_buf(), 0usize)];
            if let Some(parent) = root.parent() {
                for sibling in ["FileStorage", "All Users"] {
                    let extra = parent.join(sibling);
                    if extra.is_dir() {
                        stack.push((extra, 0usize));
                    }
                }
            }
            while let Some((dir, depth)) = stack.pop() {
                if depth > 12 || index.files.len() > 1_000_000 {
                    continue;
                }
                let Ok(entries) = fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    // Follow directory symlinks/junctions, skip non-directory symlinks
                    let file_type = entry.file_type().ok();
                    if file_type.as_ref().map(|k| k.is_symlink()).unwrap_or(false) && !metadata.is_dir() {
                        continue;
                    }
                    if metadata.is_dir() {
                        if matches!(
                            name.as_str(),
                            "db_storage" | "tmp" | "temp" | "backup" | "node_modules" | ".git"
                        ) {
                            continue;
                        }
                        stack.push((path, depth + 1));
                    } else if metadata.is_file() {
                        index.add_file(path, name.to_ascii_lowercase());
                    }
                }
            }
        }
        if !index.files.is_empty() {
            eprintln!("[i] 媒体索引: {} 个文件", index.files.len());
        }
        index
    }


    /// Full-res custom emoticons are stored at Persist/<sha[0..2]>/<sha256-of-plaintext>.
    /// The low-res Thumb decrypts to that same plaintext, so its hash is the lookup key.
    fn find_persist_by_sha256(&self, sha256: &str) -> Option<PathBuf> {
        let sha = sha256.to_ascii_lowercase();
        if sha.len() != 64 {
            return None;
        }
        for root in &self.persist_roots {
            let candidate = root.join(&sha[..2]).join(&sha);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Score a single indexed file for the requested media kind. Higher wins.
    fn score_file(&self, file: &IndexedFile, kind: MediaKind) -> i64 {
        let ext = file
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let base_score = match kind {
            MediaKind::Image => match ext.as_str() {
                "jpg" | "jpeg" | "png" | "webp" => 20,
                "gif" => 15,
                "dat" => 10,
                _ => 1,
            },
            MediaKind::Video => match ext.as_str() {
                "mp4" | "mov" | "m4v" => 20,
                "dat" => 10,
                _ => 1,
            },
            MediaKind::Voice => match ext.as_str() {
                "wav" | "silk" | "mp3" | "amr" => 20,
                "dat" => 10,
                _ => 1,
            },
            MediaKind::Emoji => match ext.as_str() {
                "gif" => 30,
                "png" | "jpg" | "jpeg" | "webp" => 15,
                "bin" => 8,
                "dat" => 6,
                "thumb" => 2,
                "" => 15, // extension-less files under Persist are full originals
                _ => 1,
            },
            MediaKind::File => {
                if ext == "dat" {
                    1
                } else {
                    20
                }
            }
        };
        let path_lower = file.path.to_string_lossy().to_ascii_lowercase();
        let mut dir_score = 0;
        if path_lower.contains("\\persist\\") || path_lower.contains("/persist/") {
            dir_score += 100;
        }
        if path_lower.contains("\\thumb\\") || path_lower.contains("/thumb/") {
            dir_score -= 40;
        }
        if path_lower.contains("\\bubble\\") || path_lower.contains("/bubble/") {
            dir_score -= 30;
        }
        if path_lower.ends_with("_t.dat") || path_lower.ends_with("_b.dat") {
            dir_score -= 20;
        }
        if path_lower.contains("_h.dat") {
            dir_score += 30;
        }
        (base_score + dir_score).max(1) as i64
    }

    /// Every candidate for this message, best first. Directory names alone are not
    /// trusted: ordering prefers candidates whose on-disk size matches the size the
    /// message declares, so a `cache/<month>/Emoticon/...` original also wins.
    fn candidates(&self, media: &super::model::MediaRef) -> Vec<usize> {
        let all_md5s = std::iter::once(&media.md5)
            .chain(media.candidate_md5s.iter())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect::<Vec<_>>();

        let mut indices: Vec<usize> = Vec::new();
        for lower in &all_md5s {
            if let Some(list) = self.by_md5.get(lower) {
                indices.extend(list.iter().copied());
            }
            if lower.len() >= 16 {
                for (token, list) in &self.by_md5 {
                    if token.starts_with(lower.as_str()) || lower.starts_with(token.as_str()) {
                        indices.extend(list.iter().copied());
                    }
                }
            }
        }
        if !media.file_name.is_empty() {
            let wanted = media.file_name.to_ascii_lowercase();
            if let Some(list) = self.by_name.get(&wanted) {
                indices.extend(list.iter().copied());
            }
            let stem = Path::new(&wanted)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if let Some(list) = self.by_name.get(&format!("{stem}.dat")) {
                indices.extend(list.iter().copied());
            }
        }
        indices.sort_unstable();
        indices.dedup();

        let declared = media.declared_size.unwrap_or(0) as i64;
        indices.sort_by(|a, b| {
            let fa = &self.files[*a];
            let fb = &self.files[*b];
            let sa = self.score_file(fa, media.kind);
            let sb = self.score_file(fb, media.kind);
            let size_a = fs::metadata(&fa.path).map(|m| m.len()).unwrap_or(0) as i64;
            let size_b = fs::metadata(&fb.path).map(|m| m.len()).unwrap_or(0) as i64;
            // 1) kind/directory score, 2) closeness to the declared plaintext length,
            // 3) larger file. Declared length is the strongest "is this the original" hint.
            sb.cmp(&sa)
                .then_with(|| {
                    if declared > 0 {
                        (size_a - declared).abs().cmp(&(size_b - declared).abs())
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| size_b.cmp(&size_a))
        });
        indices
    }

}

pub struct MediaOutcome {
    pub saved: usize,
    pub failed: usize,
    pub failures: Vec<String>,
}


#[cfg(target_os = "windows")]
pub fn auto_scan_image_key_from_process(test_ciphertext: &[u8; 16]) -> Option<[u8; 16]> {
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

        let mut chunk = vec![0u8; 1024 * 1024];

        for (pid, proc_name) in weixin_pids {
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
                if start_time.elapsed().as_millis() > 1000 || total_scanned > 64 * 1024 * 1024 {
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
                            && bytes_read >= 16
                        {
                            total_scanned += bytes_read;
                            let slice = &chunk[..bytes_read];
                            let mut i = 0;
                            while i + 16 <= slice.len() {
                                let candidate = &slice[i..i + 16];
                                if candidate.iter().all(|b| b.is_ascii_alphanumeric()) {
                                    if let Ok(cipher) = Aes128::new_from_slice(candidate) {
                                        let mut block = *test_ciphertext;
                                        let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(&mut block);
                                        cipher.decrypt_block(generic);
                                        if is_image_header(&block) {
                                            let mut key_arr = [0u8; 16];
                                            key_arr.copy_from_slice(candidate);
                                            eprintln!(
                                                "[i] 成功从运行中的 {} (PID: {}) 内存自动获取图片解密 AES 密钥: \"{}\"!",
                                                proc_name, pid, String::from_utf8_lossy(candidate)
                                            );
                                            CloseHandle(handle);
                                            return Some(key_arr);
                                        }
                                    }
                                    i += 16;
                                } else {
                                    i += 1;
                                }
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

        None
    }
}
#[cfg(not(target_os = "windows"))]
pub fn auto_scan_image_key_from_process(_test_ciphertext: &[u8; 16]) -> Option<[u8; 16]> {
    None
}

/// WeChat 4.x chat-image key derivation (verified against a real account):
/// `aes_key = md5(format!("{seed}{wxid}"))[..16]` (ASCII) and
/// `xor_key = seed & 0xff`.
pub fn image_key_from_seed(seed: u32, wxid: &str) -> ([u8; 16], u8) {
    let digest = crate::util::md5_hex(&format!("{seed}{wxid}"));
    let mut key = [0u8; 16];
    key.copy_from_slice(digest.as_bytes().get(..16).unwrap_or(b"0000000000000000"));
    (key, (seed & 0xff) as u8)
}

/// WeChat emoticon key: `md5(format!("{seed}{wxid}EMOTICON"))` hex-decoded,
/// first 16 bytes; AES-128-CBC with key == IV.
pub fn emoticon_key_from_seed(seed: u32, wxid: &str) -> Option<[u8; 16]> {
    let digest = crate::util::md5_hex(&format!("{seed}{wxid}EMOTICON"));
    let bytes = hex::decode(digest.get(..32)?).ok()?;
    let mut key = [0u8; 16];
    key.copy_from_slice(&bytes[..16]);
    Some(key)
}

fn emoticon_first_block_is_image(data: &[u8], key: &[u8; 16]) -> bool {
    if data.len() < 16 {
        return false;
    }
    let Ok(cipher) = Aes128::new_from_slice(key) else {
        return false;
    };
    let mut block = [0u8; 16];
    block.copy_from_slice(&data[..16]);
    let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(&mut block);
    cipher.decrypt_block(generic);
    for i in 0..16 {
        block[i] ^= key[i];
    }
    is_strict_image_header(&block) || block.starts_with(b"wxgf")
}

#[cfg(target_os = "windows")]
pub fn auto_scan_image_seed_from_process(
    wxid: &str,
    validate: &dyn Fn(u32) -> bool,
) -> Option<(u32, [u8; 16], u8)> {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type DWORD = u32;
    type BOOL = i32;
    #[allow(non_camel_case_types)]
    type SIZE_T = usize;

    const TH32CS_SNAPPROCESS: DWORD = 0x0000_0002;
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

    fn wide_trimmed(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    fn collect_seeds(data: &[u8], out: &mut HashSet<u32>) {
        let mut i = 0usize;
        while i < data.len() {
            if !data[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < data.len() && data[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (9..=10).contains(&len) {
                if let Ok(text) = std::str::from_utf8(&data[start..i]) {
                    if let Ok(value) = text.parse::<u64>() {
                        if value > 100_000_000 && value < 4_000_000_000 {
                            out.insert(value as u32);
                        }
                    }
                }
            }
        }
    }

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot == -1isize as HANDLE {
            return None;
        }
        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as DWORD;
        let mut pids: Vec<(u32, usize)> = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_trimmed(&entry.szExeFile);
                if name.eq_ignore_ascii_case("Weixin.exe")
                    || name.eq_ignore_ascii_case("WeChat.exe")
                {
                    let pid = entry.th32ProcessID;
                    let mut mem = 0usize;
                    let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
                    if !h.is_null() {
                        let mut pmc = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
                        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as DWORD;
                        if K32GetProcessMemoryInfo(h, &mut pmc, pmc.cb) != 0 {
                            mem = pmc.WorkingSetSize;
                        }
                        CloseHandle(h);
                    }
                    pids.push((pid, mem));
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        pids.sort_by(|a, b| b.1.cmp(&a.1));

        let mut chunk = vec![0u8; 4 * 1024 * 1024];
        for (pid, mem) in pids {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                continue;
            }
            eprintln!(
                "[i] 正在扫描 Weixin.exe (PID: {}, 内存: {}MB) 查找图片种子...",
                pid,
                mem / (1024 * 1024)
            );
            let mut seeds: HashSet<u32> = HashSet::new();
            let mut addr = 0usize;
            loop {
                let mut mbi = std::mem::zeroed::<MEMORY_BASIC_INFORMATION>();
                if VirtualQueryEx(
                    handle,
                    addr as *const c_void,
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                ) == 0
                {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                let next = match base.checked_add(size) {
                    Some(next) if next > base => next,
                    _ => break,
                };
                if mbi.State == MEM_COMMIT
                    && mbi.Type == MEM_PRIVATE
                    && size > 0
                    && mbi.Protect & PAGE_NOACCESS == 0
                    && mbi.Protect & PAGE_GUARD == 0
                {
                    let mut offset = 0usize;
                    let mut tail: Vec<u8> = Vec::new();
                    while offset < size {
                        let want = (size - offset).min(chunk.len());
                        let mut got = 0usize;
                        if ReadProcessMemory(
                            handle,
                            (base + offset) as *const c_void,
                            chunk.as_mut_ptr() as *mut c_void,
                            want,
                            &mut got,
                        ) != 0
                            && got > 0
                        {
                            let mut window = Vec::with_capacity(tail.len() + got);
                            window.extend_from_slice(&tail);
                            window.extend_from_slice(&chunk[..got]);
                            collect_seeds(&window, &mut seeds);
                            let keep = window.len().min(16);
                            tail = window[window.len() - keep..].to_vec();
                        }
                        offset = offset.saturating_add(want);
                    }
                }
                addr = next;
            }

            for seed in seeds {
                if !validate(seed) {
                    continue;
                }
                let (key, xor_key) = image_key_from_seed(seed, wxid);
                {
                    eprintln!(
                        "[+] 成功从 Weixin.exe (PID: {}) 提取图片种子: {} (AES: {}, XOR: 0x{:02x})",
                        pid,
                        seed,
                        String::from_utf8_lossy(&key),
                        xor_key
                    );
                    CloseHandle(handle);
                    return Some((seed, key, xor_key));
                }
            }
            CloseHandle(handle);
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub fn auto_scan_image_seed_from_process(
    _wxid: &str,
    _validate: &dyn Fn(u32) -> bool,
) -> Option<(u32, [u8; 16], u8)> {
    None
}

pub fn resolve_image_keys(
    index: &MediaIndex,
    account_id: &str,
    mut image_key: Option<[u8; 16]>,
) -> (Option<[u8; 16]>, Option<u8>, Option<u32>) {
    let mut image_xor_key: Option<u8> = None;
    let mut image_seed: Option<u32> = None;

    let mut v2_candidates: Vec<(u64, [u8; 16])> = Vec::new();
    let mut emoticon_candidates: Vec<(u64, [u8; 16])> = Vec::new();
    for file in &index.files {
        let is_dat = file
            .path
            .extension()
            .and_then(|v| v.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("dat"))
            .unwrap_or(false);
        let is_emoticon = file
            .path
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("emoticon")
            || file
                .path
                .extension()
                .and_then(|v| v.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("thumb"))
                .unwrap_or(false);
        if !is_dat && !is_emoticon {
            continue;
        }
        let Ok(meta) = fs::metadata(&file.path) else {
            continue;
        };
        let size = meta.len();
        if is_dat && v2_candidates.len() < 8 && size >= 31 {
            if let Ok(mut handle) = fs::File::open(&file.path) {
                use std::io::Read;
                let mut head = [0u8; 31];
                if handle.read_exact(&mut head).is_ok() && head.starts_with(V2_MAGIC) {
                    let mut block = [0u8; 16];
                    block.copy_from_slice(&head[15..31]);
                    v2_candidates.push((size, block));
                }
            }
        }
        if is_emoticon && emoticon_candidates.len() < 8 && size >= 16 && size % 16 == 0 {
            if let Ok(mut handle) = fs::File::open(&file.path) {
                use std::io::Read;
                let mut block = [0u8; 16];
                if handle.read_exact(&mut block).is_ok() && !is_image_header(&block) {
                    emoticon_candidates.push((size, block));
                }
            }
        }
        if v2_candidates.len() >= 8 && emoticon_candidates.len() >= 8 {
            break;
        }
    }
    v2_candidates.sort_by(|a, b| b.0.cmp(&a.0));
    emoticon_candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let v2_samples: Vec<[u8; 16]> = v2_candidates.into_iter().map(|(_, block)| block).collect();
    let emoticon_samples: Vec<[u8; 16]> =
        emoticon_candidates.into_iter().map(|(_, block)| block).collect();

    if let Some((seed, key, xor_key)) = *IMAGE_SEED_CACHE.lock().unwrap() {
        if image_key.is_none() {
            image_key = Some(key);
        }
        if image_xor_key.is_none() {
            image_xor_key = Some(xor_key);
        }
        if image_seed.is_none() {
            image_seed = Some(seed);
        }
    }
    let need_scan = image_key.is_none() || image_xor_key.is_none() || image_seed.is_none();
    if need_scan && !v2_samples.is_empty() {
        eprintln!(
            "[i] 使用 {} 个 V2 图片样本交叉确认图片种子",
            v2_samples.len()
        );
        let scan = auto_scan_image_seed_from_process(account_id, &|seed| {
            let (key, _) = image_key_from_seed(seed, account_id);
            let Ok(cipher) = Aes128::new_from_slice(&key) else {
                return false;
            };
            v2_samples.iter().all(|sample| {
                let mut block = *sample;
                let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(&mut block);
                cipher.decrypt_block(generic);
                is_strict_image_header(&block)
            })
        });
        if let Some((seed, key, xor_key)) = scan {
            image_seed = Some(seed);
            image_key = Some(key);
            image_xor_key = Some(xor_key);
            *IMAGE_SEED_CACHE.lock().unwrap() = Some((seed, key, xor_key));
        }
    }
    if (image_key.is_none() || image_xor_key.is_none() || image_seed.is_none()) && !emoticon_samples.is_empty() {
        eprintln!(
            "[i] 使用 {} 个表情样本交叉确认图片种子",
            emoticon_samples.len()
        );
        let scan = auto_scan_image_seed_from_process(account_id, &|seed| {
            let Some(key) = emoticon_key_from_seed(seed, account_id) else {
                return false;
            };
            emoticon_samples
                .iter()
                .all(|sample| emoticon_first_block_is_image(sample, &key))
        });
        if let Some((seed, key, xor_key)) = scan {
            image_seed = Some(seed);
            image_key = Some(key);
            image_xor_key = Some(xor_key);
            *IMAGE_SEED_CACHE.lock().unwrap() = Some((seed, key, xor_key));
        }
    }
    if image_key.is_none() {
        if let Some(test_ciphertext) = v2_samples.first() {
            if let Some(scanned) = auto_scan_image_key_from_process(test_ciphertext) {
                eprintln!(
                    "[!] 未找到图片种子，退回 16 字节 ASCII 密钥扫描: {}",
                    String::from_utf8_lossy(&scanned)
                );
                image_key = Some(scanned);
            }
        }
    }

    (image_key, image_xor_key, image_seed)
}

pub fn export_media(
    messages: &mut [ExportMessage],
    index: &MediaIndex,
    output_dir: &Path,
    image_key: Option<[u8; 16]>,
    account_id: &str,
    debug: bool,
    conversation_id: &str,
    voice: Option<&super::voice::VoiceIndex>,
    mut progress: Option<&mut dyn FnMut(usize, usize, usize, usize)>,
) -> MediaOutcome {
    let (image_key, image_xor_key, image_seed) = resolve_image_keys(index, account_id, image_key);
    let emoticon_key = image_seed.and_then(|seed| emoticon_key_from_seed(seed, account_id));
    let media_dir = output_dir.join("media");
    let message_total = messages.len();
    let attachment_total = messages.iter().filter(|message| message.media.is_some()).count();
    let mut attachment_done = 0usize;
    let mut outcome = MediaOutcome {
        saved: 0,
        failed: 0,
        failures: Vec::new(),
    };
    for (message_index, message) in messages.iter_mut().enumerate() {
        if let Some(callback) = progress.as_deref_mut() {
            callback(message_index, message_total, attachment_done, attachment_total);
        }
        let Some(media) = message.media.clone() else {
            continue;
        };
        attachment_done += 1;
        let folder = media.kind.as_str();
        let is_target_media = matches!(
            media.kind,
            MediaKind::Image | MediaKind::Emoji | MediaKind::File | MediaKind::Voice
        );
        if debug && is_target_media {
            eprintln!("\n--------------------------------------------------------------------------------");
            eprintln!(
                "[DEBUG] 消息 #{:04} [{}] (LocalId: {}, 发送者: {})",
                message.index, message.type_name, message.local_id, message.sender_name
            );
            eprintln!("  [1] 原始属性与哈希:");
            if !media.md5.is_empty() {
                eprintln!("      • 消息主哈希 (XML MD5): {}", media.md5);
            }
            if !media.candidate_md5s.is_empty() {
                eprintln!("      • 候选哈希列表 (原图/中图/缩略图): {:?}", media.candidate_md5s);
            }
            if let Some(url) = media.cdn_url.as_deref() {
                eprintln!("      • 远端 CDN 链接: {}", url);
            }
            if let Some(key) = media.aes_key {
                eprintln!("      • 消息独立 AES Key: {}", hex::encode(key));
            }
        }
        if media.kind == MediaKind::Voice {
            let payload = voice.and_then(|index| {
                index.extract(
                    message.server_id,
                    message.local_id,
                    conversation_id,
                    message.create_time,
                )
            });
            if let Some(payload) = payload {
                use sha2::{Digest, Sha256};
                let filename = if media.kind == MediaKind::File && !media.file_name.is_empty() {
                    crate::util::sanitize_file_name(&media.file_name)
                } else {
                    let digest = hex::encode(Sha256::digest(&payload.bytes));
                    format!("{digest}.{}", payload.ext)
                };
                let dest = media_dir.join(folder).join(&filename);
                if write_bytes(&dest, &payload.bytes).is_ok() {
                    if debug {
                        eprintln!(
                            "      • 语音库命中: {} 字节 -> media/{}/{}",
                            payload.bytes.len(),
                            folder,
                            filename
                        );
                    }
                    let relative = dest
                        .strip_prefix(output_dir)
                        .unwrap_or(dest.as_path())
                        .to_string_lossy()
                        .replace('\\', "/");
                    message.media_path = Some(relative);
                    outcome.saved += 1;
                    continue;
                }
            } else if debug {
                eprintln!("      • 语音库未命中：尝试本地文件与 CDN");
            }
        }

        let candidates = index.candidates(&media);
        if candidates.is_empty() {
            if debug && is_target_media {
                eprintln!("  [2] 本地寻址: 未在本地数据库/附件缓存中命中对应文件");
            }
            if let Some(url) = media.cdn_url.as_deref() {
                if debug && is_target_media {
                    eprintln!("  [3] CDN 远程补齐: 发起网络请求拉取: {}", url);
                }
                if let Some((mut data, mut ext)) = download_from_cdn(
                    url,
                    media.aes_key.as_ref(),
                    debug,
                ) {
                    if ext == "hevc" || data.starts_with(b"wxgf") {
                        if let Some((img_bytes, img_ext)) = transcode_wxgf_to_image(&data) {
                            data = img_bytes;
                            ext = img_ext;
                        }
                    }
                    let filename = if media.kind == MediaKind::File && !media.file_name.is_empty() {
                        crate::util::sanitize_file_name(&media.file_name)
                    } else {
                        use sha2::{Digest, Sha256};
                        let digest = hex::encode(Sha256::digest(&data));
                        format!("{digest}.{ext}")
                    };
                    let dest = media_dir.join(folder).join(&filename);
                    if write_bytes(&dest, &data).is_ok() {
                        if debug && is_target_media {
                            eprintln!("      • 远程补齐成功: 拉取 {} 字节，落盘至 {}", data.len(), dest.display());
                        }
                        let relative = dest
                            .strip_prefix(output_dir)
                            .unwrap_or(dest.as_path())
                            .to_string_lossy()
                            .replace('\\', "/");
                        message.media_path = Some(relative);
                        outcome.saved += 1;
                        continue;
                    }
                }
            }
            outcome.failed += 1;
            outcome.failures.push(format!(
                "message {}: media not found (md5={}, name={})",
                message.index, media.md5, media.file_name
            ));
            continue;
        }

        // Try every candidate, best first. A candidate is rejected when its decoded
        // pixels or byte length are smaller than what the message declares, which is
        // how a stale Thumb loses to an original living in cache/.../Emoticon/.
        let mut selected: Option<(PathBuf, bool, Vec<u8>, &'static str)> = None;
        let mut fallback: Option<(PathBuf, bool, Vec<u8>, &'static str, String)> = None;
        let mut tried: Vec<String> = Vec::new();
        for &ci in &candidates {
            let cand = &index.files[ci];
            let cand_is_dat = cand
                .path
                .extension()
                .and_then(|value| value.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("dat"))
                .unwrap_or(false);
            let raw = fs::read(&cand.path).unwrap_or_default();
            let Some((mut bytes, mut ext)) =
                decrypt_candidate_bytes(&raw, &cand.path, &media, cand_is_dat, image_key, image_xor_key, emoticon_key)
            else {
                tried.push(format!("{} (无法解密)", cand.path.display()));
                continue;
            };

            // Thumb -> full original upgrade by plaintext SHA-256. WeChat may keep the
            // full asset under business/emoticon/Persist/<sha[0..2]>/<sha256>.
            if media.kind == MediaKind::Emoji {
                use sha2::{Digest, Sha256};
                let digest = hex::encode(Sha256::digest(&bytes));
                if let Some(persist_path) = index.find_persist_by_sha256(&digest) {
                    let persist_bytes = fs::read(&persist_path).unwrap_or_default();
                    for key in [media.aes_key.as_ref(), emoticon_key.as_ref()]
                        .into_iter()
                        .flatten()
                    {
                        if let Some((pbytes, pext)) = decrypt_emoticon_bytes(&persist_bytes, key) {
                            if pbytes != bytes {
                                if debug {
                                    eprintln!(
                                        "      • 命中 Persist 原图: {} ({} 字节 -> {} 字节)",
                                        persist_path.display(),
                                        bytes.len(),
                                        pbytes.len()
                                    );
                                }
                                bytes = pbytes;
                                ext = pext;
                            }
                            break;
                        }
                    }
                }
            }

            // Transcode wxgf/HEVC here so a blank decode can be rejected before the
            // candidate is accepted (and before it can win the fallback slot).
            let Some((bytes, ext)) = finalize_candidate(bytes, ext) else {
                tried.push(format!("{} (空白或无法解码)", cand.path.display()));
                if debug {
                    eprintln!(
                        "      • 候选 {} 解码为空白图，跳过",
                        cand.path.display()
                    );
                }
                continue;
            };

            // An unrecognised payload (ext "bin") is not a usable image: skip it so a
            // real preview or the CDN original can win instead of writing garbage.
            if media.kind != MediaKind::File && ext == "bin" {
                tried.push(format!("{} (无法识别为图片)", cand.path.display()));
                if debug {
                    eprintln!("      • 候选 {} 解密后不是已知图片格式，跳过", cand.path.display());
                }
                continue;
            }

            if let Some(reason) = looks_like_thumbnail(&bytes, &media) {
                if debug {
                    eprintln!(
                        "      • 候选 {} 判定为缩略图: {}",
                        cand.path.display(),
                        reason
                    );
                }
                let better = fallback
                    .as_ref()
                    .map(|(existing_path, _, existing, _, _)| {
                        candidate_quality(&bytes, &media, &cand.path)
                            > candidate_quality(existing, &media, existing_path)
                    })
                    .unwrap_or(true);
                if better {
                    fallback = Some((cand.path.clone(), cand_is_dat, bytes, ext, reason));
                }
                continue;
            }

            selected = Some((cand.path.clone(), cand_is_dat, bytes, ext));
            break;
        }

        let mut used_fallback = false;
        let (selected_path, is_dat, mut decrypted) = match selected {
            Some((path, is_dat, bytes, ext)) => (path, is_dat, Some((bytes, ext))),
            None => match fallback {
                Some((path, is_dat, bytes, ext, reason)) => {
                    if debug {
                        eprintln!(
                            "      • 所有候选均小于声明值，保留其中最大者: {} ({})",
                            path.display(),
                            reason
                        );
                    }
                    used_fallback = true;
                    (path, is_dat, Some((bytes, ext)))
                }
                None => {
                    // Every local candidate was blank or undecodable: try the CDN.
                    let mut recovered = false;
                    if let Some(url) = media.cdn_url.as_deref() {
                        if debug {
                            eprintln!("  [3] 本地全部候选解码失败，改用 CDN 拉取: {}", url);
                        }
                        if let Some((mut data, mut ext)) =
                            download_from_cdn(
                                    url,
                                    media.aes_key.as_ref(),
                                    debug,
                                )
                        {
                            if ext == "hevc" || data.starts_with(b"wxgf") {
                                if let Some((img_bytes, img_ext)) = transcode_wxgf_to_image(&data) {
                                    data = img_bytes;
                                    ext = img_ext;
                                } else {
                                    data.clear();
                                }
                            }
                            if !data.is_empty()
                                && (media.kind == MediaKind::File || ext != "bin")
                            {
                                use sha2::{Digest, Sha256};
                                let digest = hex::encode(Sha256::digest(&data));
                                let dest = media_dir
                                    .join(folder)
                                    .join(format!("{digest}.{ext}"));
                                if write_bytes(&dest, &data).is_ok() {
                                    let relative = dest
                                        .strip_prefix(output_dir)
                                        .unwrap_or(dest.as_path())
                                        .to_string_lossy()
                                        .replace('\\', "/");
                                    message.media_path = Some(relative);
                                    outcome.saved += 1;
                                    recovered = true;
                                }
                            }
                        }
                    }
                    if recovered {
                        continue;
                    }
                    outcome.failed += 1;
                    outcome.failures.push(format!(
                        "message {}: media candidates unreadable [{}]",
                        message.index,
                        tried.join("; ")
                    ));
                    continue;
                }
            },
        };
        let raw_bytes = fs::read(&selected_path).unwrap_or_default();
        let file = IndexedFile {
            path: selected_path.clone(),
        };
        if debug && is_target_media {
            let meta_size = fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0);
            let dims = decrypted
                .as_ref()
                .and_then(|(bytes, _)| image_dimensions(bytes))
                .map(|(w, h)| format!("{w}x{h}"))
                .unwrap_or_else(|| "未知".to_string());
            eprintln!("  [2] 本地寻址命中:");
            eprintln!("      • 命中路径: {}", file.path.display());
            eprintln!("      • 文件大小: {} 字节 ({:.1} KB)", meta_size, meta_size as f64 / 1024.0);
            eprintln!("      • 解码尺寸: {} (声明 {:?}x{:?})", dims, media.width, media.height);
        }

        if debug && is_target_media {
            eprintln!("  [3] 解密与转码:");
            if media.kind == MediaKind::Emoji {
                eprintln!("      • 解密算法: 表情 AES-128-CBC (key==IV 由图片种子派生)");
            } else if is_dat {
                eprintln!("      • 解密算法: V2 .dat (AES-128-ECB 头部 + XOR 尾部, XOR: {:?})", image_xor_key);
            } else {
                eprintln!("      • 数据状态: 本地明文数据");
            }
        }

        // If decrypted payload is wxgf / hevc (including in chat images), transcode appropriately
        if let Some((bytes, ext)) = &mut decrypted {
            if *ext == "hevc" || bytes.starts_with(b"wxgf") {
                if debug && is_target_media {
                    eprintln!("      • 码流识别: 检测为微信 wxgf (HEVC 码流)，调用 ffmpeg 进行解码转码...");
                }
                if let Some((img_bytes, img_ext)) = transcode_wxgf_to_image(bytes) {
                    if debug && is_target_media {
                        eprintln!("      • 转码完成: 成功输出 {} 格式 ({} 字节)", img_ext, img_bytes.len());
                    }
                    *bytes = img_bytes;
                    *ext = img_ext;
                }
            }
        }

        let (ext, bytes_to_write): (String, Vec<u8>) = if let Some((bytes, ext)) = decrypted {
            (ext.to_string(), bytes)
        } else if is_image_header(&raw_bytes) {
            (detect_image_ext(&raw_bytes).to_string(), raw_bytes)
        } else if media.kind == MediaKind::File {
            let ext = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("bin")
                .to_string();
            (ext, raw_bytes)
        } else {
            outcome.failed += 1;
            outcome.failures.push(format!(
                "message {}: media decryption failed and not a plain image ({})",
                message.index, file.path.display()
            ));
            continue;
        };

        // Alpha keeps the byte-exact original from the CDN. When the local payload is a
        // preview (shorter than the declared length), replace it before writing so no
        // orphan preview is left on disk and the message points at the real file.
        let mut bytes_to_write = bytes_to_write;
        let mut ext = ext;
        let path_lower = file.path.to_string_lossy().to_ascii_lowercase();
        let local_is_thumb_path = path_lower.contains("thumb")
            || path_lower.contains("bubble")
            || path_lower.ends_with("_t.dat")
            || path_lower.ends_with("_b.dat")
            || path_lower.contains("/persist/")
            || path_lower.contains("\\persist\\");
        let declared = media.declared_size.unwrap_or(0);
        let local_len = bytes_to_write.len() as u64;
        let size_mismatch = declared_length_mismatch(declared, local_len);
        let local_is_still = is_still_extension(&ext) && declared > local_len;
        if (local_is_thumb_path || used_fallback || size_mismatch || local_is_still)
            && media.kind != MediaKind::File
        {
            if let Some(url) = media.cdn_url.as_deref() {
                if let Some((mut remote, mut remote_ext)) = download_from_cdn(
                    url,
                    media.aes_key.as_ref(),
                    debug,
                ) {
                    if remote_ext == "hevc" || remote.starts_with(b"wxgf") {
                        match transcode_wxgf_to_image(&remote) {
                            Some((img, img_ext)) => {
                                remote = img;
                                remote_ext = img_ext;
                            }
                            None => remote.clear(),
                        }
                    }
                    if !remote.is_empty() && remote_ext != "bin" {
                        if prefer_remote(
                            &bytes_to_write,
                            &ext,
                            &remote,
                            remote_ext,
                            declared,
                        ) {
                            if debug && is_target_media {
                                eprintln!(
                                    "  [+] CDN 原图替换本地预览: {} 字节 -> {} 字节 ({})",
                                    bytes_to_write.len(),
                                    remote.len(),
                                    remote_ext
                                );
                            }
                            bytes_to_write = remote;
                            ext = remote_ext.to_string();
                        }
                    }
                }
            }
        }

        let filename = if media.kind == MediaKind::File && !media.file_name.is_empty() {
            crate::util::sanitize_file_name(&media.file_name)
        } else {
            use sha2::{Digest, Sha256};
            let digest = hex::encode(Sha256::digest(&bytes_to_write));
            format!("{digest}.{ext}")
        };

        let dest = media_dir.join(folder).join(&filename);
        let existed = dest.exists();
        let result = if existed {
            Ok(dest.clone())
        } else {
            write_bytes(&dest, &bytes_to_write).map(|_| dest.clone())
        };
        if debug && is_target_media {
            eprintln!("  [4] 导出落盘:");
            eprintln!("      • 内容寻址 SHA-256: {}", &filename[..filename.find('.').unwrap_or(filename.len())]);
            eprintln!("      • 输出相对路径: media/{}/{}", folder, filename);
            eprintln!("      • 写入状态: {}", if existed { "已存在相同内容，直接复用 (自动去重)" } else { "写入新文件" });
        }
        match result {
            Ok(dest) => {
                let relative = dest
                    .strip_prefix(output_dir)
                    .unwrap_or(dest.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                message.media_path = Some(relative);
                outcome.saved += 1;
            }
            Err(error) => {
                outcome.failed += 1;
                outcome.failures.push(format!(
                    "message {}: {} ({})",
                    message.index,
                    error,
                    file.path.display()
                ));
            }
        }
    }
    outcome
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn decrypt_aes_cbc_key_iv(data: &[u8], key: &[u8; 16]) -> Option<Vec<u8>> {
    if data.len() < 16 || data.len() % 16 != 0 {
        return None;
    }
    let cipher = Aes128::new_from_slice(key).ok()?;
    let mut previous = *key;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut current = [0u8; 16];
        current.copy_from_slice(chunk);
        let mut block = current;
        let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(&mut block);
        cipher.decrypt_block(generic);
        for i in 0..16 {
            block[i] ^= previous[i];
        }
        previous = current;
        out.extend_from_slice(&block);
    }
    let pad = *out.last()? as usize;
    if pad == 0 || pad > 16 || pad > out.len() {
        return None;
    }
    out.truncate(out.len() - pad);
    Some(out)
}

static CDN_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn download_from_cdn(
    url: &str,
    aes_key: Option<&[u8; 16]>,
    debug: bool,
) -> Option<(Vec<u8>, &'static str)> {
    let request = ureq::get(url)
        .set("User-Agent", "MicroMessenger Client")
        .timeout(std::time::Duration::from_secs(60));
    let resp = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let error_code = response.header("x-error-code").unwrap_or("");
            let error_message = response.header("x-error-message").unwrap_or("");
            if debug {
                eprintln!("      • CDN 请求失败: HTTP {code} {error_code} {error_message}");
            } else if !CDN_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("[!] 腾讯 CDN 拒绝请求: HTTP {code} {error_code}");
            }
            return None;
        }
        Err(error) => {
            if debug {
                eprintln!("      • CDN 请求失败: {error}");
            }
            return None;
        }
    };
    let mut reader = resp.into_reader();
    let mut data = Vec::new();
    use std::io::Read;
    reader.read_to_end(&mut data).ok()?;
    if data.is_empty() {
        return None;
    }
    if !is_image_header(&data) {
        if let Some(key) = aes_key {
            if let Some(decrypted) = decrypt_aes_cbc_key_iv(&data, key) {
                data = decrypted;
            }
        }
    }
    let mut ext = detect_image_ext(&data);
    if ext == "hevc" || data.starts_with(b"wxgf") {
        if let Some((img_bytes, img_ext)) = transcode_wxgf_to_image(&data) {
            data = img_bytes;
            ext = img_ext;
        }
    }
    Some((data, ext))
}

/// Pixel dimensions of a decoded image, used to tell an original from a thumbnail.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    if bytes.starts_with(b"wxgf") && bytes.len() >= 12 {
        let w = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        let h = u16::from_le_bytes([bytes[10], bytes[11]]) as u32;
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if seg_len < 2 {
                break;
            }
            let is_sof = matches!(
                marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
            );
            if is_sof {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
            i += 2 + seg_len;
        }
    }
    if bytes.starts_with(b"RIFF") && bytes.len() >= 30 && &bytes[8..12] == b"WEBP" {
        match &bytes[12..16] {
            b"VP8X" => {
                let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
                let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
                return Some((w, h));
            }
            b"VP8 " => {
                let w = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
                let h = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
            b"VP8L" => {
                let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                return Some((w, h));
            }
            _ => {}
        }
    }
    None
}

/// Decide whether the decrypted bytes are a lower-resolution stand-in for the
/// asset the message describes. Uses declared pixels first, then declared bytes.
fn looks_like_thumbnail(bytes: &[u8], media: &super::model::MediaRef) -> Option<String> {
    if media.kind == MediaKind::File {
        return None;
    }
    // Pixel size is authoritative. Byte length is not: WeChat re-encodes originals to
    // JPEG/WEBP, so a genuine original is often smaller than the declared GIF length.
    if let Some((w, h)) = image_dimensions(bytes) {
        if let (Some(dw), Some(dh)) = (media.width, media.height) {
            if w * 10 < dw * 9 || h * 10 < dh * 9 {
                return Some(format!("实际 {w}x{h} < 声明 {dw}x{dh}"));
            }
        }
        return None;
    }
    // Dimensions unavailable (corrupt/unsupported header): only then fall back to size.
    if let Some(declared) = media.declared_size {
        if declared > 0 && (bytes.len() as u64) * 100 < declared * 90 {
            return Some(format!(
                "实际 {} 字节 < 声明 {} 字节",
                bytes.len(),
                declared
            ));
        }
    }
    None
}

/// Ranking key for a decoded candidate when no candidate reaches the declared size:
/// biggest pixels first, then closest to the declared byte length, then raw size.
/// Alpha's originals are byte-exact with the length declared in the message XML.
/// A local plaintext even 1% smaller is a re-compressed preview, not the original.
/// Should the remote payload replace the local one?
fn prefer_remote(
    local: &[u8],
    local_ext: &str,
    remote: &[u8],
    remote_ext: &str,
    declared: u64,
) -> bool {
    let remote_len = remote.len() as u64;
    // Alpha's originals are byte-exact with the declared length.
    let exact_original = declared > 0
        && remote_len + declared / 100 >= declared
        && remote_len <= declared * 2;
    let local_area = image_dimensions(local)
        .map(|(w, h)| w as u64 * h as u64)
        .unwrap_or(0);
    let remote_area = image_dimensions(remote)
        .map(|(w, h)| w as u64 * h as u64)
        .unwrap_or(0);
    exact_original
        || (remote_ext == "gif" && local_ext != "gif")
        || remote_area > local_area
}

fn declared_length_mismatch(declared: u64, local_len: u64) -> bool {
    declared > 0 && local_len + declared / 100 < declared
}

fn is_still_extension(ext: &str) -> bool {
    matches!(ext, "jpg" | "jpeg" | "png" | "webp")
}

fn candidate_quality(
    bytes: &[u8],
    media: &super::model::MediaRef,
    path: &Path,
) -> (i32, u64, i64, usize) {
    let path_lower = path.to_string_lossy().to_ascii_lowercase();
    let dir_rank = if path_lower.contains("thumb")
        || path_lower.contains("bubble")
        || path_lower.ends_with("_t.dat")
        || path_lower.ends_with("_b.dat")
    {
        -1
    } else if path_lower.contains("persist") {
        2
    } else {
        0
    };
    let area = image_dimensions(bytes)
        .map(|(w, h)| w as u64 * h as u64)
        .unwrap_or(0);
    let declared = media.declared_size.unwrap_or(0) as i64;
    let distance = if declared > 0 {
        (bytes.len() as i64 - declared).abs()
    } else {
        0
    };
    (dir_rank, area, -distance, bytes.len())
}

/// Decrypt one on-disk candidate with the same key ladder used for the final export.
fn decrypt_candidate_bytes(
    raw: &[u8],
    path: &Path,
    media: &super::model::MediaRef,
    is_dat: bool,
    image_key: Option<[u8; 16]>,
    image_xor_key: Option<u8>,
    emoticon_key: Option<[u8; 16]>,
) -> Option<(Vec<u8>, &'static str)> {
    // Chat files (PDF/ZIP/APK...) are stored as plaintext once received.
    if media.kind == MediaKind::File && !raw.is_empty() {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let ext = match ext.as_str() {
            "pdf" => "pdf",
            "apk" => "apk",
            "zip" => "zip",
            "rar" => "rar",
            "7z" => "7z",
            "doc" => "doc",
            "docx" => "docx",
            "xls" => "xls",
            "xlsx" => "xlsx",
            "ppt" => "ppt",
            "pptx" => "pptx",
            "txt" => "txt",
            "html" => "html",
            "mp4" => "mp4",
            "mp3" => "mp3",
            "wav" => "wav",
            _ => "bin",
        };
        return Some((raw.to_vec(), ext));
    }
    for key in [media.aes_key.as_ref(), emoticon_key.as_ref()]
        .into_iter()
        .flatten()
    {
        if media.kind == MediaKind::Emoji || !is_dat {
            if let Some((bytes, ext)) = decrypt_emoticon_bytes(raw, key) {
                return Some((bytes, ext));
            }
        }
    }
    if is_dat {
        if let Ok((bytes, ext)) = decrypt_dat_file(path, image_key, image_xor_key) {
            return Some((bytes, ext));
        }
    }
    if is_image_header(raw) {
        return Some((raw.to_vec(), detect_image_ext(raw)));
    }
    None
}

fn detect_image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "png"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"wxgf") {
        "hevc"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "tif"
    } else {
        "bin"
    }
}

fn is_image_header(bytes: &[u8]) -> bool {
    detect_image_ext(bytes) != "bin"
}

/// Seed validation only accepts JPEG/PNG. GIF/WEBP/BMP/wxgf first bytes are
/// common enough that a random AES key decrypts some ciphertext into them.
fn is_strict_image_header(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47])
}

pub(crate) fn find_ffmpeg() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("WECHAT_FFMPEG_PATH").or_else(|_| std::env::var("FFMPEG_PATH")) {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join("ffmpeg.exe"), dir.join("ffmpeg")] {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from("ffmpeg"));
    }
    None
}

fn detect_wxgf_fps(data: &[u8]) -> u32 {
    if data.len() >= 24 && data.starts_with(b"wxgf") {
        for &offset in &[19, 23, 20, 22] {
            if offset < data.len() {
                let dur = data[offset] as u32;
                if (40..=300).contains(&dur) {
                    let fps = 1000 / dur;
                    if (2..=25).contains(&fps) {
                        return fps;
                    }
                }
            }
        }
    }
    10
}

/// Split a wxgf HEVC payload into independent streams. WeChat stores HEVC-with-alpha
/// as two concatenated streams; decoding both at once yields the alpha plane (white).
fn hevc_segments(payload: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<(usize, u8)> = Vec::new();
    let mut i = 0usize;
    while i + 3 < payload.len() {
        if payload[i] == 0 && payload[i + 1] == 0 {
            let (header, consumed) = if payload[i + 2] == 1 {
                (i + 3, 3)
            } else if i + 3 < payload.len() && payload[i + 2] == 0 && payload[i + 3] == 1 {
                (i + 4, 4)
            } else {
                i += 1;
                continue;
            };
            if header < payload.len() {
                starts.push((i, (payload[header] >> 1) & 0x3F));
            }
            let _ = consumed;
            i = header + 1;
            continue;
        }
        i += 1;
    }
    let vps: Vec<usize> = starts
        .iter()
        .filter(|(_, nal_type)| *nal_type == 32)
        .map(|(pos, _)| *pos)
        .collect();
    if vps.len() <= 1 {
        return vec![payload];
    }
    let mut segments = Vec::new();
    for k in 0..vps.len() {
        let end = vps.get(k + 1).copied().unwrap_or(payload.len());
        segments.push(&payload[vps[k]..end]);
    }
    segments
}

/// True when the decoded frame is essentially uniform white, i.e. we picked an
/// alpha/monochrome plane instead of the colour stream.
fn probe_frame_is_white(ffmpeg: &Path, input: &Path) -> Option<bool> {
    let out = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "hevc",
            "-i",
            input.to_str()?,
            "-frames:v",
            "1",
            "-vf",
            "scale=8:8:flags=area",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() < 8 * 8 * 3 {
        return None;
    }
    let pixels = &out.stdout[..8 * 8 * 3];
    let count = (pixels.len() / 3) as u64;
    let luma: Vec<i64> = pixels
        .chunks_exact(3)
        .map(|p| (p[0] as i64 + p[1] as i64 + p[2] as i64) / 3)
        .collect();
    let avg = luma.iter().sum::<i64>() / count as i64;
    let var = luma
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<i64>()
        / count as i64;
    Some(avg >= 250 && var <= 4)
}

/// Normalise a decoded candidate: transcode wxgf/HEVC to a viewable image and
/// reject blank (white) decodes so the next candidate can be tried.
fn finalize_candidate(bytes: Vec<u8>, ext: &'static str) -> Option<(Vec<u8>, &'static str)> {
    if ext == "hevc" || bytes.starts_with(b"wxgf") {
        return transcode_wxgf_to_image(&bytes);
    }
    Some((bytes, ext))
}

fn transcode_wxgf_to_image(data: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let ffmpeg = find_ffmpeg()?;
    let nal_offset = data
        .windows(4)
        .position(|w| w == [0, 0, 0, 1])
        .unwrap_or(26);
    if nal_offset >= data.len() {
        return None;
    }
    let payload = &data[nal_offset..];
    let frames = if data.len() >= 14 && data.starts_with(b"wxgf") {
        u16::from_le_bytes([data[12], data[13]])
    } else {
        0
    };
    let temp_dir = tempfile::TempDir::new().ok()?;

    // Pick the colour stream: HEVC-with-alpha concatenates two streams, and decoding
    // them together (or picking the alpha plane) yields an all-white frame.
    let mut chosen: Option<PathBuf> = None;
    for (index, segment) in hevc_segments(payload).into_iter().enumerate() {
        let path = temp_dir.path().join(format!("segment{index}.hevc"));
        if fs::write(&path, segment).is_err() {
            continue;
        }
        match probe_frame_is_white(&ffmpeg, &path) {
            Some(true) => continue,
            Some(false) => {
                chosen = Some(path);
                break;
            }
            None => {
                if chosen.is_none() {
                    chosen = Some(path);
                }
            }
        }
    }
    let in_path = chosen?;

    if frames > 1 {
        let fps = detect_wxgf_fps(data).to_string();
        let out_path = temp_dir.path().join("output.gif");
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-y",
                "-r",
                &fps,
                "-f",
                "hevc",
                "-i",
                in_path.to_str()?,
                "-r",
                &fps,
                out_path.to_str()?,
            ])
            .status()
            .ok()?;
        if status.success() && out_path.exists() {
            return Some((fs::read(out_path).ok()?, "gif"));
        }
    } else {
        let out_path = temp_dir.path().join("output.jpg");
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-y",
                "-f",
                "hevc",
                "-i",
                in_path.to_str()?,
                "-frames:v",
                "1",
                "-q:v",
                "2",
                out_path.to_str()?,
            ])
            .status()
            .ok()?;
        if status.success() && out_path.exists() {
            return Some((fs::read(out_path).ok()?, "jpg"));
        }
    }
    None
}
/// AES-128-CBC (key == IV) + PKCS7 for WeChat emoticon `Thumb` / `Persist`
/// files, matching the account seed derivation.
fn decrypt_emoticon_bytes(data: &[u8], key: &[u8; 16]) -> Option<(Vec<u8>, &'static str)> {
    if is_image_header(data) {
        return Some((data.to_vec(), detect_image_ext(data)));
    }
    if data.len() < 16 || data.len() % 16 != 0 {
        return None;
    }
    let cipher = Aes128::new_from_slice(key).ok()?;
    let mut previous = *key;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut current = [0u8; 16];
        current.copy_from_slice(chunk);
        let mut block = current;
        let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(&mut block);
        cipher.decrypt_block(generic);
        for i in 0..16 {
            block[i] ^= previous[i];
        }
        previous = current;
        out.extend_from_slice(&block);
    }
    let pad = *out.last()? as usize;
    if pad == 0 || pad > 16 || pad > out.len() {
        return None;
    }
    out.truncate(out.len() - pad);
    if is_image_header(&out) {
        let ext = detect_image_ext(&out);
        if ext == "hevc" {
            if let Some((img_bytes, img_ext)) = transcode_wxgf_to_image(&out) {
                return Some((img_bytes, img_ext));
            }
            static FFMPEG_WARNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !FFMPEG_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("[!] 发现微信动画表情(HEVC/wxgf)，未在系统 PATH 或程序同级目录检测到 ffmpeg。如需自动转为 GIF 动图，请安装 ffmpeg 并加入系统 PATH。");
            }
        }
        Some((out, ext))
    } else {
        None
    }
}

fn decrypt_dat_file(
    path: &Path,
    image_key: Option<[u8; 16]>,
    xor_key: Option<u8>,
) -> Result<(Vec<u8>, &'static str)> {
    let data = fs::read(path)?;
    if is_image_header(&data) {
        return Ok((data.clone(), detect_image_ext(&data)));
    }
    let head = &data[..data.len().min(6)];
    if head == V1_MAGIC || head == V2_MAGIC {
        let key = if head == V1_MAGIC {
            *b"cfcd208495d565ef"
        } else {
            image_key.ok_or_else(|| {
                anyhow::anyhow!("V2 image encryption requires --image-key (32 hex characters)")
            })?
        };
        let decrypted = decrypt_dat_aes_xor(&data, &key, xor_key)?;
        let ext = detect_image_ext(&decrypted);
        return Ok((decrypted, ext));
    }
    if let Some(key) = detect_xor_key(&data) {
        let decrypted: Vec<u8> = data.iter().map(|byte| byte ^ key).collect();
        let ext = detect_image_ext(&decrypted);
        return Ok((decrypted, ext));
    }
    bail!("unsupported .dat encryption format")
}

fn decrypt_dat_aes_xor(data: &[u8], key: &[u8; 16], xor_key: Option<u8>) -> Result<Vec<u8>> {
    if data.len() < 15 {
        bail!("truncated .dat header");
    }
    let aes_size = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
    let xor_size = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    let aligned_aes_size = ((aes_size / 16) + 1) * 16;
    let offset = 15usize;
    let aes_end = offset
        .checked_add(aligned_aes_size)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| anyhow::anyhow!("invalid .dat AES payload length"))?;
    let cipher = Aes128::new_from_slice(key).expect("AES-128 key");
    let mut block = data[offset..aes_end].to_vec();
    if block.len() % 16 != 0 {
        bail!("invalid .dat AES block length");
    }
    for chunk in block.chunks_exact_mut(16) {
        let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(generic);
    }
    // The header explicitly declares the exact plaintext aes_size; truncate directly to it
    if aes_size <= block.len() {
        block.truncate(aes_size);
    } else {
        let pad = *block.last().unwrap_or(&0) as usize;
        if pad > 0 && pad <= 16 && pad <= block.len() {
            block.truncate(block.len() - pad);
        }
    }

    let raw_end = data
        .len()
        .checked_sub(xor_size)
        .filter(|end| *end >= aes_end)
        .ok_or_else(|| anyhow::anyhow!("invalid .dat XOR length"))?;
    let raw = &data[aes_end..raw_end];
    let xor = &data[raw_end..];

    // Alpha derives this from the account seed (`seed & 0xff`); when the
    // seed is unavailable fall back to the JPEG tail heuristic.
    let xor_key = xor_key.unwrap_or_else(|| {
        if xor.len() >= 2 {
            let k1 = xor[xor.len() - 2] ^ 0xFF;
            let k2 = xor[xor.len() - 1] ^ 0xD9;
            if k1 == k2 {
                return k1;
            }
        }
        if let Some((_, _, k)) = *IMAGE_SEED_CACHE.lock().unwrap() {
            return k;
        }
        0x14
    });

    let mut out = Vec::with_capacity(block.len() + raw.len() + xor.len());
    out.extend_from_slice(&block);
    out.extend_from_slice(raw);
    out.extend(xor.iter().map(|byte| byte ^ xor_key));
    Ok(out)
}

fn detect_xor_key(data: &[u8]) -> Option<u8> {
    const MAGICS: &[&[u8]] = &[
        &[0xFF, 0xD8, 0xFF],
        &[0x89, 0x50, 0x4E, 0x47],
        b"GIF8",
        b"RIFF",
        b"BM",
    ];
    if data.len() < 8 {
        return None;
    }
    for magic in MAGICS {
        let key = data[0] ^ magic[0];
        if magic.iter().enumerate().all(|(index, byte)| data[index] ^ key == *byte) {
            return Some(key);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkcs7(data: &[u8]) -> Vec<u8> {
        let pad = 16 - (data.len() % 16);
        let mut out = data.to_vec();
        out.extend(std::iter::repeat(pad as u8).take(pad));
        out
    }

    fn encrypt_v1(png: &[u8]) -> Vec<u8> {
        use aes::cipher::BlockEncrypt;
        let key = b"cfcd208495d565ef";
        let cipher = Aes128::new_from_slice(key).unwrap();
        let mut block = pkcs7(png);
        for chunk in block.chunks_exact_mut(16) {
            let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(chunk);
            cipher.encrypt_block(generic);
        }
        let mut out = Vec::new();
        out.extend_from_slice(V1_MAGIC);
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&block);
        out
    }

    #[test]
    fn decrypts_v1_dat() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02];
        let encrypted = encrypt_v1(&png);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.dat");
        std::fs::write(&path, &encrypted).unwrap();
        let (bytes, ext) = decrypt_dat_file(&path, None, None).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(bytes, png);
    }

    #[test]
    fn decrypts_xor_dat() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let encrypted: Vec<u8> = png.iter().map(|byte| byte ^ 0x5A).collect();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.dat");
        std::fs::write(&path, encrypted).unwrap();
        let (bytes, ext) = decrypt_dat_file(&path, None, None).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(bytes, png);
    }

    #[test]
    fn v2_requires_key() {
        let mut data = Vec::new();
        data.extend_from_slice(V2_MAGIC);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.push(0);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.dat");
        std::fs::write(&path, &data).unwrap();
        assert!(decrypt_dat_file(&path, None, None).is_err());
    }

    #[test]
    fn decrypts_v2_dat_with_key() {
        use aes::cipher::BlockEncrypt;
        let key = [0x37u8; 16];
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xAA, 0xBB, 0xCC];
        let cipher = Aes128::new_from_slice(&key).unwrap();
        let mut block = pkcs7(&png);
        for chunk in block.chunks_exact_mut(16) {
            let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(chunk);
            cipher.encrypt_block(generic);
        }
        let tail = [1u8, 2, 3];
        let mut data = Vec::new();
        data.extend_from_slice(V2_MAGIC);
        data.extend_from_slice(&(png.len() as u32).to_le_bytes());
        data.extend_from_slice(&(tail.len() as u32).to_le_bytes());
        data.push(0);
        data.extend_from_slice(&block);
        data.extend(tail.iter().map(|byte| byte ^ 0x88));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.dat");
        std::fs::write(&path, &data).unwrap();
        let (bytes, ext) = decrypt_dat_file(&path, Some(key), Some(0x88)).unwrap();
        assert_eq!(ext, "png");
        let mut expected = png.to_vec();
        expected.extend_from_slice(&tail);
        assert_eq!(bytes, expected);
    }

    fn hevc_nal(nal_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8, 0, 0, 1, nal_type << 1, 1];
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn splits_concatenated_hevc_streams_at_second_vps() {
        let mut data = Vec::new();
        data.extend(hevc_nal(32, &[0xAA; 4])); // VPS 1
        data.extend(hevc_nal(33, &[0xBB; 4])); // SPS 1
        data.extend(hevc_nal(19, &[0xCC; 8])); // IDR 1
        data.extend(hevc_nal(32, &[0xDD; 4])); // VPS 2
        data.extend(hevc_nal(33, &[0xEE; 4])); // SPS 2
        data.extend(hevc_nal(19, &[0xFF; 8])); // IDR 2
        let segments = hevc_segments(&data);
        assert_eq!(segments.len(), 2);
        assert!(segments[0].starts_with(&[0, 0, 0, 1, 64]));
        assert!(segments[1].starts_with(&[0, 0, 0, 1, 64]));
        assert_eq!(segments[0].len() + segments[1].len(), data.len());

        // A single-stream payload must stay whole.
        let single: Vec<u8> = [hevc_nal(32, &[1, 2]), hevc_nal(19, &[3, 4])].concat();
        assert_eq!(hevc_segments(&single).len(), 1);
    }

    #[test]
    fn prefers_byte_exact_original_over_preview() {
        let declared = 29560u64;
        let preview = vec![0u8; 21509];
        let original = vec![0u8; 29560];
        assert!(prefer_remote(&preview, "jpg", &original, "jpg", declared));
        // A smaller, non-exact remote never wins on a non-animated local.
        assert!(!prefer_remote(&original, "jpg", &preview, "jpg", declared));
        // An animated original replaces a still preview of the same length band.
        assert!(prefer_remote(&preview, "jpg", &original, "gif", declared));
    }

    #[test]
    fn detects_preview_smaller_than_declared_length() {
        // Message 17: local preview 21509 bytes, XML declares 29560.
        assert!(declared_length_mismatch(29560, 21509));
        // Message 702: local 2013844 vs declared 3843963.
        assert!(declared_length_mismatch(3843963, 2013844));
        // Byte-exact original from the CDN.
        assert!(!declared_length_mismatch(90546, 90546));
        // No declared length: cannot judge.
        assert!(!declared_length_mismatch(0, 123));
        assert!(is_still_extension("jpg"));
        assert!(is_still_extension("png"));
        assert!(!is_still_extension("gif"));
    }

    #[test]
    fn keeps_original_with_matching_pixels_even_if_bytes_shrink() {
        let mut jpg_like = vec![0u8; 24];
        jpg_like[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        jpg_like[16..20].copy_from_slice(&416u32.to_be_bytes());
        jpg_like[20..24].copy_from_slice(&440u32.to_be_bytes());
        let media = super::super::model::MediaRef {
            md5: "a977c9bbc0cc67c2fd4f601f5219f16e".into(),
            candidate_md5s: Vec::new(),
            file_name: String::new(),
            kind: MediaKind::Emoji,
            cdn_url: None,
            aes_key: None,
            width: Some(416),
            height: Some(440),
            declared_size: Some(29560),
            duration_ms: None,
        };
        // 21509 < 29560 but pixels match: this is the original, not a thumbnail.
        assert!(looks_like_thumbnail(&jpg_like, &media).is_none());
    }

    #[test]
    fn ranks_candidates_by_pixels_before_bytes() {
        let media = super::super::model::MediaRef {
            md5: "b1fc57f4bf68c556fd0f30b417857e3a".into(),
            candidate_md5s: Vec::new(),
            file_name: String::new(),
            kind: MediaKind::Emoji,
            cdn_url: None,
            aes_key: None,
            width: Some(240),
            height: Some(240),
            declared_size: Some(20563),
            duration_ms: None,
        };
        let mut big_pixels = vec![0u8; 24];
        big_pixels[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        big_pixels[16..20].copy_from_slice(&240u32.to_be_bytes());
        big_pixels[20..24].copy_from_slice(&240u32.to_be_bytes());
        big_pixels.resize(5000, 0);

        let mut small_pixels = vec![0u8; 24];
        small_pixels[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        small_pixels[16..20].copy_from_slice(&160u32.to_be_bytes());
        small_pixels[20..24].copy_from_slice(&160u32.to_be_bytes());
        small_pixels.resize(20560, 0);

        let p = Path::new("x/business/emoticon/Persist/b1/aa");
        assert!(
            candidate_quality(&big_pixels, &media, p) > candidate_quality(&small_pixels, &media, p)
        );
    }

    #[test]
    fn copies_chat_file_bytes_without_decryption() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("report.pdf");
        let payload = b"%PDF-1.7 fake document".to_vec();
        std::fs::write(&path, &payload).unwrap();
        let media = super::super::model::MediaRef {
            md5: String::new(),
            candidate_md5s: Vec::new(),
            file_name: "report.pdf".into(),
            kind: MediaKind::File,
            cdn_url: None,
            aes_key: None,
            width: None,
            height: None,
            declared_size: None,
            duration_ms: None,
        };
        let (bytes, ext) =
            decrypt_candidate_bytes(&payload, &path, &media, false, None, None, None)
                .expect("file must be copied verbatim");
        assert_eq!(bytes, payload);
        assert_eq!(ext, "pdf");
    }

    #[test]
    fn parses_image_dimensions_from_headers() {
        // PNG 240x240
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&240u32.to_be_bytes());
        png[20..24].copy_from_slice(&240u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((240, 240)));

        // GIF 120x90
        let mut gif = vec![0u8; 10];
        gif[0..6].copy_from_slice(b"GIF89a");
        gif[6..8].copy_from_slice(&120u16.to_le_bytes());
        gif[8..10].copy_from_slice(&90u16.to_le_bytes());
        assert_eq!(image_dimensions(&gif), Some((120, 90)));

        // wxgf header carries width/height as u16 LE at 8..12
        let mut wxgf = vec![0u8; 32];
        wxgf[0..4].copy_from_slice(b"wxgf");
        wxgf[8..10].copy_from_slice(&160u16.to_le_bytes());
        wxgf[10..12].copy_from_slice(&160u16.to_le_bytes());
        assert_eq!(image_dimensions(&wxgf), Some((160, 160)));
    }

    #[test]
    fn flags_smaller_candidate_as_thumbnail() {
        let mut png = vec![0u8; 24];
        png[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        png[16..20].copy_from_slice(&120u32.to_be_bytes());
        png[20..24].copy_from_slice(&120u32.to_be_bytes());
        let media = super::super::model::MediaRef {
            md5: "b1fc57f4bf68c556fd0f30b417857e3a".into(),
            candidate_md5s: Vec::new(),
            file_name: String::new(),
            kind: MediaKind::Emoji,
            cdn_url: None,
            aes_key: None,
            width: Some(240),
            height: Some(240),
            declared_size: Some(90546),
            duration_ms: None,
        };
        assert!(looks_like_thumbnail(&png, &media).is_some());

        let mut full = vec![0u8; 90546];
        full[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        full[16..20].copy_from_slice(&240u32.to_be_bytes());
        full[20..24].copy_from_slice(&240u32.to_be_bytes());
        assert!(looks_like_thumbnail(&full, &media).is_none());
    }

    #[test]
    fn ranks_cache_original_above_thumb_by_declared_size() {
        let root = tempfile::TempDir::new().unwrap();
        let thumb_dir = root.path().join("business").join("emoticon").join("Thumb").join("b1");
        let cache_dir = root.path().join("cache").join("2026-09").join("Emoticon").join("b1");
        std::fs::create_dir_all(&thumb_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        let md5 = "b1fc57f4bf68c556fd0f30b417857e3a";
        let thumb = thumb_dir.join(format!("{md5}.thumb"));
        let original = cache_dir.join(md5);
        std::fs::write(&thumb, vec![0u8; 62128]).unwrap();
        std::fs::write(&original, vec![0u8; 90546]).unwrap();

        let index = MediaIndex::build(Some(root.path()));
        let media = super::super::model::MediaRef {
            md5: md5.to_string(),
            candidate_md5s: Vec::new(),
            file_name: String::new(),
            kind: MediaKind::Emoji,
            cdn_url: None,
            aes_key: None,
            width: Some(240),
            height: Some(240),
            declared_size: Some(90546),
            duration_ms: None,
        };
        let ranked = index.candidates(&media);
        let top = &index.files[ranked[0]].path;
        assert_eq!(top, &original, "candidate closest to declared size must win");
    }

    #[test]
    fn finds_full_emoticon_in_persist_by_plaintext_sha256() {
        let root = tempfile::TempDir::new().unwrap();
        let persist = root
            .path()
            .join("business")
            .join("emoticon")
            .join("Persist")
            .join("c2");
        std::fs::create_dir_all(&persist).unwrap();
        let sha = "c2a92741dae62b23cadd16c30fc053ac9043a37900e78f635bf41ebb9e3b965d";
        let full = persist.join(sha);
        std::fs::write(&full, b"full-emoticon-bytes").unwrap();

        let index = MediaIndex::build(Some(root.path()));
        assert_eq!(index.find_persist_by_sha256(sha), Some(full));
        assert_eq!(
            index.find_persist_by_sha256("00000000000000000000000000000000000000000000000000000000000000ff"),
            None
        );
    }

    #[test]
    fn prefers_persist_original_over_thumb_for_same_md5() {
        let root = tempfile::TempDir::new().unwrap();
        let thumb_dir = root
            .path()
            .join("business")
            .join("emoticon")
            .join("Thumb")
            .join("b1");
        let persist_dir = root
            .path()
            .join("business")
            .join("emoticon")
            .join("Persist")
            .join("b1");
        std::fs::create_dir_all(&thumb_dir).unwrap();
        std::fs::create_dir_all(&persist_dir).unwrap();
        let md5 = "b1fc57f4bf68c556fd0f30b417857e3a";
        let thumb = thumb_dir.join(format!("{md5}.thumb"));
        let full = persist_dir.join(md5);
        std::fs::write(&thumb, vec![0u8; 4096]).unwrap();
        std::fs::write(&full, vec![0u8; 90546]).unwrap();

        let index = MediaIndex::build(Some(root.path()));
        let media = super::super::model::MediaRef {
            md5: md5.to_string(),
            candidate_md5s: Vec::new(),
            file_name: String::new(),
            kind: MediaKind::Emoji,
            cdn_url: None,
            aes_key: None,
            width: None,
            height: None,
            declared_size: None,
            duration_ms: None,
        };
        let ranked = index.candidates(&media);
        let top = &index.files[ranked[0]].path;
        assert_eq!(top, &full, "Persist original must beat Thumb");
    }

    #[test]
    fn derives_image_and_emoticon_keys_from_seed() {
        let wxid = "wxid_z2yief89nr2s22";
        let (key, xor_key) = image_key_from_seed(3_642_459_924, wxid);
        assert_eq!(&key, b"b866e5aa60cda50c");
        assert_eq!(xor_key, 0x14);
        let expected_emoticon: [u8; 16] = hex::decode("7995167f11b67c406b3e75f397cb5095")
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            emoticon_key_from_seed(3_642_459_924, wxid).unwrap(),
            expected_emoticon
        );
    }

    #[test]
    #[ignore]
    fn decrypts_real_emoticon_sample() {
        let Ok(path) = std::env::var("EMOTICON_SAMPLE") else {
            return;
        };
        let data = std::fs::read(path).expect("read sample");
        let key = emoticon_key_from_seed(3_642_459_924, "wxid_z2yief89nr2s22").unwrap();
        assert!(emoticon_first_block_is_image(&data, &key));
        let (bytes, ext) = decrypt_emoticon_bytes(&data, &key).expect("decrypt");
        eprintln!("real emoticon: {} bytes, ext={ext}", bytes.len());
        assert!(is_image_header(&bytes));
    }

    #[test]
    fn decrypts_emoticon_cbc() {
        use aes::cipher::BlockEncrypt;
        let key = [0x11u8; 16];
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
        let mut plain = pkcs7(&png);
        let cipher = Aes128::new_from_slice(&key).unwrap();
        let mut previous = key;
        for chunk in plain.chunks_exact_mut(16) {
            for i in 0..16 {
                chunk[i] ^= previous[i];
            }
            let generic = aes::cipher::generic_array::GenericArray::from_mut_slice(chunk);
            cipher.encrypt_block(generic);
            previous.copy_from_slice(chunk);
        }
        let (bytes, ext) = decrypt_emoticon_bytes(&plain, &key).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(bytes, png);
    }
}
