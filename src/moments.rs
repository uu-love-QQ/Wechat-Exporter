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
        let timestamp = crate::wechat::parse::xml_value(timeline, "createTime")
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(table_time);
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
        let content_type = crate::wechat::parse::xml_value(timeline, "contentStyle")
            .or_else(|| content_object.and_then(|value| crate::wechat::parse::xml_value(value, "type")))
            .or_else(|| crate::wechat::parse::xml_value(timeline, "contentType"))
            .or_else(|| crate::wechat::parse::xml_value(timeline, "type"))
            .unwrap_or_else(|| "1".to_string());
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
            .find(timeline)
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

    // Outgoing likes are not part of the incoming notification feed on some
    // WeChat builds; recover them from SnsMessage_tmp3 when available.
    if let Some(message_table) = tables
        .iter()
        .find(|table| table.eq_ignore_ascii_case("SnsMessage_tmp3") || table.to_ascii_lowercase().contains("snsmessage"))
    {
        if let Ok(columns) = db.columns(message_table) {
            let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
            let feed_col = ["feed_id", "feedId"].iter().find(|name| has(name)).copied();
            let type_col = ["type", "Type"].iter().find(|name| has(name)).copied();
            let from_col = ["from_username", "fromUsername"].iter().find(|name| has(name)).copied();
            if let (Some(feed_col), Some(type_col), Some(from_col)) = (feed_col, type_col, from_col) {
                let del_filter = if has("del_status") { " WHERE COALESCE(del_status, 0) = 0" } else { "" };
                let sql = format!(
                    "SELECT {}, {}, {} FROM {}{}",
                    crate::db::quote_ident(feed_col),
                    crate::db::quote_ident(type_col),
                    crate::db::quote_ident(from_col),
                    crate::db::quote_ident(message_table),
                    del_filter
                );
                if let Ok(mut stmt) = db.conn().prepare(&sql) {
                    if let Ok(mut rows) = stmt.query([]) {
                        while let Ok(Some(row)) = rows.next() {
                            if row_i64(&row, 1) != Some(1) {
                                continue;
                            }
                            if row_text(&row, 2).trim().to_lowercase() != self_id {
                                continue;
                            }
                            let Some(feed_id) = row_i64(&row, 0) else { continue };
                            let canonical = if posts.contains_key(&feed_id) { feed_id } else if posts.contains_key(&-feed_id) { -feed_id } else { feed_id.abs() };
                            let Some((author, is_self, _)) = posts.get(&canonical) else { continue };
                            if *is_self || author.is_empty() || author.to_lowercase() == self_id {
                                continue;
                            }
                            if seen_likes.insert((canonical, self_id.clone())) {
                                stats.likes_given += 1;
                                *stats.top_liked_authors.entry(author.clone()).or_default() += 1;
                            }
                        }
                    }
                }
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
    fn recovers_outgoing_likes_from_notifications() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (tid INTEGER PRIMARY KEY, user_name TEXT, content TEXT);
             CREATE TABLE SnsMessage_tmp3 (feed_id INTEGER, type INTEGER, from_username TEXT, del_status INTEGER);
             INSERT INTO SnsTimeLine VALUES (100, 'me', '<TimelineObject><createTime>1700000000</createTime><contentStyle>1</contentStyle></TimelineObject>');
             INSERT INTO SnsTimeLine VALUES (101, 'friend', '<TimelineObject><createTime>1700000001</createTime><contentStyle>2</contentStyle></TimelineObject>');
             INSERT INTO SnsMessage_tmp3 VALUES (101, 1, 'me', 0), (101, 1, 'me', 1);",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.likes_received, 0);
        assert_eq!(stats.likes_given, 1);
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
    let mut post_types = stats.post_types.iter().collect::<Vec<_>>();
    post_types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let payload = serde_json::json!({
        "status": "ok",
        "year": options.report_year,
        "posts": stats.posts,
        "likesReceived": stats.likes_received,
        "likesGiven": stats.likes_given,
        "postTypes": post_types.into_iter().map(|(label, count)| serde_json::json!({"contentType": label, "count": count})).collect::<Vec<_>>(),
        "topLikers": ranked(&stats.top_likers, 10).into_iter().map(|(username, count)| serde_json::json!({"username": username, "displayName": display(&username), "count": count})).collect::<Vec<_>>(),
        "topLikedAuthors": ranked(&stats.top_liked_authors, 10).into_iter().map(|(username, count)| serde_json::json!({"username": username, "displayName": display(&username), "count": count})).collect::<Vec<_>>(),
    });
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
