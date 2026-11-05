//! Voice-message media extraction.
//!
//! WeChat 4.x keeps voice payloads in a media database (`db_storage/media/*.db`
//! and relatives) inside a column named `voice_data`. The blob is normally a
//! RIFF/WAV file; legacy rows may be SILK (`#!SILK`) or AMR (`#!AMR`).

use std::fs;
use std::path::{Path, PathBuf};

struct VoiceTable {
    name: String,
    blob_col: String,
    server_col: Option<String>,
    local_col: Option<String>,
    msg_col: Option<String>,
    chat_col: Option<String>,
    time_col: Option<String>,
}

struct VoiceDatabase {
    db: crate::db::Db,
    tables: Vec<VoiceTable>,
    /// `chat_name_id` is a rowid into `Name2Id(user_name)`, not a wxid string.
    chat_rowids: std::collections::HashMap<String, i64>,
}

pub struct VoiceIndex {
    databases: Vec<VoiceDatabase>,
    debug: bool,
    misses: std::sync::atomic::AtomicUsize,
}

pub struct VoicePayload {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
}

/// A column that may hold the voice payload.
fn find_voice_blob_column(columns: &[String]) -> Option<String> {
    // Exact names first, then anything voice-ish, then generic blob names.
    if let Some(found) = find_column(columns, &["voice_data", "voicebuf", "voice_blob"]) {
        return Some(found);
    }
    if let Some(column) = columns
        .iter()
        .find(|column| column.to_ascii_lowercase().contains("voice"))
    {
        return Some(column.clone());
    }
    find_column(columns, &["data", "buf", "blob", "content"])
}

fn find_column(columns: &[String], names: &[&str]) -> Option<String> {
    columns.iter().find_map(|column| {
        let lower = column.to_ascii_lowercase();
        names
            .iter()
            .find(|wanted| lower == **wanted)
            .map(|_| column.clone())
    })
}

impl VoiceIndex {
    pub fn build(
        db_storage: &Path,
        key: Option<&[u8; 32]>,
        plaintext: bool,
        debug: bool,
    ) -> Self {
        let mut probe = Vec::new();
        // Prefer dedicated media databases; fall back to message shards only when
        // none of them exposes a voice payload column.
        let mut databases = collect_voice_databases(db_storage, key, plaintext, &mut probe);
        if databases.is_empty() {
            let shards = db_storage.join("message");
            if let Ok(entries) = std::fs::read_dir(&shards) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                        continue;
                    };
                    let lower = name.to_ascii_lowercase();
                    if !lower.ends_with(".db") || !lower.starts_with("message_") {
                        continue;
                    }
                    if let Some(database) =
                        open_voice_database(&path, key, plaintext, &mut probe)
                    {
                        databases.push(database);
                    }
                }
            }
        }
        if debug {
            if probe.is_empty() {
                eprintln!("[i] 语音库: 未发现候选数据库");
            } else {
                eprintln!("[i] 语音库探测:");
                for line in probe.iter().take(60) {
                    eprintln!("    {line}");
                }
                if probe.len() > 60 {
                    eprintln!("    ... 其余 {} 行省略", probe.len() - 60);
                }
                eprintln!(
                    "[i] 语音库: {} 个数据库, {} 张语音表",
                    databases.len(),
                    databases.iter().map(|db| db.tables.len()).sum::<usize>()
                );
            }
        }
        Self {
            databases,
            debug,
            misses: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.databases.is_empty()
    }

    /// Resolve the voice payload for one message.
    pub fn extract(
        &self,
        server_id: i64,
        local_id: i64,
        chat_id: &str,
        timestamp: i64,
    ) -> Option<VoicePayload> {
        for database in &self.databases {
            let chat_rowid = database.chat_rowids.get(chat_id).copied();
            for table in &database.tables {
                if let Some((blob, source)) = query_table(
                    &database.db,
                    table,
                    server_id,
                    local_id,
                    chat_id,
                    chat_rowid,
                    timestamp,
                ) {
                    if self.debug {
                        eprintln!(
                            "[voice] 命中 {}: {} 字节, 头部 {}",
                            source,
                            blob.len(),
                            preview_hex(&blob)
                        );
                    }
                    return decode_voice(blob, self.debug);
                }
            }
        }
        if self.debug
            && self
                .misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                < 3
        {
            eprintln!(
                "[voice] 未命中: svr_id={server_id} local_id={local_id} chat={chat_id} ts={timestamp}"
            );
            for database in &self.databases {
                let chat_rowid = database.chat_rowids.get(chat_id).copied();
                eprintln!(
                    "[voice]   chat_rowid={chat_rowid:?} (Name2Id 共 {} 条)",
                    database.chat_rowids.len()
                );
                for table in &database.tables {
                    dump_sample(&database.db, table, server_id, local_id, chat_rowid, timestamp);
                }
            }
        }
        None
    }
}

fn preview_hex(bytes: &[u8]) -> String {
    hex::encode(&bytes[..bytes.len().min(16)])
}

fn pcm_to_wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn silk_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"#!SILK")
        || (bytes.len() > 8 && bytes[0] == 0x02 && bytes[1..].starts_with(b"#!SILK"))
}

fn dump_sample(
    db: &crate::db::Db,
    table: &VoiceTable,
    server_id: i64,
    local_id: i64,
    chat_rowid: Option<i64>,
    timestamp: i64,
) {
    let quote = |name: &str| crate::db::quote_ident(name);
    let mut select = vec![quote(&table.blob_col)];
    let mut fields: Vec<&str> = Vec::new();
    for column in [
        table.server_col.as_deref(),
        table.local_col.as_deref(),
        table.chat_col.as_deref(),
        table.time_col.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        fields.push(column);
        select.push(quote(column));
    }
    let sql = format!(
        "SELECT {} FROM {} LIMIT 1",
        select.join(","),
        quote(&table.name)
    );
    if let Ok(mut stmt) = db.conn().prepare(&sql) {
        if let Ok(mut rows) = stmt.query([]) {
            if let Ok(Some(row)) = rows.next() {
                let len = row.get::<_, Vec<u8>>(0).map(|b| b.len()).unwrap_or(0);
                let mut parts = Vec::new();
                for (index, field) in fields.iter().enumerate() {
                    let value: String = row
                        .get::<_, rusqlite::types::Value>(index + 1)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".into());
                    parts.push(format!("{field}={value}"));
                }
                eprintln!(
                    "[voice]   样本 {}: blob={}B {}",
                    table.name,
                    len,
                    parts.join(" ")
                );
            }
        }
    }
    if let Some(server_col) = &table.server_col {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} = ?1",
            quote(&table.name),
            quote(server_col)
        );
        if let Ok(count) = db
            .conn()
            .query_row(&sql, [server_id], |row| row.get::<_, i64>(0))
        {
            eprintln!("[voice]   {} = {} 命中 {} 行", server_col, server_id, count);
        }
    }
    if let Some(local_col) = &table.local_col {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} = ?1",
            quote(&table.name),
            quote(local_col)
        );
        if let Ok(count) = db
            .conn()
            .query_row(&sql, [local_id], |row| row.get::<_, i64>(0))
        {
            eprintln!("[voice]   {} = {} 命中 {} 行", local_col, local_id, count);
        }
    }
    if let Some(chat_col) = &table.chat_col {
        if let Some(rowid) = chat_rowid {
            let sql = format!(
                "SELECT COUNT(*) FROM {} WHERE {} = ?1",
                quote(&table.name),
                quote(chat_col)
            );
            if let Ok(count) = db
                .conn()
                .query_row(&sql, [rowid], |row| row.get::<_, i64>(0))
            {
                eprintln!("[voice]   {} = {} 命中 {} 行", chat_col, rowid, count);
            }
        }
    }
    // Show the rows closest to the message time so the stored unit and ids are
    // visible directly.
    if let (Some(time_col), Some(chat_col), Some(rowid)) =
        (&table.time_col, &table.chat_col, chat_rowid)
    {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = ?1 ORDER BY abs(COALESCE({},0) - ?2) LIMIT 3",
            select.join(","),
            quote(&table.name),
            quote(chat_col),
            quote(time_col)
        );
        for center in [timestamp, timestamp * 1000] {
            if center <= 0 {
                continue;
            }
            if let Ok(mut stmt) = db.conn().prepare(&sql) {
                if let Ok(mut rows) = stmt.query(rusqlite::params![rowid, center]) {
                    while let Ok(Some(row)) = rows.next() {
                        let len = row.get::<_, Vec<u8>>(0).map(|b| b.len()).unwrap_or(0);
                        let mut parts = Vec::new();
                        for (index, field) in fields.iter().enumerate() {
                            let value: String = row
                                .get::<_, rusqlite::types::Value>(index + 1)
                                .map(|v| format!("{v:?}"))
                                .unwrap_or_else(|_| "?".into());
                            parts.push(format!("{field}={value}"));
                        }
                        eprintln!(
                            "[voice]   最近(center={center}) {}: blob={}B {}",
                            table.name,
                            len,
                            parts.join(" ")
                        );
                    }
                }
            }
        }
    }
}

fn query_table(
    db: &crate::db::Db,
    table: &VoiceTable,
    server_id: i64,
    local_id: i64,
    chat_id: &str,
    chat_rowid: Option<i64>,
    timestamp: i64,
) -> Option<(Vec<u8>, String)> {
    let quote = |name: &str| crate::db::quote_ident(name);
    let blob = quote(&table.blob_col);
    let select = format!("SELECT {blob} FROM {}", quote(&table.name));
    let not_empty = format!("{blob} IS NOT NULL AND length({blob}) > 0");

    // (sql, args, label, ambiguous_guard)
    let mut attempts: Vec<(String, Vec<Box<dyn rusqlite::ToSql>>, String, bool)> = Vec::new();

    if let Some(col) = &table.server_col {
        if server_id > 0 {
            let sql = format!(
                "{select} WHERE {not_empty} AND {} = ?1 LIMIT 2",
                quote(col)
            );
            attempts.push((
                sql.clone(),
                vec![Box::new(server_id) as Box<dyn rusqlite::ToSql>],
                "svr_id".into(),
                true,
            ));
            attempts.push((
                sql,
                vec![Box::new(server_id.to_string()) as Box<dyn rusqlite::ToSql>],
                "svr_id(text)".into(),
                true,
            ));
        }
    }

    // In WeChat 4.x `VoiceInfo.local_id` is a media-row id, not the message's
    // local_id; matching it unscoped returns another conversation's voice (local
    // ids repeat per chat). Alpha never uses it: chat + create_time is the only
    // reliable locator. Keep the key solely for legacy tables with no chat column.
    if let (Some(local_col), None) = (&table.local_col, &table.chat_col) {
        let sql = format!("{select} WHERE {not_empty} AND {} = ?1 LIMIT 2", quote(local_col));
        attempts.push((
            sql,
            vec![Box::new(local_id) as Box<dyn rusqlite::ToSql>],
            "local_id".into(),
            true,
        ));
    }

    if let Some(col) = &table.msg_col {
        if server_id > 0 {
            let sql = format!("{select} WHERE {not_empty} AND {} = ?1 LIMIT 2", quote(col));
            attempts.push((
                sql.clone(),
                vec![Box::new(server_id) as Box<dyn rusqlite::ToSql>],
                "msg_id".into(),
                true,
            ));
            attempts.push((
                sql,
                vec![Box::new(server_id.to_string()) as Box<dyn rusqlite::ToSql>],
                "msg_id(text)".into(),
                true,
            ));
        }
    }

    if let (Some(time_col), true) = (&table.time_col, timestamp > 0) {
        // WeChat 4.x stores milliseconds; older builds store seconds. Widen the
        // window and pick the closest row, matching Alpha's chat + time lookup.
        let (stamp_ms, stamp_s) = if timestamp >= 100_000_000_000 {
            (timestamp, timestamp / 1000)
        } else {
            (timestamp.saturating_mul(1000), timestamp)
        };
        let units: [(i64, i64, &str); 2] = [(stamp_ms, 300_000, "ms"), (stamp_s, 300, "s")];
        for (stamp, window, unit) in units {
            let base = format!(
                "{select} WHERE {not_empty} AND {} BETWEEN ?1 AND ?2",
                quote(time_col)
            );
            let order = format!("ORDER BY abs(COALESCE({},0) - ?4) ASC LIMIT 1", quote(time_col));
            if let Some(chat_col) = &table.chat_col {
                if let Some(rowid) = chat_rowid {
                    let sql = format!("{base} AND {} = ?3 {order}", quote(chat_col));
                    attempts.push((
                        sql,
                        vec![
                            Box::new(stamp - window) as Box<dyn rusqlite::ToSql>,
                            Box::new(stamp + window) as Box<dyn rusqlite::ToSql>,
                            Box::new(rowid) as Box<dyn rusqlite::ToSql>,
                            Box::new(stamp) as Box<dyn rusqlite::ToSql>,
                        ],
                        format!("chat+time({unit})"),
                        false,
                    ));
                }
                let sql = format!("{base} AND {} = ?3 {order}", quote(chat_col));
                attempts.push((
                    sql,
                    vec![
                        Box::new(stamp - window) as Box<dyn rusqlite::ToSql>,
                        Box::new(stamp + window) as Box<dyn rusqlite::ToSql>,
                        Box::new(chat_id.to_string()) as Box<dyn rusqlite::ToSql>,
                        Box::new(stamp) as Box<dyn rusqlite::ToSql>,
                    ],
                    format!("chat(text)+time({unit})"),
                    false,
                ));
            } else {
                let sql = format!(
                    "{base} ORDER BY abs(COALESCE({},0) - ?3) ASC LIMIT 1",
                    quote(time_col)
                );
                attempts.push((
                    sql,
                    vec![
                        Box::new(stamp - window) as Box<dyn rusqlite::ToSql>,
                        Box::new(stamp + window) as Box<dyn rusqlite::ToSql>,
                        Box::new(stamp) as Box<dyn rusqlite::ToSql>,
                    ],
                    format!("time({unit})"),
                    false,
                ));
            }
        }
    }

    for (sql, args, label, unique) in attempts {
        let Ok(mut stmt) = db.conn().prepare(&sql) else {
            continue;
        };
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|arg| arg.as_ref()).collect();
        let Ok(mut rows) = stmt.query(refs.as_slice()) else {
            continue;
        };
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        while let Ok(Some(row)) = rows.next() {
            if let Ok(blob) = row.get::<_, Vec<u8>>(0) {
                if !blob.is_empty() {
                    blobs.push(blob);
                }
            }
            if blobs.len() > 1 {
                break;
            }
        }
        if unique {
            if blobs.len() == 1 {
                return blobs.pop().map(|blob| (blob, label));
            }
            continue;
        }
        if let Some(blob) = blobs.into_iter().next() {
            return Some((blob, label));
        }
    }
    None
}

fn decode_voice(bytes: Vec<u8>, debug: bool) -> Option<VoicePayload> {
    if bytes.starts_with(b"RIFF") {
        return Some(VoicePayload { bytes, ext: "wav" });
    }
    if bytes.starts_with(b"#!AMR") {
        return convert_with_ffmpeg(&bytes, "amr", "wav")
            .or_else(|| Some(VoicePayload { bytes, ext: "amr" }));
    }
    if silk_magic(&bytes) {
        match silk_codec::decode_silk(bytes.as_slice(), 24000) {
            Ok(pcm) if !pcm.is_empty() => {
                if debug {
                    eprintln!(
                        "[voice] SILK 解码成功: {} 字节 PCM -> WAV 24000Hz",
                        pcm.len()
                    );
                }
                return Some(VoicePayload {
                    bytes: pcm_to_wav(&pcm, 24000),
                    ext: "wav",
                });
            }
            Ok(_) => {
                if debug {
                    eprintln!("[voice] SILK 解码结果为空");
                }
            }
            Err(error) => {
                if debug {
                    eprintln!("[voice] SILK 解码失败: {error:?}");
                }
            }
        }
        return Some(VoicePayload {
            bytes,
            ext: "silk",
        });
    }
    if debug {
        eprintln!("[voice] 未识别音频头: {}", preview_hex(&bytes));
    }
    Some(VoicePayload {
        bytes,
        ext: "bin",
    })
}

fn convert_with_ffmpeg(bytes: &[u8], input_ext: &str, output_ext: &str) -> Option<VoicePayload> {
    let ffmpeg = super::media::find_ffmpeg()?;
    let dir = tempfile::TempDir::new().ok()?;
    let input = dir.path().join(format!("input.{input_ext}"));
    let output = dir.path().join(format!("output.{output_ext}"));
    fs::write(&input, bytes).ok()?;
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "fatal",
            "-y",
            "-i",
            input.to_str()?,
            output.to_str()?,
        ])
        .status()
        .ok()?;
    if status.success() && output.exists() {
        Some(VoicePayload {
            bytes: fs::read(output).ok()?,
            ext: "wav",
        })
    } else {
        None
    }
}

fn collect_voice_databases(
    db_storage: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    probe: &mut Vec<String>,
) -> Vec<VoiceDatabase> {
    let mut out = Vec::new();
    for path in voice_database_candidates(db_storage) {
        if let Some(database) = open_voice_database(&path, key, plaintext, probe) {
            out.push(database);
        }
    }
    out
}

fn blob_looks_like_audio(blob: &[u8]) -> bool {
    blob.starts_with(b"RIFF") || blob.starts_with(b"#!SILK") || blob.starts_with(b"#!AMR")
}

fn table_has_audio_sample(
    db: &crate::db::Db,
    table: &VoiceTable,
) -> bool {
    let sql = format!(
        "SELECT {} FROM {} WHERE {} IS NOT NULL LIMIT 1",
        crate::db::quote_ident(&table.blob_col),
        crate::db::quote_ident(&table.name),
        crate::db::quote_ident(&table.blob_col)
    );
    let Ok(mut stmt) = db.conn().prepare(&sql) else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    match rows.next() {
        Ok(Some(row)) => row
            .get::<_, Vec<u8>>(0)
            .map(|blob| blob_looks_like_audio(&blob))
            .unwrap_or(false),
        _ => false,
    }
}

fn open_voice_database(
    path: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    probe: &mut Vec<String>,
) -> Option<VoiceDatabase> {
    // Some builds ship an unencrypted media database; detect it by header.
    let plaintext = plaintext || looks_like_plain_sqlite(path);
    let db = match crate::db::Db::open(path, key, plaintext) {
        Ok(db) => db,
        Err(error) => {
            probe.push(format!("{}: 打开失败 ({error})", path.display()));
            return None;
        }
    };
    let tables = match db.tables() {
        Ok(tables) => tables,
        Err(error) => {
            probe.push(format!("{}: 列表失败 ({error})", path.display()));
            return None;
        }
    };
    probe.push(format!("{}: {} 张表", path.display(), tables.len()));
    let mut voice_tables = Vec::new();
    for name in tables {
        let Ok(columns) = db.columns(&name) else {
            continue;
        };
        let Some(blob_col) = find_voice_blob_column(&columns) else {
            probe.push(format!("  {name}: 无语音 BLOB 列 [{}]", columns.join(",")));
            continue;
        };
        probe.push(format!(
            "  {name}: blob={blob_col} cols=[{}]",
            columns.join(",")
        ));
        voice_tables.push(VoiceTable {
            name,
            blob_col,
            server_col: find_column(
                &columns,
                &["svr_id", "server_id", "svrid", "serverid", "msg_svr_id"],
            ),
            local_col: find_column(&columns, &["local_id", "msg_local_id"]),
            msg_col: find_column(&columns, &["msg_id", "message_id"]),
            chat_col: find_column(&columns, &["chat_name_id", "chatnameid", "username", "talker"]),
            time_col: find_column(&columns, &["create_time", "time", "insert_time", "msg_time"]),
        });
    }
    // A generic `data` BLOB column may hold images or files; only keep tables whose
    // name/column says "voice" or whose sample starts with an audio magic.
    voice_tables.retain(|table| {
        let named = table.name.to_ascii_lowercase().contains("voice")
            || table.blob_col.to_ascii_lowercase().contains("voice");
        named || table_has_audio_sample(&db, table)
    });
    if voice_tables.is_empty() {
        return None;
    }
    for table in &voice_tables {
        let rows = db.count(&table.name).unwrap_or(0);
        probe.push(format!("  {}: {} 行语音", table.name, rows));
    }
    let mut chat_rowids = std::collections::HashMap::new();
    if let Ok(mut stmt) = db
        .conn()
        .prepare("SELECT rowid, user_name FROM Name2Id")
    {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(row)) = rows.next() {
                let id: i64 = row.get(0).unwrap_or(0);
                let name: String = row.get(1).unwrap_or_default();
                if id > 0 && !name.is_empty() {
                    chat_rowids.insert(name, id);
                }
            }
        }
    }
    probe.push(format!("  Name2Id: {} 条会话映射", chat_rowids.len()));
    Some(VoiceDatabase {
        db,
        tables: voice_tables,
        chat_rowids,
    })
}

fn looks_like_plain_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    file.read_exact(&mut header).is_ok() && &header == b"SQLite format 3\0"
}

fn voice_database_candidates(db_storage: &Path) -> Vec<PathBuf> {
    let mut named = Vec::new();
    let mut shards = Vec::new();
    for dir in [
        db_storage.join("media"),
        db_storage.to_path_buf(),
        db_storage.join("message"),
        db_storage.join("message").join("media"),
        db_storage.join("message").join("voice"),
        db_storage.join("voice"),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".db") {
                continue;
            }
            if lower.contains("media") || lower.contains("voice") || lower == "hardlink.db" {
                named.push(path);
            } else if lower.starts_with("message_") || lower.starts_with("biz_message_") {
                // Some builds keep `voice_data` inside the message shard itself.
                shards.push(path);
            }
        }
    }
    named.sort();
    named.dedup();
    shards.sort();
    shards.dedup();
    named.extend(shards);
    named
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> VoiceIndex {
        let media = dir.join("media");
        std::fs::create_dir_all(&media).unwrap();
        let path = media.join("media.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE voice (server_id INTEGER, local_id INTEGER, chat_name_id TEXT, create_time INTEGER, voice_data BLOB);",
        )
        .unwrap();
        let wav = b"RIFF....WAVEfmt fake".to_vec();
        conn.execute(
            "INSERT INTO voice VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![123456789i64, 42i64, "wxid_chat", 1_700_000_000_000i64, wav],
        )
        .unwrap();
        VoiceIndex::build(dir, None, true, false)
    }

    fn wechat_fixture(dir: &Path) -> VoiceIndex {
        let media = dir.join("message");
        std::fs::create_dir_all(&media).unwrap();
        let path = media.join("media_0.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE Name2Id (user_name TEXT);
             CREATE TABLE VoiceInfo (
                 chat_name_id INTEGER,
                 create_time INTEGER,
                 local_id INTEGER,
                 svr_id INTEGER,
                 voice_data BLOB,
                 data_index INTEGER
             );",
        )
        .unwrap();
        conn.execute("INSERT INTO Name2Id (user_name) VALUES ('wxid_chat')", [])
            .unwrap();
        let wav = b"RIFF....WAVEfmt voice".to_vec();
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (1, ?1, 42, 987654321, ?2, 0)",
            rusqlite::params![1_700_000_000_000i64, wav],
        )
        .unwrap();
        VoiceIndex::build(dir, None, true, false)
    }

    #[test]
    fn matches_wechat_voice_info_schema_by_svr_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = wechat_fixture(dir.path());
        assert!(!index.is_empty());
        let payload = index
            .extract(987654321, 42, "wxid_chat", 1_700_000_000_000)
            .expect("voice payload by svr_id");
        assert!(payload.bytes.starts_with(b"RIFF"));
    }

    #[test]
    fn matches_voice_info_through_name2id_rowid() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = wechat_fixture(dir.path());
        // server/local do not match; chat rowid + time window must resolve it.
        let payload = index
            .extract(0, 999, "wxid_chat", 1_700_000_001_500)
            .expect("voice payload through Name2Id mapping");
        assert!(payload.bytes.starts_with(b"RIFF"));
    }

    #[test]
    fn resolves_voice_by_server_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = fixture(dir.path());
        assert!(!index.is_empty());
        let payload = index
            .extract(123456789, 42, "wxid_chat", 1_700_000_000_000)
            .expect("voice payload");
        assert_eq!(payload.ext, "wav");
        assert!(payload.bytes.starts_with(b"RIFF"));
    }

    #[test]
    fn falls_back_to_chat_and_time_window() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = fixture(dir.path());
        let payload = index
            .extract(0, 999, "wxid_chat", 1_700_000_000_500)
            .expect("voice payload through time window");
        assert!(payload.bytes.starts_with(b"RIFF"));
        // Wrong chat must not match.
        assert!(index.extract(0, 999, "wxid_other", 1_700_000_000_500).is_none());
    }

    #[test]
    fn resolves_millisecond_rows_from_second_timestamps() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = wechat_fixture(dir.path());
        // Message timestamps are seconds; VoiceInfo.create_time is milliseconds.
        let payload = index
            .extract(0, 999, "wxid_chat", 1_700_000_000)
            .expect("voice payload with second-based message timestamp");
        assert!(payload.bytes.starts_with(b"RIFF"));
    }

    #[test]
    fn picks_the_closest_voice_in_the_chat() {
        let dir = tempfile::TempDir::new().unwrap();
        let media = dir.path().join("message");
        std::fs::create_dir_all(&media).unwrap();
        let path = media.join("media_0.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE Name2Id (user_name TEXT);
             CREATE TABLE VoiceInfo (
                 chat_name_id INTEGER,
                 create_time INTEGER,
                 local_id INTEGER,
                 svr_id INTEGER,
                 voice_data BLOB,
                 data_index INTEGER
             );",
        )
        .unwrap();
        conn.execute("INSERT INTO Name2Id (user_name) VALUES ('wxid_chat')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (1, 1700000000000, 1, 0, ?1, 0)",
            rusqlite::params![b"RIFFfirst".to_vec()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (1, 1700000030000, 2, 0, ?1, 0)",
            rusqlite::params![b"RIFFsecond".to_vec()],
        )
        .unwrap();
        drop(conn);
        let index = VoiceIndex::build(dir.path(), None, true, false);
        let payload = index
            .extract(0, 999, "wxid_chat", 1_700_000_029)
            .expect("closest voice");
        assert_eq!(payload.bytes, b"RIFFsecond");
        let earlier = index
            .extract(0, 999, "wxid_chat", 1_700_000_001)
            .expect("first voice");
        assert_eq!(earlier.bytes, b"RIFFfirst");
    }

    #[test]
    fn decodes_silk_payloads_to_24000hz_wav() {
        let samples: Vec<u8> = (0..4800i32)
            .flat_map(|i| ((i * 97 % 1000) as i16 * 20).to_le_bytes())
            .collect();
        for tencent in [false, true] {
            let silk = silk_codec::encode_silk(samples.as_slice(), 24000, 24000, tencent)
                .expect("encode silk");
            assert!(silk_magic(&silk));
            let payload = decode_voice(silk, false).expect("decode silk");
            assert_eq!(payload.ext, "wav");
            assert!(payload.bytes.starts_with(b"RIFF"));
            assert_eq!(&payload.bytes[20..22], &1u16.to_le_bytes()); // PCM
            assert_eq!(&payload.bytes[22..24], &1u16.to_le_bytes()); // mono
            assert_eq!(
                u32::from_le_bytes(payload.bytes[24..28].try_into().unwrap()),
                24000
            );
            assert_eq!(&payload.bytes[34..36], &16u16.to_le_bytes());
            assert!(payload.bytes.len() > 44 + samples.len() / 2);
        }
    }

    #[test]
    fn exports_unknown_voice_blobs_instead_of_dropping_them() {
        let payload = decode_voice(b"\x00\x01weird".to_vec(), false).expect("raw fallback");
        assert_eq!(payload.ext, "bin");
        assert_eq!(payload.bytes, b"\x00\x01weird");
    }

    #[test]
    fn never_matches_another_chats_voice_by_local_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let media = dir.path().join("message");
        std::fs::create_dir_all(&media).unwrap();
        let path = media.join("media_0.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE Name2Id (user_name TEXT);
             CREATE TABLE VoiceInfo (
                 chat_name_id INTEGER,
                 create_time INTEGER,
                 local_id INTEGER,
                 svr_id INTEGER,
                 voice_data BLOB,
                 data_index INTEGER
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO Name2Id(rowid,user_name) VALUES (1,'wxid_chat'),(2,'wxid_other')",
            [],
        )
        .unwrap();
        // Another conversation owns media row 197; our message has local_id 197 too.
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (2, 1700000000000, 197, 0, ?1, 0)",
            rusqlite::params![b"RIFFother".to_vec()],
        )
        .unwrap();
        // Our chat's voice is located by chat + time, its media local_id differs.
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (1, 1700000005000, 1, 0, ?1, 0)",
            rusqlite::params![b"RIFFours".to_vec()],
        )
        .unwrap();
        drop(conn);
        let index = VoiceIndex::build(dir.path(), None, true, false);
        let payload = index
            .extract(0, 197, "wxid_chat", 1_700_000_000)
            .expect("voice of our chat");
        assert_eq!(payload.bytes, b"RIFFours");
    }

    #[test]
    fn rejects_ambiguous_server_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let _index = fixture(dir.path());
        let media = dir.path().join("media").join("media.db");
        let conn = rusqlite::Connection::open(&media).unwrap();
        conn.execute(
            "INSERT INTO voice VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![123456789i64, 43i64, "wxid_chat", 1_700_000_000_001i64, b"RIFF2".to_vec()],
        )
        .unwrap();
        drop(conn);
        let index = VoiceIndex::build(dir.path(), None, true, false);
        assert!(index.extract(123456789, 0, "wxid_chat", 0).is_none());
        assert!(!index.is_empty());
    }
}
