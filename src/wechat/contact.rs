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
        let avatar_expr = [
            "small_head_url",
            "smallheadurl",
            "small_head_img_url",
            "smallheadimgurl",
            "head_img_url",
            "headimgurl",
            "big_head_url",
            "bigheadurl",
            "big_head_img_url",
            "bigheadimgurl",
            "avatar_url",
        ]
        .iter()
        .find(|name| has(name))
        .map(|name| text_expr(name))
        .unwrap_or_else(|| "''".to_string());
        let sql = format!(
            "SELECT {}, {}, {}, {}, {}, {}, {} FROM {}",
            quote_ident(&username),
            text_expr(nickname),
            text_expr("remark"),
            text_expr("alias"),
            num_expr("local_type"),
            num_expr("delete_flag"),
            avatar_expr,
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
                row.get::<_, Option<String>>(6)?,
            ))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            let (Some(username), nickname, remark, alias, local_type, delete_flag, avatar_url) = row else {
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
                avatar_url: avatar_url.unwrap_or_default(),
            };
            out.entry(username).or_insert(contact);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_group_members_from_chatroom_member() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("contact.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE name2id (user_name TEXT);
             INSERT INTO name2id(rowid,user_name) VALUES (1,'wxid_a'),(2,'wxid_b'),(3,'room@chatroom');
             CREATE TABLE chatroom_member (room_id INTEGER, member_id INTEGER);
             INSERT INTO chatroom_member VALUES (3,1),(3,2);",
        )
        .unwrap();
        drop(conn);
        let members = load_group_members(Some(&path), "room@chatroom", None, true);
        assert_eq!(members, vec!["wxid_a", "wxid_b"]);
    }

    #[test]
    fn loads_avatar_url_when_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("contact.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE contact (username TEXT, nick_name TEXT, remark TEXT, alias TEXT, local_type INTEGER, delete_flag INTEGER, small_head_url TEXT);
             INSERT INTO contact VALUES ('wxid_a', 'Alice', '', '', 1, 0, 'http://example.test/a.jpg');",
        )
        .unwrap();
        drop(conn);
        let contacts = load_contacts(Some(&path), None, true);
        let contact = contacts.get("wxid_a").expect("contact");
        assert_eq!(contact.avatar_url, "http://example.test/a.jpg");
    }
}

/// Best-effort full group roster from the contact database.
///
/// WeChat 4 stores the group/member link in `chatroom_member` and maps numeric
/// member ids through `name2id`. Other schema variants simply return an empty
/// list; callers then fall back to senders observed in messages.
pub fn load_group_members(
    path: Option<&Path>,
    chatroom_id: &str,
    key: Option<&[u8; 32]>,
    plaintext: bool,
) -> Vec<String> {
    let Some(path) = path else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let Ok(db) = Db::open(path, key, plaintext) else {
        return Vec::new();
    };
    let Ok(tables) = db.tables() else {
        return Vec::new();
    };
    let Some(member_table) = tables
        .iter()
        .find(|name| {
            name.eq_ignore_ascii_case("chatroom_member")
                || name.eq_ignore_ascii_case("chat_room_member")
        })
        .cloned()
    else {
        return Vec::new();
    };
    let Some(name_table) = tables
        .iter()
        .find(|name| name.eq_ignore_ascii_case("name2id"))
        .cloned()
    else {
        return Vec::new();
    };
    let Ok(member_columns) = db.columns(&member_table) else {
        return Vec::new();
    };
    let member_has = |name: &str| {
        member_columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(name))
    };
    if !(member_has("room_id") && member_has("member_id")) {
        return Vec::new();
    }
    let Ok(name_columns) = db.columns(&name_table) else {
        return Vec::new();
    };
    let name_has = |name: &str| {
        name_columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(name))
    };
    if !name_has("user_name") {
        return Vec::new();
    }
    let sql = format!(
        "SELECT DISTINCT n.user_name FROM {} cm JOIN {} n ON n.rowid = cm.member_id WHERE cm.room_id = (SELECT rowid FROM {} WHERE user_name = ?1 LIMIT 1)",
        quote_ident(&member_table),
        quote_ident(&name_table),
        quote_ident(&name_table)
    );
    let Ok(mut stmt) = db.conn().prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([chatroom_id], |row| row.get::<_, Option<String>>(0)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows.flatten().flatten() {
        let value = row.trim();
        if !value.is_empty() && !out.iter().any(|existing| existing == value) {
            out.push(value.to_string());
        }
    }
    out
}
