use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
use regex::Regex;

use crate::cli::{FavoriteType, ValidatedCli};
use crate::db::{quote_ident, Db};
use crate::wechat::discover::Layout;

fn type_code(kind: FavoriteType) -> Option<i64> {
    match kind {
        FavoriteType::All => None,
        FavoriteType::Text => Some(1),
        FavoriteType::Image => Some(2),
        FavoriteType::Voice => Some(3),
        FavoriteType::Video => Some(4),
        FavoriteType::Article => Some(5),
        FavoriteType::File => Some(8),
        FavoriteType::ChatRecord => Some(14),
        FavoriteType::Location => Some(6),
        FavoriteType::Note => Some(16),
        FavoriteType::MiniProgram => Some(17),
        FavoriteType::Card => Some(19),
        FavoriteType::VideoChannel => Some(20),
        FavoriteType::Other => None,
    }
}

fn type_label(code: i64) -> &'static str {
    match code {
        1 => "文本",
        2 => "图片",
        3 => "语音",
        4 => "视频",
        5 => "文章",
        6 => "位置",
        8 => "文件",
        14 => "聊天记录",
        16 => "笔记",
        17 => "小程序",
        19 => "名片",
        20 => "视频号",
        _ => "其他",
    }
}

fn cell_text(row: &rusqlite::Row<'_>, index: usize) -> String {
    match row.get::<_, Option<rusqlite::types::Value>>(index).ok().flatten() {
        Some(rusqlite::types::Value::Text(value)) => value,
        Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        _ => String::new(),
    }
}

fn cell_i64(row: &rusqlite::Row<'_>, index: usize) -> i64 {
    match row.get::<_, Option<rusqlite::types::Value>>(index).ok().flatten() {
        Some(rusqlite::types::Value::Integer(value)) => value,
        Some(rusqlite::types::Value::Text(value)) => value.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

pub fn run(options: &ValidatedCli, layout: &Layout) -> Result<()> {
    let path = layout
        .favorite_db
        .as_deref()
        .context("favorite.db not found; make sure WeChat favorites are present")?;
    let db = Db::open(path, options.key.as_ref(), options.plaintext)
        .with_context(|| format!("failed to open favorite database {}", path.display()))?;
    let tables = db.tables()?;
    let table = tables
        .iter()
        .find(|table| table.eq_ignore_ascii_case("fav_db_item") || table.to_ascii_lowercase().contains("fav_db_item"))
        .context("fav_db_item table not found")?
        .clone();
    let columns = db.columns(&table)?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let col = |names: &[&'static str]| -> Option<&'static str> {
        names.iter().find(|name| has(name)).copied()
    };
    let id_col = col(&["local_id", "localId", "id"]);
    let type_col = col(&["type", "Type"]).context("favorite type column missing")?;
    let time_col = col(&["update_time", "updateTime", "create_time", "createTime"]);
    let content_col = col(&["content", "Content"]).context("favorite content column missing")?;
    let from_col = col(&["fromusr", "from_username", "fromUser"]);
    let chat_col = col(&["realchatname", "chat_name", "chatName"]);

    let mut fields = Vec::new();
    let id_index = id_col.map(|column| { let index = fields.len(); fields.push(quote_ident(column)); index });
    let type_index = { let index = fields.len(); fields.push(quote_ident(type_col)); index };
    let time_index = time_col.map(|column| { let index = fields.len(); fields.push(quote_ident(column)); index });
    let content_index = { let index = fields.len(); fields.push(quote_ident(content_col)); index };
    let from_index = from_col.map(|column| { let index = fields.len(); fields.push(quote_ident(column)); index });
    let chat_index = chat_col.map(|column| { let index = fields.len(); fields.push(quote_ident(column)); index });
    let sql = format!("SELECT {} FROM {}", fields.join(", "), quote_ident(&table));
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut items = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        let code = cell_i64(&row, type_index);
        if options.favorite_type == FavoriteType::Other {
            if matches!(code, 1 | 2 | 5 | 19 | 20) {
                continue;
            }
        } else if let Some(wanted) = type_code(options.favorite_type) {
            if code != wanted {
                continue;
            }
        }
        let content = cell_text(&row, content_index);
        let from = from_index.map(|index| cell_text(&row, index)).unwrap_or_default();
        let chat = chat_index.map(|index| cell_text(&row, index)).unwrap_or_default();
        if let Some(query) = &options.list_query {
            let needle = query.to_lowercase();
            if !content.to_lowercase().contains(&needle)
                && !from.to_lowercase().contains(&needle)
                && !chat.to_lowercase().contains(&needle)
            {
                continue;
            }
        }
        let timestamp = time_index.map(|index| cell_i64(&row, index)).unwrap_or(0);
        let timestamp = if timestamp > 9_999_999_999 { timestamp / 1000 } else { timestamp };
        let time = Local
            .timestamp_opt(timestamp, 0)
            .single()
            .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        let details = favorite_details(&content, code);
        let preview = if options.favorite_full {
            details.description.clone()
        } else {
            let mut preview = details.description.chars().take(200).collect::<String>();
            if details.description.chars().count() > 200 {
                preview.push_str("...");
            }
            preview
        };
        let mut item = serde_json::json!({
            "id": id_index.map(|index| cell_i64(&row, index)),
            "type": type_label(code),
            "typeNum": code,
            "timestamp": timestamp,
            "time": time,
            "title": details.title,
            "description": details.description,
            "preview": preview,
            "url": details.url,
            "thumb": details.thumb,
            "duration": details.duration,
            "fileSize": details.file_size,
            "messages": details.messages,
            "from": from,
            "chat": chat,
        });
        if options.favorite_full || options.output_dir.is_some() {
            item["content"] = serde_json::Value::String(content);
        }
        items.push(item);
    }
    items.sort_by(|a, b| b["timestamp"].as_i64().cmp(&a["timestamp"].as_i64()));
    let total = items.len();
    if let Some(dir) = &options.output_dir {
        std::fs::create_dir_all(dir)?;
        let items_dir = dir.join("favorites");
        std::fs::create_dir_all(&items_dir)?;
        for item in &items {
            let id = item["id"].as_i64().unwrap_or(0);
            let type_name = item["type"].as_str().unwrap_or("item");
            let content = item["content"].as_str().unwrap_or("");
            let base = items_dir.join(format!("{id}-{type_name}"));
            std::fs::write(base.with_extension("xml"), content)?;
            if item["typeNum"].as_i64() == Some(14) {
                std::fs::write(base.with_extension("json"), serde_json::to_string_pretty(&item["messages"])?)?;
            }
        }
        let report = dir.join("favorites.json");
        let value = serde_json::json!({
            "schema": crate::util::SCHEMA,
            "generator": crate::util::GENERATOR,
            "total": total,
            "items": items,
        });
        std::fs::write(&report, serde_json::to_string_pretty(&value)?)?;
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"favorites","report":report,"total":total}))?);
    } else {
        let offset = options.favorite_offset as usize;
        let limit = options.page_size as usize;
        let page = items.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
        let value = serde_json::json!({
            "schema": crate::util::SCHEMA,
            "generator": crate::util::GENERATOR,
            "type": favorite_type_name(options.favorite_type),
            "query": options.list_query,
            "total": total,
            "count": page.len(),
            "items": page,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn favorite_type_name(kind: FavoriteType) -> &'static str {
    match kind {
        FavoriteType::All => "all",
        FavoriteType::Text => "text",
        FavoriteType::Image => "image",
        FavoriteType::Voice => "voice",
        FavoriteType::Video => "video",
        FavoriteType::Article => "article",
        FavoriteType::File => "file",
        FavoriteType::ChatRecord => "chat-record",
        FavoriteType::Location => "location",
        FavoriteType::Note => "note",
        FavoriteType::MiniProgram => "mini-program",
        FavoriteType::Card => "card",
        FavoriteType::VideoChannel => "video-channel",
        FavoriteType::Other => "other",
    }
}

#[derive(Default)]
struct FavoriteDetails {
    title: String,
    description: String,
    url: Option<String>,
    thumb: Option<String>,
    duration: Option<String>,
    file_size: Option<String>,
    messages: Vec<serde_json::Value>,
}

fn favorite_details(content: &str, code: i64) -> FavoriteDetails {
    let field = |tag: &str| {
        Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}>"))
            .ok()
            .and_then(|re| re.captures(content))
            .and_then(|caps| caps.get(1))
            .map(|m| crate::wechat::parse::unescape_xml(m.as_str()).trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let mut details = FavoriteDetails::default();
    match code {
        1 => {
            details.title = field("title").unwrap_or_else(|| "文本笔记".into());
            details.description = field("desc").or_else(|| field("content")).unwrap_or_else(|| content.trim().to_string());
        }
        2 => {
            details.title = field("datatitle").unwrap_or_else(|| "收藏图片".into());
            details.description = field("datadesc").unwrap_or_default();
        }
        3 => {
            details.title = "语音消息".into();
            details.description = field("datadesc").unwrap_or_else(|| "语音收藏".into());
            details.duration = field("duration");
            details.file_size = field("fullsize");
        }
        4 => {
            details.title = field("datatitle").unwrap_or_else(|| "收藏视频".into());
            details.description = field("datadesc").unwrap_or_default();
        }
        5 => {
            details.title = field("pagetitle").unwrap_or_else(|| "收藏文章".into());
            details.description = field("pagedesc").unwrap_or_default();
            details.url = field("link").or_else(|| field("url"));
            details.thumb = field("pagethumb_url").or_else(|| field("thumburl"));
        }
        6 => {
            details.title = field("poiname").unwrap_or_else(|| "位置收藏".into());
            details.description = field("poiaddress").or_else(|| field("label")).unwrap_or_default();
        }
        8 => {
            details.title = field("datatitle").unwrap_or_else(|| "收藏文件".into());
            details.description = field("datadesc").unwrap_or_default();
            details.file_size = field("fullsize");
        }
        14 => {
            if let Ok(re) = Regex::new(r"(?is)<dataitem[^>]*>(.*?)</dataitem>") {
                for item in re.captures_iter(content) {
                    let block = item.get(1).map(|m| m.as_str()).unwrap_or("");
                    let desc = Regex::new(r"(?is)<datadesc[^>]*>(.*?)</datadesc>").ok().and_then(|r| r.captures(block)).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or("");
                    let source = Regex::new(r"(?is)<datasrcname[^>]*>(.*?)</datasrcname>").ok().and_then(|r| r.captures(block)).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or("");
                    let time = Regex::new(r"(?is)<datasrctime[^>]*>(.*?)</datasrctime>").ok().and_then(|r| r.captures(block)).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or("").to_string();
                    let desc = crate::wechat::parse::unescape_xml(desc).trim().to_string();
                    if desc.is_empty() { continue; }
                    let source = crate::wechat::parse::unescape_xml(source).trim().to_string();
                    details.messages.push(serde_json::json!({
                        "source": source,
                        "time": time,
                        "text": desc,
                    }));
                }
            }
            details.title = format!("聊天记录 ({} 条消息)", details.messages.len());
            details.description = details.messages.iter().map(|m| m["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("
");
        }
        16 => {
            details.title = field("datatitle").unwrap_or_else(|| "收藏笔记".into());
            details.description = field("datadesc").unwrap_or_default();
        }
        17 => {
            details.title = field("pagetitle").unwrap_or_else(|| "收藏小程序".into());
            details.description = field("pagedesc").unwrap_or_default();
            details.url = field("link").or_else(|| field("url"));
        }
        _ => {
            details.title = field("title").or_else(|| field("datatitle")).or_else(|| field("pagetitle")).unwrap_or_default();
            details.description = field("desc").or_else(|| field("datadesc")).or_else(|| field("pagedesc")).unwrap_or_else(|| content.trim().to_string());
        }
    }
    details.thumb = details.thumb.or_else(|| field("thumburl")).filter(|url| url.starts_with("http"));
    details
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_favorite_title_preview_and_url() {
        let details = favorite_details(
            "<favitem><pagetitle>Article title</pagetitle><link>https://example.com/a</link></favitem>",
            5,
        );
        assert_eq!(details.title, "Article title");
        assert_eq!(details.url.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn maps_favorite_types() {
        assert_eq!(type_label(1), "文本");
        assert_eq!(type_label(2), "图片");
        assert_eq!(type_label(5), "文章");
        assert_eq!(type_label(3), "语音");
        assert_eq!(type_label(4), "视频");
        assert_eq!(type_label(14), "聊天记录");
        assert_eq!(type_label(19), "名片");
        assert_eq!(type_label(99), "其他");
    }
}
