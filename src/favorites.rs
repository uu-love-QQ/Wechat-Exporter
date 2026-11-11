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
        FavoriteType::Article => Some(5),
        FavoriteType::Card => Some(19),
        FavoriteType::Video => Some(20),
        FavoriteType::Other => None,
    }
}

fn type_label(code: i64) -> &'static str {
    match code {
        1 => "文本",
        2 => "图片",
        5 => "文章",
        19 => "名片",
        20 => "视频",
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

fn favorite_preview(content: &str, full: bool) -> (String, Option<String>) {
    let xml_value = |tag: &str| {
        Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}>"))
            .ok()
            .and_then(|re| re.captures(content))
            .and_then(|caps| caps.get(1))
            .map(|m| crate::wechat::parse::unescape_xml(m.as_str()).trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let text = xml_value("title")
        .or_else(|| xml_value("desc"))
        .or_else(|| xml_value("content"))
        .unwrap_or_else(|| content.trim().to_string());
    let preview = if full {
        text.clone()
    } else {
        let mut preview = text.chars().take(200).collect::<String>();
        if text.chars().count() > 200 {
            preview.push_str("...");
        }
        preview
    };
    let url = Regex::new(r#"https?://[^\s"'<>]+"#)
        .ok()
        .and_then(|re| re.find(content))
        .map(|m| m.as_str().to_string());
    (preview, url)
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
        let (preview, url) = favorite_preview(&content, options.favorite_full);
        let mut item = serde_json::json!({
            "id": id_index.map(|index| cell_i64(&row, index)),
            "type": type_label(code),
            "typeNum": code,
            "timestamp": timestamp,
            "time": time,
            "preview": preview,
            "from": from,
            "chat": chat,
        });
        if let Some(url) = url {
            item["url"] = serde_json::Value::String(url);
        }
        if options.favorite_full {
            item["content"] = serde_json::Value::String(content);
        }
        items.push(item);
    }
    items.sort_by(|a, b| b["timestamp"].as_i64().cmp(&a["timestamp"].as_i64()));
    let total = items.len();
    let offset = options.favorite_offset as usize;
    let limit = options.page_size as usize;
    let page = items.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    let value = serde_json::json!({
        "schema": crate::util::SCHEMA,
        "generator": crate::util::GENERATOR,
        "type": match options.favorite_type { FavoriteType::All => "all", FavoriteType::Text => "text", FavoriteType::Image => "image", FavoriteType::Article => "article", FavoriteType::Card => "card", FavoriteType::Video => "video", FavoriteType::Other => "other" },
        "query": options.list_query,
        "total": total,
        "count": page.len(),
        "items": page,
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_favorite_title_preview_and_url() {
        let (preview, url) = favorite_preview(
            "<favitem><title>Article title</title><url>https://example.com/a</url></favitem>",
            false,
        );
        assert_eq!(preview, "Article title");
        assert_eq!(url.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn maps_favorite_types() {
        assert_eq!(type_label(1), "文本");
        assert_eq!(type_label(2), "图片");
        assert_eq!(type_label(5), "文章");
        assert_eq!(type_label(19), "名片");
        assert_eq!(type_label(20), "视频");
        assert_eq!(type_label(99), "其他");
    }
}
