use anyhow::Result;
use std::path::Path;

use super::model::SessionRow;
use crate::db::{quote_ident, Db};

pub fn load_sessions(
    path: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
) -> Result<Vec<SessionRow>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = Db::open(path, key, plaintext)?;
    let tables = db.tables()?;
    let Some(table) = tables
        .iter()
        .find(|name| name.eq_ignore_ascii_case("SessionTable"))
        .cloned()
    else {
        return Ok(Vec::new());
    };
    let columns = db.columns(&table)?;
    let has = |name: &str| {
        columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(name))
    };
    let col = |name: &str| {
        if has(name) {
            quote_ident(name)
        } else {
            "NULL".to_string()
        }
    };
    let unread = if has("unread_count") {
        "CAST(unread_count AS INTEGER)"
    } else {
        "0"
    };
    let sort_timestamp = if has("sort_timestamp") {
        "CAST(sort_timestamp AS INTEGER)"
    } else if has("last_timestamp") {
        "CAST(last_timestamp AS INTEGER)"
    } else {
        "0"
    };
    let last_timestamp = if has("last_timestamp") {
        "CAST(last_timestamp AS INTEGER)"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT {username}, {unread}, {sort_ts}, {last_ts} FROM {table}",
        username = col("username"),
        unread = unread,
        sort_ts = sort_timestamp,
        last_ts = last_timestamp,
        table = quote_ident(&table),
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(SessionRow {
            username: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            unread_count: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            sort_timestamp: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            last_timestamp: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let row = row?;
        if row.username.is_empty() {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}
