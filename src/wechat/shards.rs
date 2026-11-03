use anyhow::{Context, Result};
use rusqlite::types::ValueRef;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::model::RawMessage;
use crate::db::{quote_ident, Db};
use crate::util::{md5_hex, message_table_name};

pub struct Shard {
    pub db: Db,
    pub name2id: HashMap<i64, String>,
    pub user_by_table: HashMap<String, String>,
    pub tables: Vec<String>,
}

impl Shard {
    fn load(path: &Path, _index: usize, key: Option<&[u8; 32]>, plaintext: bool) -> Result<Shard> {
        let db = Db::open(path, key, plaintext)?;
        let tables = db.tables()?;
        let mut message_tables = Vec::new();
        for table in &tables {
            if table.len() > 4 && table[..4].eq_ignore_ascii_case("Msg_") {
                message_tables.push(table.clone());
            }
        }
        let mut name2id = HashMap::new();
        let name_table = tables
            .iter()
            .find(|name| name.eq_ignore_ascii_case("name2id"))
            .cloned();
        if let Some(name_table) = name_table {
            let columns = db.columns(&name_table)?;
            let user_column = columns
                .iter()
                .find(|c| c.eq_ignore_ascii_case("user_name") || c.eq_ignore_ascii_case("username"))
                .cloned();
            if let Some(user_column) = user_column {
                let sql = format!(
                    "SELECT rowid, {} FROM {}",
                    quote_ident(&user_column),
                    quote_ident(&name_table)
                );
                let mut stmt = db.conn().prepare(&sql)?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                })?;
                for row in rows {
                    let (rowid, user) = row?;
                    if let Some(user) = user {
                        if !user.is_empty() {
                            name2id.insert(rowid, user);
                        }
                    }
                }
            }
        }
        let mut user_by_table = HashMap::new();
        for user in name2id.values() {
            user_by_table.insert(message_table_name(user), user.clone());
        }
        Ok(Shard {
            db,
            name2id,
            user_by_table,
            tables: message_tables,
        })
    }

    pub fn table_for_user(&self, conversation_id: &str) -> Option<String> {
        let wanted = message_table_name(conversation_id);
        self.tables
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&wanted))
            .cloned()
    }

    pub fn count_table(&self, table: &str) -> i64 {
        self.db.count(table).unwrap_or(0)
    }

    pub fn message_rows(&self, table: &str) -> Result<Vec<RawMessage>> {
        let columns = self.db.columns(table)?;
        let has = |name: &str| {
            columns
                .iter()
                .any(|column| column.eq_ignore_ascii_case(name))
        };
        let expr = |name: &str, fallback: &str| {
            if has(name) {
                quote_ident(name)
            } else {
                fallback.to_string()
            }
        };
        let local_id = if has("local_id") {
            "CAST(local_id AS INTEGER)".to_string()
        } else {
            "rowid".to_string()
        };
        let select = format!(
            "SELECT {local_id}, CAST({server} AS INTEGER), CAST({local_type} AS INTEGER), \
             CAST({sort_seq} AS INTEGER), CAST({sender} AS INTEGER), CAST({create_time} AS INTEGER), \
             {source}, {content}, {compress}, {packed}, CAST({ct_content} AS INTEGER), CAST({ct_source} AS INTEGER) \
             FROM {table} ORDER BY CAST({create_time} AS INTEGER), CAST({sort_seq} AS INTEGER), {local_id}",
            local_id = local_id,
            server = expr("server_id", "0"),
            local_type = expr("local_type", "0"),
            sort_seq = expr("sort_seq", "0"),
            sender = expr("real_sender_id", "0"),
            create_time = expr("create_time", "0"),
            source = expr("source", "''"),
            content = expr("message_content", "NULL"),
            compress = expr("compress_content", "NULL"),
            packed = expr("packed_info_data", "NULL"),
            ct_content = expr("WCDB_CT_message_content", "0"),
            ct_source = expr("WCDB_CT_source", "0"),
            table = quote_ident(table),
        );
        let mut stmt = self.db.conn().prepare(&select)?;
        let rows = stmt.query_map([], |row| {
            let local_id: i64 = row.get(0)?;
            let server_id: i64 = row.get(1)?;
            let local_type: i64 = row.get(2)?;
            let sort_seq: i64 = row.get(3)?;
            let real_sender_id: i64 = row.get(4)?;
            let create_time: i64 = row.get(5)?;
            let source = value_to_bytes(row.get_ref(6)?);
            let content = value_to_bytes(row.get_ref(7)?);
            let compress = value_to_bytes(row.get_ref(8)?);
            let packed = value_to_bytes(row.get_ref(9)?);
            let ct_content: i64 = row.get(10)?;
            Ok((
                local_id,
                server_id,
                local_type,
                sort_seq,
                real_sender_id,
                create_time,
                source,
                content,
                compress,
                packed,
                ct_content,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                local_id,
                server_id,
                local_type,
                sort_seq,
                real_sender_id,
                create_time,
                source,
                content,
                compress,
                packed,
                ct_content,
            ) = row?;
            let mut payload = content;
            if payload.is_empty() && !compress.is_empty() {
                payload = compress;
            }
            let payload = decompress_payload(payload, ct_content);
            let raw_content = String::from_utf8_lossy(&payload).to_string();
            let source_text = String::from_utf8_lossy(&source).to_string();
            let base_type = local_type & 0xFFFF_FFFF;
            let sub_type = (local_type >> 32) & 0xFFFF_FFFF;
            let sender_id = self
                .name2id
                .get(&real_sender_id)
                .cloned()
                .unwrap_or_default();
            out.push(RawMessage {
                local_id,
                server_id,
                local_type,
                base_type,
                sub_type,
                sort_seq,
                create_time,
                sender_id,
                source: source_text,
                raw_content,
                packed_info: packed,
            });
        }
        Ok(out)
    }
}

pub struct Store {
    pub shards: Vec<Shard>,
}

impl Store {
    pub fn open(message_dbs: &[PathBuf], key: Option<&[u8; 32]>, plaintext: bool) -> Result<Store> {
        let mut shards = Vec::new();
        for (index, path) in message_dbs.iter().enumerate() {
            let shard = Shard::load(path, index, key, plaintext)
                .with_context(|| format!("failed to open message shard {}", path.display()))?;
            shards.push(shard);
        }
        if shards.is_empty() {
            anyhow::bail!("no readable message shards");
        }
        Ok(Store { shards })
    }

    pub fn conversation_ids(&self) -> Vec<String> {
        let mut set = HashSet::new();
        for shard in &self.shards {
            for table in &shard.tables {
                if let Some(user) = shard.user_by_table.get(table) {
                    set.insert(user.clone());
                }
            }
        }
        let mut out: Vec<String> = set.into_iter().collect();
        out.sort();
        out
    }

    pub fn message_count(&self, conversation_id: &str) -> i64 {
        let mut total = 0;
        for shard in &self.shards {
            if let Some(table) = shard.table_for_user(conversation_id) {
                total += shard.count_table(&table);
            }
        }
        total
    }

    pub fn latest_time(&self, conversation_id: &str) -> i64 {
        let mut latest = 0;
        for shard in &self.shards {
            if let Some(table) = shard.table_for_user(conversation_id) {
                if let Ok(time) = shard.db.conn().query_row(
                    &format!(
                        "SELECT MAX(CAST(create_time AS INTEGER)) FROM {}",
                        quote_ident(&table)
                    ),
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                ) {
                    latest = latest.max(time.unwrap_or(0));
                }
            }
        }
        latest
    }

    pub fn detect_self_username(
        &self,
        account_root: Option<&Path>,
        contacts: &HashMap<String, super::model::Contact>,
        explicit: Option<&str>,
    ) -> Option<String> {
        if let Some(value) = explicit {
            return Some(value.to_string());
        }
        let raw_name = account_root
            .and_then(|root| root.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_string);
        let candidate = raw_name.as_deref().map(crate::util::clean_account_id);
        let mut candidates = Vec::new();
        if let Some(raw) = &raw_name {
            candidates.push(raw.clone());
            if let Some((head, _)) = raw.rsplit_once('_') {
                if !head.is_empty() {
                    candidates.push(head.to_string());
                }
            }
        }
        if let Some(candidate) = &candidate {
            candidates.push(candidate.clone());
        }
        candidates.dedup();
        for candidate in &candidates {
            if contacts.contains_key(candidate) {
                return Some(candidate.clone());
            }
            let known = self
                .shards
                .iter()
                .any(|shard| shard.name2id.values().any(|value| value == candidate));
            if known {
                return Some(candidate.clone());
            }
        }

        let mut counts: HashMap<String, i64> = HashMap::new();
        for shard in self.shards.iter().take(6) {
            for table in shard.tables.iter().take(40) {
                let sql = format!(
                    "SELECT CAST(real_sender_id AS INTEGER), COUNT(*) FROM {} GROUP BY real_sender_id ORDER BY 2 DESC LIMIT 50",
                    quote_ident(table)
                );
                let Ok(mut stmt) = shard.db.conn().prepare(&sql) else {
                    continue;
                };
                let Ok(rows) = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                }) else {
                    continue;
                };
                for row in rows.flatten() {
                    if let Some(user) = shard.name2id.get(&row.0) {
                        *counts.entry(user.clone()).or_insert(0) += row.1;
                    }
                }
            }
        }
        let mut ranked: Vec<(String, i64)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                let a_pref = a.0.starts_with("wxid_");
                let b_pref = b.0.starts_with("wxid_");
                b_pref.cmp(&a_pref)
            })
        });
        ranked
            .into_iter()
            .find(|(user, _)| !user.ends_with("@chatroom"))
            .map(|(user, _)| user)
            .or_else(|| candidate)
    }

    pub fn load_messages(
        &self,
        conversation_id: &str,
        self_username: Option<&str>,
        contact_lookup: &HashMap<String, super::model::Contact>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> Result<Vec<super::model::ExportMessage>> {
        let start_sec = start_time_ms.map(|value| value.div_euclid(1000));
        let end_sec = end_time_ms.map(|value| value.div_euclid(1000));
        let mut raw = Vec::new();
        for shard in &self.shards {
            let Some(table) = shard.table_for_user(conversation_id) else {
                continue;
            };
            let rows = shard.message_rows(&table)?;
            for message in rows {
                if let Some(start) = start_sec {
                    if message.create_time < start {
                        continue;
                    }
                }
                if let Some(end) = end_sec {
                    if message.create_time > end {
                        continue;
                    }
                }
                raw.push(message);
            }
        }

        let mut seen = HashSet::new();
        raw.retain(|message| {
            let key = if message.server_id > 0 {
                format!("s:{}", message.server_id)
            } else {
                format!(
                    "l:{}:{}:{}:{}",
                    message.local_id,
                    message.create_time,
                    message.sender_id,
                    md5_hex(&message.raw_content)
                )
            };
            seen.insert(key)
        });
        raw.sort_by_key(|message| (message.create_time, message.sort_seq, message.local_id));

        let mut out = Vec::with_capacity(raw.len());
        for (index, message) in raw.into_iter().enumerate() {
            out.push(super::build::export_message(
                &message,
                self_username,
                contact_lookup,
                conversation_id,
                index + 1,
            ));
        }
        Ok(out)
    }
}

fn value_to_bytes(value: ValueRef<'_>) -> Vec<u8> {
    match value {
        ValueRef::Null => Vec::new(),
        ValueRef::Integer(value) => value.to_string().into_bytes(),
        ValueRef::Real(value) => value.to_string().into_bytes(),
        ValueRef::Text(value) => value.to_vec(),
        ValueRef::Blob(value) => value.to_vec(),
    }
}

pub fn decompress_payload(mut payload: Vec<u8>, ct: i64) -> Vec<u8> {
    const ZSTD_MAGIC: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD];
    let offset = payload
        .windows(4)
        .position(|window| window == ZSTD_MAGIC)
        .filter(|offset| *offset <= 4);
    let is_zstd = ct == 4 || offset.is_some();
    if is_zstd && !payload.is_empty() {
        let candidate = offset.map(|offset| payload[offset..].to_vec()).unwrap_or_else(|| payload.clone());
        if let Ok(decoded) = decode_zstd(&candidate) {
            payload = decoded;
        }
    }
    while payload.last() == Some(&0) {
        payload.pop();
    }
    payload
}

fn decode_zstd(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(data)
        .map_err(|error| anyhow::anyhow!("zstd decode error: {error}"))?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub fn media_from_packed_info(blob: &[u8]) -> Option<String> {
    if blob.is_empty() {
        return None;
    }
    let hex_chars = |byte: u8| byte.is_ascii_hexdigit();
    let mut i = 0;
    while i + 32 <= blob.len() {
        if blob[i..i + 32].iter().all(|byte| hex_chars(*byte)) {
            if let Ok(value) = std::str::from_utf8(&blob[i..i + 32]) {
                return Some(value.to_ascii_lowercase());
            }
        }
        i += 1;
    }
    None
}
