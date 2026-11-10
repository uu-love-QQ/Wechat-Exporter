use std::collections::HashMap;
use std::path::Path;

use super::model::Contact;
use crate::db::{quote_ident, Db};
use crate::util::is_group_id;

fn load_official_types(db: &Db, tables: &[String]) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let Some(table) = tables.iter().find(|name| name.to_ascii_lowercase().contains("biz_info")) else {
        return out;
    };
    let Ok(columns) = db.columns(table) else { return out };
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let user = ["username", "user_name"].iter().find(|n| has(n)).copied();
    let kind = ["type", "account_type", "accountType"].iter().find(|n| has(n)).copied();
    let (Some(user), Some(kind)) = (user, kind) else { return out };
    let sql = format!(
        "SELECT {}, {} FROM {}",
        quote_ident(user), quote_ident(kind), quote_ident(table)
    );
    let Ok(mut stmt) = db.conn().prepare(&sql) else { return out };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<rusqlite::types::Value>>(1)?))
    }) else { return out };
    for (user, kind) in rows.flatten() {
        let Some(user) = user.filter(|user| !user.is_empty()) else { continue };
        let kind = match kind {
            Some(rusqlite::types::Value::Integer(value)) => Some(value),
            Some(rusqlite::types::Value::Text(value)) => value.trim().parse::<i64>().ok(),
            _ => None,
        };
        if let Some(kind) = kind {
            out.insert(user, kind);
        }
    }
    out
}

fn official_kind(value: Option<i64>) -> String {
    match value {
        Some(0) => "subscription".to_string(),
        Some(1) => "service".to_string(),
        Some(2) | Some(3) => "enterprise".to_string(),
        _ => "unknown".to_string(),
    }
}

fn classify_contact(
    username: &str,
    table_name: &str,
    local_type: i64,
    flag: i64,
    quan_pin: &str,
    alias: &str,
    remark: &str,
    official_type: Option<i64>,
) -> (String, bool, bool, bool) {
    let lower = username.to_lowercase();
    let is_group = username.ends_with("@chatroom") || table_name == "chat_room" || local_type == 2;
    let is_openim_enterprise = lower.contains("@openim") && !lower.contains("@kefu.openim");
    let is_enterprise = is_openim_enterprise || local_type == 5;
    let is_official = username.starts_with("gh_")
        && local_type == 1
        && username != "gh_f0a92aa7146c";
    let is_blocked = !is_group && !is_official && (flag & 8) == 8;
    let is_former = !is_group
        && !is_official
        && !is_blocked
        && !username.starts_with("gh_")
        && ((local_type == 0 && !quan_pin.trim().is_empty())
            || (local_type == 3 && flag != 4 && (!quan_pin.trim().is_empty() || !alias.trim().is_empty() || !remark.trim().is_empty())));
    let kind = if is_group {
        "group"
    } else if is_official {
        "official"
    } else if is_blocked {
        "blocked"
    } else if is_former {
        "former_friend"
    } else if is_openim_enterprise || local_type == 1 {
        "friend"
    } else {
        "other"
    };
    let _ = official_type;
    (kind.to_string(), is_enterprise, is_blocked, is_former)
}

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
    let official_types = load_official_types(&db, &tables);
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
        let local_name = ["local_type", "localType"].iter().find(|name| has(name)).copied();
        let flag_name = ["flag", "contact_flag", "contactFlag"].iter().find(|name| has(name)).copied();
        let quan_name = ["quan_pin", "quanPin"].iter().find(|name| has(name)).copied();
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
            "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {} FROM {}",
            quote_ident(&username),
            text_expr(nickname),
            text_expr("remark"),
            text_expr("alias"),
            local_name.map(num_expr).unwrap_or_else(|| "0".to_string()),
            num_expr("delete_flag"),
            avatar_expr,
            flag_name.map(num_expr).unwrap_or_else(|| "0".to_string()),
            quan_name.map(text_expr).unwrap_or_else(|| "''".to_string()),
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
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            let (Some(username), nickname, remark, alias, local_type, delete_flag, avatar_url, flag, quan_pin) = row else {
                continue;
            };
            if username.is_empty() {
                continue;
            }
            let _ = delete_flag;
            let remark = remark.unwrap_or_default();
            let alias = alias.unwrap_or_default();
            let local_type = local_type.unwrap_or(0);
            let flag = flag.unwrap_or(0);
            let quan_pin = quan_pin.unwrap_or_default();
            let official_type = official_types.get(&username).copied();
            let (kind, is_enterprise, is_blocked, is_former_friend) = classify_contact(
                &username, table_name, local_type, flag, &quan_pin, &alias, &remark, official_type,
            );
            let contact = Contact {
                username: username.clone(),
                nickname: nickname.unwrap_or_default(),
                remark,
                alias,
                is_group: is_group_id(&username) || table_name == "chat_room" || local_type == 2,
                avatar_url: avatar_url.unwrap_or_default(),
                kind,
                official_account_kind: if username.starts_with("gh_") && local_type == 1 {
                    Some(official_kind(official_type))
                } else {
                    None
                },
                official_account_type: if username.starts_with("gh_") && local_type == 1 { official_type } else { None },
                is_enterprise,
                is_blocked,
                is_former_friend,
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
    fn classifies_official_enterprise_former_and_blocked() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("contact.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE contact (username TEXT, nick_name TEXT, remark TEXT, alias TEXT, local_type INTEGER, delete_flag INTEGER, small_head_url TEXT, flag INTEGER, quan_pin TEXT);
             CREATE TABLE biz_info (username TEXT, type INTEGER);
             INSERT INTO contact VALUES ('wxid_friend','Friend','','',1,0,'',0,'');
             INSERT INTO contact VALUES ('wxid_former','Former','','',0,0,'',0,'CengJing');
             INSERT INTO contact VALUES ('wxid_blocked','Blocked','','',1,0,'',8,'');
             INSERT INTO contact VALUES ('gh_service','Service','','',1,0,'',0,'');
             INSERT INTO contact VALUES ('wxid_ent@openim','Enterprise','','',5,0,'',0,'');
             INSERT INTO contact VALUES ('room@chatroom','EntGroup','','',5,0,'',0,'');
             INSERT INTO biz_info VALUES ('gh_service',1);",
        )
        .unwrap();
        drop(conn);
        let contacts = load_contacts(Some(&path), None, true);
        assert_eq!(contacts.get("wxid_friend").map(|c| c.kind.as_str()), Some("friend"));
        assert!(contacts.get("wxid_former").map(|c| c.is_former_friend).unwrap_or(false));
        assert!(contacts.get("wxid_blocked").map(|c| c.is_blocked).unwrap_or(false));
        assert_eq!(contacts.get("gh_service").and_then(|c| c.official_account_kind.as_deref()), Some("service"));
        assert!(contacts.get("wxid_ent@openim").map(|c| c.is_enterprise && !c.is_group).unwrap_or(false));
        assert!(contacts.get("room@chatroom").map(|c| c.is_enterprise && c.is_group).unwrap_or(false));
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
