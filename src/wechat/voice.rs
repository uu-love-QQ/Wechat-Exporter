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
}

pub struct VoiceIndex {
    databases: Vec<VoiceDatabase>,
}

pub struct VoicePayload {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
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
    ) -> Self {
        // Prefer dedicated media databases; fall back to message shards only when
        // none of them exposes a `voice_data` column.
        let mut databases = collect_voice_databases(db_storage, key, plaintext);
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
                    if let Some(database) = open_voice_database(&path, key, plaintext) {
                        databases.push(database);
                    }
                }
            }
        }
        Self { databases }
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
            for table in &database.tables {
                if let Some(blob) = query_table(&database.db, table, server_id, local_id, chat_id, timestamp) {
                    return classify(blob);
                }
            }
        }
        None
    }
}

fn query_table(
    db: &crate::db::Db,
    table: &VoiceTable,
    server_id: i64,
    local_id: i64,
    chat_id: &str,
    timestamp: i64,
) -> Option<Vec<u8>> {
    let select = format!(
        "SELECT {} FROM {}",
        crate::db::quote_ident(&table.blob_col),
        crate::db::quote_ident(&table.name)
    );

    // Prefer the strongest identifier available; a 2-row answer means the key is
    // ambiguous (Alpha reports `server_id_ambiguous` and moves on).
    let attempts: Vec<(String, Vec<Box<dyn rusqlite::ToSql>>)> = {
        let mut attempts: Vec<(String, Vec<Box<dyn rusqlite::ToSql>>)> = Vec::new();
        if let Some(col) = &table.server_col {
            if server_id > 0 {
                attempts.push((
                    format!("{select} WHERE {} = ?1 LIMIT 2", crate::db::quote_ident(col)),
                    vec![Box::new(server_id) as Box<dyn rusqlite::ToSql>],
                ));
            }
        }
        if let Some(col) = &table.local_col {
            attempts.push((
                format!("{select} WHERE {} = ?1 LIMIT 2", crate::db::quote_ident(col)),
                vec![Box::new(local_id) as Box<dyn rusqlite::ToSql>],
            ));
        }
        if let Some(col) = &table.msg_col {
            if server_id > 0 {
                attempts.push((
                    format!("{select} WHERE {} = ?1 LIMIT 2", crate::db::quote_ident(col)),
                    vec![Box::new(server_id.to_string()) as Box<dyn rusqlite::ToSql>],
                ));
            }
        }
        if let (Some(time_col), true) = (&table.time_col, timestamp > 0) {
            let mut sql = format!(
                "{select} WHERE {} BETWEEN ?1 AND ?2",
                crate::db::quote_ident(time_col)
            );
            let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new(timestamp - 2000) as Box<dyn rusqlite::ToSql>,
                Box::new(timestamp + 2000) as Box<dyn rusqlite::ToSql>,
            ];
            if let Some(chat_col) = &table.chat_col {
                sql.push_str(&format!(" AND {} = ?3", crate::db::quote_ident(chat_col)));
                args.push(Box::new(chat_id.to_string()) as Box<dyn rusqlite::ToSql>);
            }
            sql.push_str(" LIMIT 2");
            attempts.push((sql, args));
        }
        attempts
    };

    for (sql, args) in attempts {
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
                blobs.push(blob);
            }
            if blobs.len() > 1 {
                break;
            }
        }
        if blobs.len() == 1 {
            return blobs.pop();
        }
    }
    None
}

fn classify(bytes: Vec<u8>) -> Option<VoicePayload> {
    if bytes.starts_with(b"RIFF") {
        return Some(VoicePayload { bytes, ext: "wav" });
    }
    if bytes.starts_with(b"#!AMR") {
        return convert_with_ffmpeg(&bytes, "amr", "wav")
            .or_else(|| Some(VoicePayload { bytes, ext: "amr" }));
    }
    if bytes.starts_with(b"#!SILK") {
        // No SILK decoder is bundled; keep the raw container instead of dropping
        // the message, and let ffmpeg try in case the build supports it.
        return convert_with_ffmpeg(&bytes, "silk", "wav")
            .or_else(|| Some(VoicePayload { bytes, ext: "silk" }));
    }
    None
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
) -> Vec<VoiceDatabase> {
    let mut out = Vec::new();
    for path in voice_database_candidates(db_storage) {
        if let Some(database) = open_voice_database(&path, key, plaintext) {
            out.push(database);
        }
    }
    out
}

fn open_voice_database(
    path: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
) -> Option<VoiceDatabase> {
    let db = crate::db::Db::open(path, key, plaintext).ok()?;
    let tables = db.tables().ok()?;
    let mut voice_tables = Vec::new();
    for name in tables {
        let Ok(columns) = db.columns(&name) else {
            continue;
        };
        let Some(blob_col) = find_column(&columns, &["voice_data", "voicebuf"]) else {
            continue;
        };
        voice_tables.push(VoiceTable {
            name,
            blob_col,
            server_col: find_column(&columns, &["server_id", "svrid", "msg_svr_id"]),
            local_col: find_column(&columns, &["local_id", "msg_local_id"]),
            msg_col: find_column(&columns, &["msg_id", "message_id"]),
            chat_col: find_column(&columns, &["chat_name_id", "chatnameid", "username", "talker"]),
            time_col: find_column(&columns, &["create_time", "time", "insert_time", "msg_time"]),
        });
    }
    if voice_tables.is_empty() {
        return None;
    }
    Some(VoiceDatabase {
        db,
        tables: voice_tables,
    })
}

fn voice_database_candidates(db_storage: &Path) -> Vec<PathBuf> {
    let mut named = Vec::new();
    let mut shards = Vec::new();
    for dir in [
        db_storage.join("media"),
        db_storage.to_path_buf(),
        db_storage.join("message"),
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
        VoiceIndex::build(dir, None, true)
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
        let index = VoiceIndex::build(dir.path(), None, true);
        assert!(index.extract(123456789, 0, "wxid_chat", 0).is_none());
        assert!(!index.is_empty());
    }
}
