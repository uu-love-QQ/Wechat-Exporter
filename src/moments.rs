use anyhow::{Context, Result};
use base64::Engine as _;
use chrono::{Datelike, Local, TimeZone};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::db::Db;

#[derive(Default, Debug, Clone)]
pub struct MomentsStats {
    pub posts: usize,
    pub likes_received: usize,
    pub likes_given: usize,
    pub post_types: BTreeMap<String, usize>,
    pub top_likers: BTreeMap<String, usize>,
    pub top_liked_authors: BTreeMap<String, usize>,
}

pub fn moments_db_path(account_root: Option<&Path>, db_storage: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = account_root {
        candidates.push(root.join("sns").join("sns.db"));
        candidates.push(root.join("db_storage").join("sns").join("sns.db"));
    }
    if let Some(db) = db_storage {
        candidates.push(db.join("sns").join("sns.db"));
        if let Some(parent) = db.parent() {
            candidates.push(parent.join("sns").join("sns.db"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

pub fn load_moments(
    account_root: Option<&Path>,
    db_storage: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    self_username: Option<&str>,
    year: i32,
) -> Option<MomentsStats> {
    let path = moments_db_path(account_root, db_storage)?;
    load_moments_from_path(&path, key, plaintext, self_username, year).ok().flatten()
}

pub fn load_moments_from_path(
    path: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    self_username: Option<&str>,
    year: i32,
) -> Result<Option<MomentsStats>> {
    let db = Db::open(path, key, plaintext)
        .with_context(|| format!("failed to open moments database {}", path.display()))?;
    let tables = db.tables().context("failed to list moments tables")?;
    let table = tables
        .iter()
        .find(|table| table.eq_ignore_ascii_case("SnsTimeLine") || table.to_ascii_lowercase().contains("snstimeline"))
        .cloned()
        .context("SnsTimeLine table not found")?;
    let columns = db.columns(&table).context("failed to read SnsTimeLine columns")?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let tid_col = ["tid", "Tid", "ID"].iter().find(|name| has(name)).copied();
    let user_col = ["user_name", "userName", "username"].iter().find(|name| has(name)).copied();
    let content_col = ["content", "Content"].iter().find(|name| has(name)).copied();
    let time_col = ["create_time", "createTime", "createtime"].iter().find(|name| has(name)).copied();
    let Some(content_col) = content_col else {
        return Ok(None);
    };

    let mut fields = Vec::new();
    let mut next_index = 0usize;
    let tid_index = if let Some(column) = tid_col {
        fields.push(crate::db::quote_ident(column));
        let index = next_index;
        next_index += 1;
        Some(index)
    } else {
        None
    };
    let user_index = next_index;
    if let Some(column) = user_col {
        fields.push(crate::db::quote_ident(column));
    } else {
        fields.push("''".to_string());
    }
    next_index += 1;
    let content_index = next_index;
    fields.push(crate::db::quote_ident(content_col));
    next_index += 1;
    let time_index = time_col.map(|column| {
        fields.push(crate::db::quote_ident(column));
        next_index
    });
    let sql = format!("SELECT {} FROM {}", fields.join(", "), crate::db::quote_ident(&table));
    let mut stmt = db.conn().prepare(&sql).context("failed to prepare moments query")?;
    let mut rows = stmt.query([]).context("failed to query moments")?;

    let timeline_re = Regex::new(r"(?is)<TimelineObject\b.*?</TimelineObject>").expect("valid timeline regex");
    let content_object_re = Regex::new(r"(?is)<ContentObject\b.*?</ContentObject>").expect("valid content regex");
    let like_list_re = Regex::new(r"(?is)<(?:LikeUserList|likeUserList|likeList|like_user_list)\b.*?</(?:LikeUserList|likeUserList|likeList|like_user_list)>").expect("valid like list regex");
    let username_re = Regex::new(r"(?is)<username\b[^>]*>(.*?)</username>").expect("valid username regex");
    let self_id = self_username.unwrap_or("").trim().to_lowercase();
    let mut stats = MomentsStats::default();
    let mut posts: HashMap<i64, (String, bool, Vec<String>)> = HashMap::new();
    let mut seen_posts = std::collections::HashSet::new();
    let mut synthetic_id = -1i64;

    while let Ok(Some(row)) = rows.next() {
        let post_id = tid_index
            .and_then(|index| row_i64(&row, index))
            .unwrap_or_else(|| {
                let id = synthetic_id;
                synthetic_id -= 1;
                id
            });
        let poster = row_text(&row, user_index);
        let content = row_content(&row, content_index);
        let table_time = time_index.map(|index| row_i64(&row, index).unwrap_or(0)).unwrap_or(0);
        let Some(timeline) = timeline_re.find(&content).map(|value| value.as_str()) else {
            continue;
        };
        let xml_timestamp = crate::wechat::parse::xml_value(&content, "createTime")
            .or_else(|| crate::wechat::parse::xml_value(timeline, "createTime"))
            .and_then(|value| value.trim().parse::<i64>().ok());
        // Alpha/WeFlow filter years strictly by the XML createTime. A table
        // timestamp is not the post time (it can be a cache/sync time).
        let timestamp = if year != 0 {
            xml_timestamp.unwrap_or(0)
        } else {
            xml_timestamp.unwrap_or(table_time)
        };
        let timestamp = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
        if timestamp <= 0 {
            continue;
        }
        let Some(date) = Local.timestamp_opt(timestamp, 0).single() else {
            continue;
        };
        if year != 0 && date.year() != year {
            continue;
        }
        let poster = poster.trim().to_string();
        let poster_id = poster.to_lowercase();
        let is_self = !self_id.is_empty() && poster_id == self_id;
        let content_object = content_object_re.find(timeline).map(|value| value.as_str());
        let content_type = crate::wechat::parse::xml_value(&content, "contentStyle")
            .or_else(|| content_object.and_then(|value| crate::wechat::parse::xml_value(value, "type")))
            .or_else(|| crate::wechat::parse::xml_value(timeline, "contentType"))
            .or_else(|| crate::wechat::parse::xml_value(timeline, "type"))
            .unwrap_or_else(|| "1".to_string());
        // 朋友圈的“位置”动态（ContentObject type 7）不是 Alpha 年度统计
        // 中的发帖/点赞对象。真实样本中它们通常没有正文和互动。
        if content_type.trim() == "7" {
            continue;
        }
        let label = match content_type.trim() {
            "1" => "图片",
            "2" => "文字",
            "3" => "链接",
            "4" | "42" => "音乐",
            "5" => "分享",
            "15" => "视频",
            "28" => "视频号",
            "34" => "视频号直播",
            "54" => "实况照片",
            _ => "其他",
        };
        let xml_likes = like_list_re
            .find(&content)
            .map(|block| {
                username_re
                    .captures_iter(block.as_str())
                    .filter_map(|capture| capture.get(1).map(|value| crate::wechat::parse::unescape_xml(value.as_str()).trim().to_string()))
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if is_self && seen_posts.insert(post_id) {
            stats.posts += 1;
            *stats.post_types.entry(label.to_string()).or_default() += 1;
        }
        posts.insert(post_id, (poster, is_self, xml_likes));
    }

    // WeFlow/Alpha derive annual likes directly from the timeline XML.
    let mut seen_likes: std::collections::HashSet<(i64, String)> = std::collections::HashSet::new();
    for (post_id, (author, is_self, likes)) in &posts {
        for liker in likes {
            let liker_id = liker.to_lowercase();
            if !seen_likes.insert((*post_id, liker_id.clone())) {
                continue;
            }
            if *is_self {
                if liker_id != self_id {
                    stats.likes_received += 1;
                    *stats.top_likers.entry(liker.clone()).or_default() += 1;
                }
            } else if liker_id == self_id && author.to_lowercase() != self_id {
                stats.likes_given += 1;
                *stats.top_liked_authors.entry(author.clone()).or_default() += 1;
            }
        }
    }

    if stats.posts == 0 && stats.likes_received == 0 && stats.likes_given == 0 {
        return Ok(None);
    }
    Ok(Some(stats))
}

fn row_content(row: &rusqlite::Row<'_>, index: usize) -> String {
    let Some(value) = row.get::<_, Option<rusqlite::types::Value>>(index).ok().flatten() else {
        return String::new();
    };
    let raw = match value {
        rusqlite::types::Value::Text(value) => return decode_sns_text(value),
        rusqlite::types::Value::Blob(bytes) => bytes,
        _ => return String::new(),
    };
    let bytes = if raw.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = match ruzstd::decoding::StreamingDecoder::new(raw.as_slice()) {
            Ok(decoder) => decoder,
            Err(_) => return String::new(),
        };
        let mut decoded = Vec::new();
        use std::io::Read;
        if decoder.read_to_end(&mut decoded).is_err() {
            return String::new();
        }
        decoded
    } else {
        raw
    };
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn decode_sns_text(value: String) -> String {
    let text = value.trim().to_string();
    if text.is_empty() || text.starts_with('<') {
        return text;
    }
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.len() >= 16 && compact.len() % 2 == 0 && compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        if let Ok(bytes) = hex::decode(&compact) {
            if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
                if let Ok(mut decoder) = ruzstd::decoding::StreamingDecoder::new(bytes.as_slice()) {
                    let mut decoded = Vec::new();
                    use std::io::Read;
                    if decoder.read_to_end(&mut decoded).is_ok() {
                        return String::from_utf8_lossy(&decoded).trim().to_string();
                    }
                }
            }
            if let Ok(text) = String::from_utf8(bytes) {
                return text.trim().to_string();
            }
        }
    }
    if compact.len() >= 24 && compact.len() % 4 == 0 {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&compact) {
            if let Ok(text) = String::from_utf8(bytes) {
                return text.trim().to_string();
            }
        }
    }
    text
}

fn row_text(row: &rusqlite::Row<'_>, index: usize) -> String {
    match row.get::<_, Option<rusqlite::types::Value>>(index).ok().flatten() {
        Some(rusqlite::types::Value::Text(value)) => value,
        Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        _ => String::new(),
    }
}

fn row_i64(row: &rusqlite::Row<'_>, index: usize) -> Option<i64> {
    match row.get::<_, Option<rusqlite::types::Value>>(index).ok().flatten() {
        Some(rusqlite::types::Value::Integer(value)) => Some(value),
        Some(rusqlite::types::Value::Text(value)) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub fn ranked(rows: &BTreeMap<String, usize>, limit: usize) -> Vec<(String, usize)> {
    let mut rows = rows.iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows.into_iter().take(limit).map(|(name, count)| (name.clone(), *count)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_all_like_usernames_and_excludes_self() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (user_name TEXT, content TEXT);
             INSERT INTO SnsTimeLine VALUES ('me', '<timelineObject><createTime>1700000000</createTime><contentType>1</contentType><likeList><like><username>friend</username><username>other</username><username>me</username></like></likeList></timelineObject>');
             INSERT INTO SnsTimeLine VALUES ('friend', '<timelineObject><createTime>1700000001</createTime><contentType>2</contentType><likeList><like><username>me</username></like></likeList></timelineObject>');",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.likes_received, 2);
        assert_eq!(stats.likes_given, 1);
        assert_eq!(stats.top_likers.get("friend"), Some(&1));
        assert_eq!(stats.top_likers.get("other"), Some(&1));
        assert_eq!(stats.top_likers.get("me"), None);
        assert_eq!(stats.top_liked_authors.get("friend"), Some(&1));
    }

    #[test]
    fn ignores_rows_without_xml_create_time_for_year_reports() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (tid INTEGER PRIMARY KEY, user_name TEXT, content TEXT, create_time INTEGER);
             INSERT INTO SnsTimeLine VALUES (100, 'me', '<TimelineObject><createTime>1760000000</createTime><contentStyle>1</contentStyle></TimelineObject>', 1760000000);
             INSERT INTO SnsTimeLine VALUES (101, 'me', '<TimelineObject><contentStyle>1</contentStyle></TimelineObject>', 1760000000);
             INSERT INTO SnsTimeLine VALUES (102, 'me', '<TimelineObject><createTime>1730000000</createTime><contentStyle>2</contentStyle></TimelineObject>', 1730000000);",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 2025).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.post_types.get("图片"), Some(&1));
    }

    #[test]
    fn ignores_location_updates() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (tid INTEGER PRIMARY KEY, user_name TEXT, content TEXT);
             INSERT INTO SnsTimeLine VALUES (100, 'me', '<TimelineObject><createTime>1700000000</createTime><ContentObject><type>1</type></ContentObject></TimelineObject>');
             INSERT INTO SnsTimeLine VALUES (101, 'me', '<TimelineObject><createTime>1700000001</createTime><ContentObject><type>7</type></ContentObject></TimelineObject>');",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.post_types.get("图片"), Some(&1));
        assert_eq!(stats.post_types.get("其他"), None);
    }

    #[test]
    fn reads_like_list_outside_timeline_object() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (tid INTEGER PRIMARY KEY, user_name TEXT, content TEXT);
             INSERT INTO SnsTimeLine VALUES (100, 'me', '<TimelineObject><createTime>1700000000</createTime><contentStyle>1</contentStyle></TimelineObject><likeUserList><User><username>friend</username></User><User><username>other</username></User></likeUserList>');
             INSERT INTO SnsTimeLine VALUES (101, 'friend', '<TimelineObject><createTime>1700000001</createTime><contentStyle>2</contentStyle></TimelineObject><likeUserList><User><username>me</username></User></likeUserList>');",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.likes_received, 2);
        assert_eq!(stats.likes_given, 1);
        assert_eq!(stats.top_likers.get("friend"), Some(&1));
        assert_eq!(stats.top_liked_authors.get("friend"), Some(&1));
    }

    #[test]
    fn handles_weflow_like_user_list_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (user_name TEXT, content TEXT);
             INSERT INTO SnsTimeLine VALUES ('me', '<TimelineObject><username>me</username><createTime>1700000000</createTime><ContentObject><type>1</type></ContentObject><LikeUserList><LikeUser><username>friend</username></LikeUser></LikeUserList></TimelineObject>');
             INSERT INTO SnsTimeLine VALUES ('friend', '<TimelineObject><username>friend</username><createTime>1700000001</createTime><ContentObject><type>2</type></ContentObject><LikeUserList><LikeUser><username>me</username></LikeUser></LikeUserList></TimelineObject>');",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.likes_received, 1);
        assert_eq!(stats.likes_given, 1);
        assert_eq!(stats.top_likers.get("friend"), Some(&1));
        assert_eq!(stats.top_liked_authors.get("friend"), Some(&1));
    }
}

pub fn load_timeline(
    path: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    year: i32,
) -> Result<Vec<serde_json::Value>> {
    let db = Db::open(path, key, plaintext)?;
    let tables = db.tables()?;
    let Some(table) = tables.iter().find(|table| table.eq_ignore_ascii_case("SnsTimeLine") || table.to_ascii_lowercase().contains("snstimeline")) else {
        return Ok(Vec::new());
    };
    let columns = db.columns(table)?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let tid_col = ["tid", "Tid", "ID"].iter().find(|name| has(name)).copied();
    let user_col = ["user_name", "userName", "username"].iter().find(|name| has(name)).copied();
    let content_col = ["content", "Content"].iter().find(|name| has(name)).copied();
    let Some(content_col) = content_col else { return Ok(Vec::new()) };
    let mut fields = Vec::new();
    let tid_index = tid_col.map(|c| { let i = fields.len(); fields.push(crate::db::quote_ident(c)); i });
    let user_index = fields.len();
    if let Some(c) = user_col { fields.push(crate::db::quote_ident(c)); } else { fields.push("''".into()); }
    let content_index = fields.len();
    fields.push(crate::db::quote_ident(content_col));
    let sql = format!("SELECT {} FROM {}", fields.join(", "), crate::db::quote_ident(table));
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let timeline_re = Regex::new(r"(?is)<TimelineObject\b.*?</TimelineObject>")?;
    let media_re = Regex::new(r"(?is)<media\b.*?</media>")?;
    let like_re = Regex::new(r"(?is)<(?:LikeUserList|likeUserList|likeList|like_user_list)\b.*?</(?:LikeUserList|likeUserList|likeList|like_user_list)>")?;
    let username_re = Regex::new(r"(?is)<username\b[^>]*>(.*?)</username>")?;
    let mut out = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        let content = row_content(&row, content_index);
        let Some(timeline) = timeline_re.find(&content).map(|m| m.as_str()) else { continue };
        let xml_time = crate::wechat::parse::xml_value(&content, "createTime")
            .or_else(|| crate::wechat::parse::xml_value(timeline, "createTime"))
            .and_then(|v| v.trim().parse::<i64>().ok());
        if year != 0 {
            let Some(ts) = xml_time else { continue };
            let Some(date) = Local.timestamp_opt(ts, 0).single() else { continue };
            if date.year() != year { continue; }
        }
        let content_type = crate::wechat::parse::xml_value(&content, "contentStyle")
            .or_else(|| {
                Regex::new(r"(?is)<ContentObject\b.*?</ContentObject>")
                    .ok()
                    .and_then(|re| re.find(timeline))
                    .and_then(|m| crate::wechat::parse::xml_value(m.as_str(), "type"))
            })
            .or_else(|| crate::wechat::parse::xml_value(timeline, "contentType"))
            .or_else(|| crate::wechat::parse::xml_value(timeline, "type"))
            .unwrap_or_else(|| "1".into());
        let mut media = Vec::new();
        for block in media_re.find_iter(&content) {
            let piece = block.as_str();
            let url = crate::wechat::parse::xml_value(piece, "url");
            let thumb = crate::wechat::parse::xml_value(piece, "thumb");
            if url.is_none() && thumb.is_none() { continue; }
            media.push(serde_json::json!({
                "url": url,
                "thumb": thumb,
                "md5": crate::wechat::parse::xml_attr(piece, "url", "md5"),
                "width": crate::wechat::parse::xml_attr(piece, "size", "width"),
                "height": crate::wechat::parse::xml_attr(piece, "size", "height"),
            }));
        }
        let likes = like_re.find(&content).map(|block| {
            username_re.captures_iter(block.as_str())
                .filter_map(|c| c.get(1).map(|v| crate::wechat::parse::unescape_xml(v.as_str()).trim().to_string()))
                .filter(|v| !v.is_empty()).collect::<Vec<_>>()
        }).unwrap_or_default();
        let text = crate::wechat::parse::xml_value(&content, "contentDesc")
            .map(|v| crate::wechat::parse::unescape_xml(&v))
            .unwrap_or_default();
        out.push(serde_json::json!({
            "tid": tid_index.and_then(|i| row_i64(&row, i)),
            "author": row_text(&row, user_index),
            "createTime": xml_time,
            "type": content_type,
            "content": text,
            "media": media,
            "likes": likes,
        }));
    }
    out.sort_by(|a, b| b["createTime"].as_i64().cmp(&a["createTime"].as_i64()));
    Ok(out)
}

fn debug_rows(
    path: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    self_username: Option<&str>,
    year: i32,
) -> Result<serde_json::Value> {
    let db = Db::open(path, key, plaintext)?;
    let tables = db.tables()?;
    let Some(table) = tables.iter().find(|table| table.eq_ignore_ascii_case("SnsTimeLine") || table.to_ascii_lowercase().contains("snstimeline")) else {
        return Ok(serde_json::json!({"error": "SnsTimeLine not found", "tables": tables}));
    };
    let columns = db.columns(table)?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let tid_col = ["tid", "Tid", "ID"].iter().find(|name| has(name)).copied();
    let user_col = ["user_name", "userName", "username"].iter().find(|name| has(name)).copied();
    let content_col = ["content", "Content"].iter().find(|name| has(name)).copied();
    let time_col = ["create_time", "createTime", "createtime"].iter().find(|name| has(name)).copied();
    let Some(content_col) = content_col else { return Ok(serde_json::json!({"error":"content column missing"})) };
    let mut fields = Vec::new();
    let mut ti = None;
    if let Some(c) = tid_col { ti = Some(fields.len()); fields.push(crate::db::quote_ident(c)); }
    let ui = fields.len();
    if let Some(c) = user_col { fields.push(crate::db::quote_ident(c)); } else { fields.push("''".into()); }
    let ci = fields.len();
    fields.push(crate::db::quote_ident(content_col));
    let timei = time_col.map(|c| { let i = fields.len(); fields.push(crate::db::quote_ident(c)); i });
    let sql = format!("SELECT {} FROM {}", fields.join(", "), crate::db::quote_ident(table));
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let timeline_re = Regex::new(r"(?is)<TimelineObject\b.*?</TimelineObject>")?;
    let like_re = Regex::new(r"(?is)<(?:LikeUserList|likeUserList|likeList|like_user_list)\b.*?</(?:LikeUserList|likeUserList|likeList|like_user_list)>")?;
    let username_re = Regex::new(r"(?is)<username\b[^>]*>(.*?)</username>")?;
    let self_id = self_username.unwrap_or("").to_lowercase();
    let mut total = 0usize;
    let mut items = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        total += 1;
        let author = row_text(&row, ui);
        if !self_id.is_empty() && author.trim().to_lowercase() != self_id {
            continue;
        }
        let content = row_content(&row, ci);
        let Some(timeline) = timeline_re.find(&content).map(|m| m.as_str()) else { continue };
        let xml_time = crate::wechat::parse::xml_value(&content, "createTime")
            .or_else(|| crate::wechat::parse::xml_value(timeline, "createTime"))
            .and_then(|v| v.trim().parse::<i64>().ok());
        let style = crate::wechat::parse::xml_value(&content, "contentStyle")
            .or_else(|| crate::wechat::parse::xml_value(timeline, "contentStyle"));
        if year != 0 {
            let Some(ts) = xml_time else { continue };
            let Some(date) = Local.timestamp_opt(ts, 0).single() else { continue };
            if date.year() != year { continue; }
        }
        let like_count = like_re.find(&content).map(|block| username_re.captures_iter(block.as_str()).count()).unwrap_or(0);
        let content_type = crate::wechat::parse::xml_value(timeline, "type")
            .or_else(|| crate::wechat::parse::xml_value(&content, "contentType"));
        items.push(serde_json::json!({
            "tid": ti.and_then(|i| row_i64(&row, i)),
            "author": author,
            "xmlCreateTime": xml_time,
            "tableTime": timei.and_then(|i| row_i64(&row, i)),
            "contentStyle": style,
            "contentType": content_type,
            "private": crate::wechat::parse::xml_value(&content, "private"),
            "contentDesc": crate::wechat::parse::xml_value(&content, "contentDesc").map(|v| v.chars().take(80).collect::<String>()),
            "hasMedia": content.contains("<mediaList"),
            "likeCount": like_count,
            "commentCount": content.matches("<comment>").count(),
            "contentLength": content.len(),
            "contentStart": content.chars().take(80).collect::<String>(),
        }));
    }
    let messages = tables.iter().find(|t| t.to_ascii_lowercase().contains("snsmessage")).and_then(|t| {
        let sql = format!("SELECT COUNT(*) FROM {}", crate::db::quote_ident(t));
        db.conn().query_row(&sql, [], |r| r.get::<_, i64>(0)).ok()
    });
    Ok(serde_json::json!({"totalRows": total, "selfPosts": items, "messageRows": messages}))
}

pub fn run(options: &crate::cli::ValidatedCli, layout: &crate::wechat::discover::Layout) -> Result<()> {
    let path = moments_db_path(layout.account_root.as_deref(), layout.db_storage.as_deref())
        .context("sns.db not found; make sure the WeChat account has moments data")?;
    let self_username = options
        .self_username
        .as_deref()
        .map(crate::util::clean_account_id)
        .or_else(|| layout.account_root.as_ref().and_then(|root| root.file_name()).and_then(|name| name.to_str()).map(crate::util::clean_account_id));
    let stats = load_moments_from_path(
        &path,
        options.key.as_ref(),
        options.plaintext,
        self_username.as_deref(),
        options.report_year,
    )?
    .context("no moments posts or likes found for the selected period")?;
    let contacts = crate::wechat::contact::load_contacts(
        layout.contact_db.as_deref(),
        options.key.as_ref(),
        options.plaintext,
    );
    let display = |username: &str| {
        contacts
            .get(username)
            .map(|contact| contact.display_name())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| username.to_string())
    };
    let avatar_url = |username: &str| {
        contacts
            .get(username)
            .map(|contact| contact.avatar_url.trim().to_string())
            .filter(|url| !url.is_empty())
    };
    let mut post_types = stats.post_types.iter().collect::<Vec<_>>();
    post_types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut payload = serde_json::json!({
        "status": "ok",
        "year": options.report_year,
        "posts": stats.posts,
        "likesReceived": stats.likes_received,
        "likesGiven": stats.likes_given,
        "postTypes": post_types.into_iter().map(|(label, count)| serde_json::json!({"contentType": label, "count": count})).collect::<Vec<_>>(),
        "topLikers": ranked(&stats.top_likers, 10).into_iter().map(|(username, count)| serde_json::json!({"username": username, "displayName": display(&username), "avatarUrl": avatar_url(&username), "count": count})).collect::<Vec<_>>(),
        "topLikedAuthors": ranked(&stats.top_liked_authors, 10).into_iter().map(|(username, count)| serde_json::json!({"username": username, "displayName": display(&username), "avatarUrl": avatar_url(&username), "count": count})).collect::<Vec<_>>(),
    });
    if options.debug {
        payload["debug"] = debug_rows(&path, options.key.as_ref(), options.plaintext, self_username.as_deref(), options.report_year)?;
    }
    if options.moments_posts {
        let mut timeline = load_timeline(&path, options.key.as_ref(), options.plaintext, options.report_year)?;
        for item in &mut timeline {
            let username = item["author"].as_str().unwrap_or("").to_string();
            item["displayName"] = serde_json::Value::String(display(&username));
            if let Some(url) = avatar_url(&username) {
                item["avatarUrl"] = serde_json::Value::String(url);
            }
        }
        if let Some(filter) = &options.moments_user {
            let needle = filter.to_lowercase();
            timeline.retain(|item| {
                item["author"].as_str().unwrap_or("").to_lowercase().contains(&needle)
                    || item["displayName"].as_str().unwrap_or("").to_lowercase().contains(&needle)
            });
        }
        let total = timeline.len();
        let offset = options.moments_offset as usize;
        let limit = options.moments_limit as usize;
        let page = timeline.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
        payload["timeline"] = serde_json::Value::Array(page);
        payload["timelineTotal"] = serde_json::json!(total);
    }
    if let Some(dir) = &options.output_dir {
        std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
        let output = dir.join("moments.json");
        std::fs::write(&output, serde_json::to_string_pretty(&payload)?)
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"moments","report":output,"year":options.report_year}))?);
    } else {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    }
    Ok(())
}
