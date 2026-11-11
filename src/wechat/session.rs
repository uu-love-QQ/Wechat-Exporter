use anyhow::Result;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
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

/// Read one SessionTable row with all columns as JSON for a raw backup.
pub fn raw_session(
    path: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    username: &str,
) -> Result<Option<Value>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let db = Db::open(path, key, plaintext)?;
    let tables = db.tables()?;
    let Some(table) = tables
        .iter()
        .find(|name| name.eq_ignore_ascii_case("SessionTable"))
        .cloned()
    else {
        return Ok(None);
    };
    let columns = db.columns(&table)?;
    if !columns.iter().any(|column| column.eq_ignore_ascii_case("username")) {
        return Ok(None);
    }
    let select = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select} FROM {} WHERE username = ?1 LIMIT 1",
        quote_ident(&table)
    );
    let mut statement = db.conn().prepare(&sql)?;
    let mut rows = statement.query([username])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(value) => Value::from(value),
            ValueRef::Real(value) => Value::from(value),
            ValueRef::Text(value) => Value::from(String::from_utf8_lossy(value).to_string()),
            ValueRef::Blob(_) => Value::Null,
        };
        object.insert(column.clone(), value);
    }
    Ok(Some(Value::Object(object)))
}
