use anyhow::{Context, Result};
use chrono::{Datelike, Local, TimeZone};
use regex::Regex;
use std::collections::BTreeMap;
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
    let user_col = ["user_name", "userName", "username"].iter().find(|name| has(name)).copied();
    let content_col = ["content", "Content"].iter().find(|name| has(name)).copied();
    let time_col = ["create_time", "createTime", "createtime"].iter().find(|name| has(name)).copied();
    let Some(content_col) = content_col else {
        return Ok(None);
    };

    let mut fields = Vec::new();
    if let Some(column) = user_col {
        fields.push(crate::db::quote_ident(column));
    } else {
        fields.push("''".to_string());
    }
    fields.push(crate::db::quote_ident(content_col));
    if let Some(column) = time_col {
        fields.push(crate::db::quote_ident(column));
    }
    let sql = format!("SELECT {} FROM {}", fields.join(", "), crate::db::quote_ident(&table));
    let mut stmt = db.conn().prepare(&sql).context("failed to prepare moments query")?;
    let mut rows = stmt.query([]).context("failed to query moments")?;

    let self_id = self_username.unwrap_or("").trim().to_lowercase();
    let like_block = Regex::new(r"(?is)<like(?:\s[^>]*)?>(.*?)</like>").expect("valid like regex");
    let username = Regex::new(r"(?is)<username>(.*?)</username>").expect("valid username regex");
    let mut stats = MomentsStats::default();

    while let Ok(Some(row)) = rows.next() {
        let poster = match row.get::<_, Option<rusqlite::types::Value>>(0).ok().flatten() {
            Some(rusqlite::types::Value::Text(value)) => value,
            Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
            _ => String::new(),
        };
        let content = match row.get::<_, Option<rusqlite::types::Value>>(1).ok().flatten() {
            Some(rusqlite::types::Value::Text(value)) => value,
            Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
            _ => String::new(),
        };
        let table_time = if time_col.is_some() {
            match row.get::<_, Option<rusqlite::types::Value>>(2).ok().flatten() {
                Some(rusqlite::types::Value::Integer(value)) => value,
                Some(rusqlite::types::Value::Text(value)) => value.trim().parse::<i64>().unwrap_or(0),
                _ => 0,
            }
        } else {
            0
        };
        let timestamp = crate::wechat::parse::xml_value(&content, "createTime")
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
        let poster = if poster.trim().is_empty() {
            crate::wechat::parse::xml_value(&content, "username")
                .map(|value| crate::wechat::parse::unescape_xml(&value))
                .unwrap_or_default()
        } else {
            poster
        };
        let poster_id = poster.trim().to_lowercase();
        let likes = like_block
            .captures_iter(&content)
            .flat_map(|block| {
                let inner = block.get(1).map(|value| value.as_str()).unwrap_or("");
                username
                    .captures_iter(inner)
                    .filter_map(|capture| capture.get(1).map(|value| crate::wechat::parse::unescape_xml(value.as_str()).trim().to_string()))
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        if !self_id.is_empty() && poster_id == self_id {
            stats.posts += 1;
            stats.likes_received += likes.len();
            let content_type = crate::wechat::parse::xml_value(&content, "contentType")
                .or_else(|| crate::wechat::parse::xml_value(&content, "type"))
                .unwrap_or_else(|| "0".to_string());
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
            *stats.post_types.entry(label.to_string()).or_default() += 1;
            for liker in likes {
                if liker.trim().to_lowercase() != self_id {
                    *stats.top_likers.entry(liker).or_default() += 1;
                }
            }
        } else if !self_id.is_empty() && likes.iter().any(|liker| liker.trim().to_lowercase() == self_id) {
            stats.likes_given += 1;
            if !poster_id.is_empty() && poster_id != self_id {
                *stats.top_liked_authors.entry(poster).or_default() += 1;
            }
        }
    }

    if stats.posts == 0 && stats.likes_given == 0 {
        return Ok(None);
    }
    Ok(Some(stats))
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
        assert_eq!(stats.likes_received, 3);
        assert_eq!(stats.likes_given, 1);
        assert_eq!(stats.top_likers.get("friend"), Some(&1));
        assert_eq!(stats.top_likers.get("other"), Some(&1));
        assert_eq!(stats.top_likers.get("me"), None);
        assert_eq!(stats.top_liked_authors.get("friend"), Some(&1));
    }
}

pub fn run(options: &crate::cli::ValidatedCli, layout: &crate::wechat::discover::Layout) -> Result<()> {
    let path = moments_db_path(layout.account_root.as_deref(), layout.db_storage.as_deref())
        .context("sns.db not found; make sure the WeChat account has moments data")?;
    let self_username = options
        .self_username
        .clone()
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
