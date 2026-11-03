use aes::cipher::{BlockDecrypt, KeyInit};
use aes::Aes128;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{ExportMessage, MediaKind};

const V1_MAGIC: &[u8] = b"\x07\x08V1\x08\x07";
const V2_MAGIC: &[u8] = b"\x07\x08V2\x08\x07";

#[derive(Clone, Debug)]
struct IndexedFile {
    path: PathBuf,
    stem: String,
}

pub struct MediaIndex {
    files: Vec<IndexedFile>,
    by_name: HashMap<String, Vec<usize>>,
    by_md5: HashMap<String, Vec<usize>>,
}

fn is_md5_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl MediaIndex {
    pub fn build(account_root: Option<&Path>) -> MediaIndex {
        let mut index = MediaIndex {
            files: Vec::new(),
            by_name: HashMap::new(),
            by_md5: HashMap::new(),
        };
        if let Some(root) = account_root {
            let mut stack = vec![(root.to_path_buf(), 0usize)];
            while let Some((dir, depth)) = stack.pop() {
                if depth > 10 || index.files.len() > 300_000 {
                    continue;
                }
                let Ok(entries) = fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if entry.file_type().map(|kind| kind.is_symlink()).unwrap_or(true) {
                        continue;
                    }
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    if metadata.is_dir() {
                        if matches!(
                            name.as_str(),
                            "db_storage" | "cache" | "tmp" | "temp" | "backup" | "node_modules" | ".git"
                        ) {
                            continue;
                        }
                        stack.push((path, depth + 1));
                    } else if metadata.is_file() {
                        let lower_name = name.to_ascii_lowercase();
                        let stem = Path::new(&lower_name)
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .unwrap_or("")
                            .to_string();
                        let idx = index.files.len();
                        index.by_name.entry(lower_name.clone()).or_default().push(idx);
                        let mut tokens = Vec::new();
                        for token in stem.split(['_', '-', '.']) {
                            if is_md5_token(token) {
                                tokens.push(token.to_string());
                            }
                        }
                        if is_md5_token(&stem) {
                            tokens.push(stem.clone());
                        }
                        for token in tokens {
                            index.by_md5.entry(token).or_default().push(idx);
                        }
                        index.files.push(IndexedFile { path, stem });
                    }
                }
            }
        }
        index
    }

    fn find(&self, media: &super::model::MediaRef) -> Option<&IndexedFile> {
        if !media.md5.is_empty() {
            if let Some(indices) = self.by_md5.get(&media.md5.to_ascii_lowercase()) {
                if let Some(index) = self.pick(indices, media.kind) {
                    return self.files.get(index);
                }
            }
            // Some builds store an uppercase or truncated md5 in the XML.
            let prefix = media.md5.to_ascii_lowercase();
            if prefix.len() >= 16 {
                for (token, indices) in &self.by_md5 {
                    if token.starts_with(&prefix) || prefix.starts_with(token.as_str()) {
                        if let Some(index) = self.pick(indices, media.kind) {
                            return self.files.get(index);
                        }
                    }
                }
            }
        }
        if !media.file_name.is_empty() {
            let wanted = media.file_name.to_ascii_lowercase();
            if let Some(indices) = self.by_name.get(&wanted) {
                if let Some(index) = self.pick(indices, media.kind) {
                    return self.files.get(index);
                }
            }
            let stem = Path::new(&wanted)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if let Some(indices) = self.by_name.get(&format!("{stem}.dat")) {
                if let Some(index) = self.pick(indices, media.kind) {
                    return self.files.get(index);
                }
            }
        }
        None
    }

    fn pick(&self, indices: &[usize], kind: MediaKind) -> Option<usize> {
        let mut best = None;
        for &index in indices {
            let file = self.files.get(index)?;
            let ext = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let score = match kind {
                MediaKind::Image => match ext.as_str() {
                    "dat" => 5,
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "hevc" => 10,
                    _ => 1,
                },
                MediaKind::Video => match ext.as_str() {
                    "mp4" | "mov" | "m4v" => 10,
                    "dat" => 4,
                    _ => 1,
                },
                MediaKind::Voice => match ext.as_str() {
                    "silk" | "amr" | "mp3" | "wav" | "aud" => 10,
                    "dat" => 3,
                    _ => 1,
                },
                MediaKind::Emoji => match ext.as_str() {
                    "gif" | "png" | "jpg" | "jpeg" | "webp" => 10,
                    "dat" => 4,
                    _ => 1,
                },
                MediaKind::File => {
                    if ext == "dat" {
                        1
                    } else {
                        10
                    }
                }
            };
            if best.map(|(_, best_score)| score > best_score).unwrap_or(true) {
                best = Some((index, score));
            }
        }
        best.map(|(index, _)| index)
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

pub fn export_media(
    messages: &mut [ExportMessage],
    index: &MediaIndex,
    output_dir: &Path,
    mut image_key: Option<[u8; 16]>,
) -> MediaOutcome {
    if image_key.is_none() {
        for file in &index.files {
            let is_dat = file.path.extension().and_then(|v| v.to_str()).map(|ext| ext.eq_ignore_ascii_case("dat")).unwrap_or(false);
            if is_dat {
                if let Ok(data) = fs::read(&file.path) {
                    if data.starts_with(V2_MAGIC) && data.len() >= 31 {
                        let mut test_ciphertext = [0u8; 16];
                        test_ciphertext.copy_from_slice(&data[15..31]);
                        if let Some(scanned) = auto_scan_image_key_from_process(&test_ciphertext) {
                            image_key = Some(scanned);
                            break;
                        }
                    }
                }
            }
        }
    }
    let media_dir = output_dir.join("media");
    let mut outcome = MediaOutcome {
        saved: 0,
        failed: 0,
        failures: Vec::new(),
    };
    for message in messages.iter_mut() {
        let Some(media) = message.media.clone() else {
            continue;
        };
        let Some(file) = index.find(&media) else {
            outcome.failed += 1;
            outcome.failures.push(format!(
                "message {}: media not found (md5={}, name={})",
                message.index, media.md5, media.file_name
            ));
            continue;
        };
        let stem = if media.kind == MediaKind::File && !file.stem.is_empty() {
            file.stem.clone()
        } else if !media.md5.is_empty() {
            media.md5.clone()
        } else {
            file.stem.clone()
        };
        let safe_stem = crate::util::sanitize_file_name(&stem);
        let prefix = format!("{:05}_{}", message.index, safe_stem);
        let result = if file
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("dat"))
            .unwrap_or(false)
        {
            match decrypt_dat_file(&file.path, image_key) {
                Ok((bytes, ext)) => {
                    let dest = media_dir.join(format!("{prefix}.{ext}"));
                    if let Err(error) = write_bytes(&dest, &bytes) {
                        Err(error)
                    } else {
                        Ok(dest)
                    }
                }
                Err(error) => Err(error),
            }
        } else {
            let ext = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("bin");
            let dest = media_dir.join(format!("{prefix}.{ext}"));
            crate::util::copy_media_file(&file.path, &dest)
                .map(|_| dest)
                .map_err(anyhow::Error::from)
        };
        match result {
            Ok(dest) => {
                message.media_path = Some(format!(
                    "media/{}",
                    dest.file_name().and_then(|value| value.to_str()).unwrap_or("media.bin")
                ));
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

fn decrypt_dat_file(path: &Path, image_key: Option<[u8; 16]>) -> Result<(Vec<u8>, &'static str)> {
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
        let decrypted = decrypt_dat_aes_xor(&data, &key)?;
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

fn decrypt_dat_aes_xor(data: &[u8], key: &[u8; 16]) -> Result<Vec<u8>> {
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
    let pad = *block.last().unwrap_or(&0) as usize;
    if pad == 0 || pad > 16 || pad > block.len() {
        bail!("invalid .dat AES padding");
    }
    block.truncate(block.len() - pad);

    let raw_end = data
        .len()
        .checked_sub(xor_size)
        .filter(|end| *end >= aes_end)
        .ok_or_else(|| anyhow::anyhow!("invalid .dat XOR length"))?;
    let raw = &data[aes_end..raw_end];
    let xor = &data[raw_end..];

    // Deduce xor key from JPEG tail FF D9 if possible, else 0x88
    let xor_key = if xor.len() >= 2 {
        let k1 = xor[xor.len() - 2] ^ 0xFF;
        let k2 = xor[xor.len() - 1] ^ 0xD9;
        if k1 == k2 { k1 } else { 0x88 }
    } else {
        0x88
    };

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
        let (bytes, ext) = decrypt_dat_file(&path, None).unwrap();
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
        let (bytes, ext) = decrypt_dat_file(&path, None).unwrap();
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
        assert!(decrypt_dat_file(&path, None).is_err());
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
        let (bytes, ext) = decrypt_dat_file(&path, Some(key)).unwrap();
        assert_eq!(ext, "png");
        let mut expected = png.to_vec();
        expected.extend_from_slice(&tail);
        assert_eq!(bytes, expected);
    }
}
