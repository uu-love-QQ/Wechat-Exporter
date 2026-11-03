use std::collections::HashMap;
use std::path::Path;

use super::model::Contact;
use crate::db::{quote_ident, Db};
use crate::util::is_group_id;

pub fn load_contacts(
    path: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
) -> HashMap<String, Contact> {
    let Some(path) = path else {
        return HashMap::new();
    };
    if !path.exists() {
        return HashMap::new();
    }
    let Ok(db) = Db::open(path, key, plaintext) else {
        eprintln!("warning: could not open contact database {}", path.display());
        return HashMap::new();
    };
    let Ok(tables) = db.tables() else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for table_name in ["contact", "stranger", "chat_room"] {
        let Some(table) = tables
            .iter()
            .find(|name| name.eq_ignore_ascii_case(table_name))
            .cloned()
        else {
            continue;
        };
        let Ok(columns) = db.columns(&table) else {
            continue;
        };
        let has = |name: &str| {
            columns
                .iter()
                .any(|column| column.eq_ignore_ascii_case(name))
        };
        let text_expr = |name: &str| {
            if has(name) {
                format!("COALESCE({}, '')", quote_ident(name))
            } else {
                "''".to_string()
            }
        };
        let num_expr = |name: &str| {
            if has(name) {
                format!("CAST(COALESCE({}, 0) AS INTEGER)", quote_ident(name))
            } else {
                "0".to_string()
            }
        };
        let username = if has("username") {
            "username".to_string()
        } else if has("user_name") {
            "user_name".to_string()
        } else {
            continue;
        };
        let nickname = if has("nick_name") {
            "nick_name"
        } else if has("nickname") {
            "nickname"
        } else {
            "''"
        };
        let sql = format!(
            "SELECT {}, {}, {}, {}, {}, {} FROM {}",
            quote_ident(&username),
            text_expr(nickname),
            text_expr("remark"),
            text_expr("alias"),
            num_expr("local_type"),
            num_expr("delete_flag"),
            quote_ident(&table)
        );
        let Ok(mut stmt) = db.conn().prepare(&sql) else {
            continue;
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            let (Some(username), nickname, remark, alias, local_type, delete_flag) = row else {
                continue;
            };
            if username.is_empty() {
                continue;
            }
            if delete_flag.unwrap_or(0) != 0 {
                continue;
            }
            let contact = Contact {
                username: username.clone(),
                nickname: nickname.unwrap_or_default(),
                remark: remark.unwrap_or_default(),
                alias: alias.unwrap_or_default(),
                is_group: is_group_id(&username)
                    || table_name == "chat_room"
                    || local_type == Some(2),
            };
            out.entry(username).or_insert(contact);
        }
    }
    out
}
