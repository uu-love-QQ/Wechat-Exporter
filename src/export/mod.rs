use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::Format;
use crate::util::{format_iso_timestamp, format_timestamp, GENERATOR, SCHEMA};
use crate::wechat::model::{Conversation, ExportMessage};

pub struct ExportContext<'a> {
    pub conversation: &'a Conversation,
    pub messages: &'a [ExportMessage],
    pub self_username: Option<&'a str>,
    pub self_name: String,
    pub exported_at_ms: i64,
    pub include_media: bool,
}

impl<'a> ExportContext<'a> {
    pub fn exported_at_sec(&self) -> i64 {
        self.exported_at_ms / 1000
    }

    pub fn group(&self) -> bool {
        self.conversation.is_group
    }

    pub fn display_name(&self) -> &str {
        &self.conversation.name
    }
}

pub fn write_conversation(
    context: &ExportContext<'_>,
    output_dir: &Path,
    formats: &[Format],
) -> Result<BTreeMap<String, PathBuf>> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut written = BTreeMap::new();
    for format in formats {
        let path = output_dir.join(format.file_name());
        match format {
            Format::Json => write_json(context, &path)?,
            Format::Markdown => write_markdown(context, &path)?,
            Format::ChatlabJson => write_chatlab_json(context, &path, false)?,
            Format::ChatlabJsonl => write_chatlab_json(context, &path, true)?,
            Format::Html => write_html(context, &path)?,
            Format::Txt => write_txt(context, &path)?,
            Format::Xlsx => write_xlsx(context, &path)?,
            Format::ArkmeJson => write_arkme_json(context, &path)?,
            Format::Sql => write_sql(context, &path)?,
            Format::Weclone => write_weclone(context, &path)?,
        }
        written.insert(format.name().to_string(), path);
    }
    Ok(written)
}

fn session_json(context: &ExportContext<'_>) -> Value {
    let conversation = context.conversation;
    json!({
        "id": conversation.id,
        "name": conversation.name,
        "nickname": conversation.nickname,
        "remark": conversation.remark,
        "alias": conversation.alias,
        "kind": if conversation.is_group { "group" } else { "private" },
        "kindText": if conversation.is_group { "群聊" } else { "私聊" },
        "lastTime": conversation.last_timestamp,
        "messageCount": context.messages.len()
    })
}

pub fn message_json(message: &ExportMessage, include_media: bool) -> Value {
    let mut object = Map::new();
    object.insert("id".into(), json!(message.local_id));
    object.insert(
        "platformId".into(),
        json!(message.platform_message_id().unwrap_or_default()),
    );
    object.insert("time".into(), json!(message.create_time));
    object.insert(
        "timeText".into(),
        json!(format_timestamp(message.create_time)),
    );
    object.insert("type".into(), json!(message.type_name));
    object.insert("typeCode".into(), json!(message.base_type));
    object.insert("content".into(), json!(message.content));
    object.insert("fromMe".into(), json!(message.is_send));
    let mut sender = Map::new();
    sender.insert("id".into(), json!(message.sender_id));
    sender.insert("name".into(), json!(message.sender_name));
    if !message.nickname.is_empty() && message.nickname != message.sender_name {
        sender.insert("nickname".into(), json!(message.nickname));
    }
    if !message.remark.is_empty() {
        sender.insert("remark".into(), json!(message.remark));
    }
    if !message.alias.is_empty() {
        sender.insert("alias".into(), json!(message.alias));
    }
    object.insert("sender".into(), Value::Object(sender));
    if let Some(subtype) = message.app_subtype {
        object.insert("app".into(), json!({
            "type": subtype,
            "kind": app_kind(subtype),
        }));
    }
    if !message.source.notes.is_empty() {
        object.insert("source".into(), json!(message.source.notes));
    }
    if include_media {
        if let Some(media) = &message.media {
            let mut item = Map::new();
            item.insert("kind".into(), json!(media.kind.as_str()));
            if !media.md5.is_empty() {
                item.insert("md5".into(), json!(media.md5));
            }
            if !media.file_name.is_empty() {
                item.insert("name".into(), json!(media.file_name));
            }
            if let Some(path) = &message.media_path {
                item.insert("path".into(), json!(path));
            } else {
                item.insert("status".into(), json!("missing"));
            }
            object.insert("media".into(), Value::Object(item));
        }
    }
    Value::Object(object)
}

fn app_kind(subtype: i64) -> &'static str {
    match subtype {
        3 => "music",
        5 | 49 => "link",
        6 => "file",
        19 => "chat_record",
        33 | 36 => "mini_program",
        57 => "quote",
        87 => "announcement",
        2000 => "transfer",
        _ => "other",
    }
}

fn write_json(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let messages: Vec<Value> = context
        .messages
        .iter()
        .map(|message| message_json(message, context.include_media))
        .collect();
    let value = json!({
        "schema": SCHEMA,
        "generator": GENERATOR,
        "weflow": {
            "version": env!("CARGO_PKG_VERSION"),
            "format": "json",
            "exportedAt": context.exported_at_sec()
        },
        "session": session_json(context),
        "messages": messages
    });
    fs::write(path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn sender_members(context: &ExportContext<'_>) -> Vec<Value> {
    let mut members = Vec::new();
    let mut seen = HashMap::new();
    let self_id = context.self_username.unwrap_or("");
    if !self_id.is_empty() {
        let name = if context.self_name.is_empty() {
            self_id.to_string()
        } else {
            context.self_name.clone()
        };
        seen.insert(self_id.to_string(), 0usize);
        members.push(json!({
            "platformId": self_id,
            "accountName": name
        }));
    }
    for message in context.messages {
        let id = if message.sender_id.is_empty() {
            context.conversation.id.clone()
        } else {
            message.sender_id.clone()
        };
        if seen.contains_key(&id) {
            continue;
        }
        seen.insert(id.clone(), members.len());
        let mut member = Map::new();
        member.insert("platformId".into(), json!(id));
        member.insert("accountName".into(), json!(message.sender_name));
        if !message.alias.is_empty() {
            member.insert("aliases".into(), json!([message.alias]));
        }
        if context.group() && !message.nickname.is_empty() {
            member.insert("groupNickname".into(), json!(message.nickname));
        }
        members.push(Value::Object(member));
    }
    // Include the conversation partner in private chats even when no message
    // was readable (for example a media-only chat).
    let conversation_id = &context.conversation.id;
    if !seen.contains_key(conversation_id) && !context.group() {
        members.push(json!({
            "platformId": conversation_id,
            "accountName": context.conversation.name,
        }));
    }
    members
}

fn chatlab_type(message: &ExportMessage) -> i64 {
    if message.type_name == "系统消息" {
        if message.content.contains("撤回") {
            return 81;
        }
        return 80;
    }
    match message.base_type {
        1 => 0,
        3 => 1,
        34 => 2,
        43 => 3,
        47 => 5,
        48 => 8,
        42 => 27,
        50 => 23,
        49 => match message.app_subtype {
            Some(6) => 4,
            Some(19) => 26,
            Some(57) => 25,
            Some(33) | Some(36) | Some(51) | Some(63) | Some(115) => 24,
            Some(2000) => 21,
            Some(87) => 80,
            _ => {
                if message.content.contains("红包") {
                    20
                } else if message.content.contains("拍一拍") {
                    22
                } else {
                    7
                }
            }
        },
        _ => 99,
    }
}

fn chatlab_message(message: &ExportMessage) -> Value {
    let mut object = Map::new();
    object.insert("sender".into(), json!(message.sender_id));
    object.insert("accountName".into(), json!(message.sender_name));
    if !message.nickname.is_empty() {
        object.insert("groupNickname".into(), json!(message.nickname));
    }
    object.insert("timestamp".into(), json!(message.create_time));
    object.insert("type".into(), json!(chatlab_type(message)));
    object.insert("content".into(), json!(message.content));
    if let Some(platform_id) = message.platform_message_id() {
        object.insert("platformMessageId".into(), json!(platform_id));
    }
    if let Some(path) = &message.media_path {
        object.insert("mediaPath".into(), json!(path));
    }
    Value::Object(object)
}

fn write_chatlab_json(context: &ExportContext<'_>, path: &Path, jsonl: bool) -> Result<()> {
    let meta = json!({
        "name": context.display_name(),
        "platform": "wechat",
        "type": if context.group() { "group" } else { "private" },
        "groupId": context.conversation.id,
    });
    let header = json!({
        "version": "0.0.2",
        "exportedAt": context.exported_at_sec(),
        "generator": GENERATOR,
        "description": format!("WeChat chat export of {}", context.conversation.id)
    });
    if jsonl {
        let mut output = String::new();
        output.push_str(&serde_json::to_string(&json!({
            "_type": "header",
            "chatlab": header,
            "meta": meta,
        }))?);
        output.push('\n');
        for member in sender_members(context) {
            let mut object = member.as_object().cloned().unwrap_or_default();
            object.insert("_type".into(), json!("member"));
            output.push_str(&serde_json::to_string(&Value::Object(object))?);
            output.push('\n');
        }
        for message in context.messages {
            let mut object = chatlab_message(message).as_object().cloned().unwrap_or_default();
            object.insert("_type".into(), json!("message"));
            output.push_str(&serde_json::to_string(&Value::Object(object))?);
            output.push('\n');
        }
        fs::write(path, output).with_context(|| format!("failed to write {}", path.display()))
    } else {
        let value = json!({
            "chatlab": header,
            "meta": meta,
            "members": sender_members(context),
            "messages": context.messages.iter().map(chatlab_message).collect::<Vec<_>>()
        });
        fs::write(path, serde_json::to_vec_pretty(&value)?)
            .with_context(|| format!("failed to write {}", path.display()))
    }
}

fn write_arkme_json(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut sender_ids: HashMap<String, i64> = HashMap::new();
    let mut senders = Vec::new();
    let mut sender_profiles: HashMap<String, (String, String, String, String)> = HashMap::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    for message in context.messages {
        let id = if message.sender_id.is_empty() {
            context.conversation.id.clone()
        } else {
            message.sender_id.clone()
        };
        *counts.entry(id.clone()).or_insert(0) += 1;
        sender_profiles
            .entry(id.clone())
            .or_insert_with(|| {
                (
                    message.sender_name.clone(),
                    message.nickname.clone(),
                    message.remark.clone(),
                    message.alias.clone(),
                )
            });
        if !sender_ids.contains_key(&id) {
            let sender_id = (senders.len() + 1) as i64;
            sender_ids.insert(id.clone(), sender_id);
            let (display, nickname, remark, alias) = sender_profiles.get(&id).cloned().unwrap_or_default();
            let mut sender = Map::new();
            sender.insert("senderID".into(), json!(sender_id));
            sender.insert("accountId".into(), json!(id));
            sender.insert("displayName".into(), json!(display));
            sender.insert("nickname".into(), json!(nickname));
            if !remark.is_empty() {
                sender.insert("remark".into(), json!(remark));
            }
            if !alias.is_empty() {
                sender.insert("alias".into(), json!(alias));
            }
            senders.push(Value::Object(sender));
        }
    }
    let messages: Vec<Value> = context
        .messages
        .iter()
        .map(|message| {
            let id = if message.sender_id.is_empty() {
                context.conversation.id.clone()
            } else {
                message.sender_id.clone()
            };
            let sender_id = sender_ids.get(&id).copied().unwrap_or(1);
            let mut object = message_json(message, context.include_media)
                .as_object()
                .cloned()
                .unwrap_or_default();
            object.insert("senderID".into(), json!(sender_id));
            Value::Object(object)
        })
        .collect();
    let mut value = json!({
        "weflow": {
            "version": env!("CARGO_PKG_VERSION"),
            "format": "arkme-json",
            "exportedAt": context.exported_at_sec(),
            "generator": GENERATOR
        },
        "session": session_json(context),
        "senders": senders,
        "messages": messages
    });
    if context.group() {
        let mut members: Vec<Value> = sender_profiles
            .iter()
            .map(|(id, (display, nickname, remark, alias))| {
                let mut member = Map::new();
                member.insert("accountId".into(), json!(id));
                member.insert("displayName".into(), json!(display));
                member.insert("nickname".into(), json!(nickname));
                member.insert("remark".into(), json!(remark));
                member.insert("alias".into(), json!(alias));
                member.insert("isFriend".into(), json!(false));
                member.insert("messageCount".into(), json!(counts.get(id).copied().unwrap_or(0)));
                Value::Object(member)
            })
            .collect();
        members.sort_by(|a, b| {
            b.get("messageCount")
                .and_then(Value::as_i64)
                .cmp(&a.get("messageCount").and_then(Value::as_i64))
        });
        value
            .as_object_mut()
            .expect("object")
            .insert("groupMembers".into(), Value::Array(members));
    }
    fs::write(path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_markdown(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", context.display_name()));
    out.push_str(&format!("- 会话 ID: `{}`\n", context.conversation.id));
    out.push_str(&format!(
        "- 类型: {}\n",
        if context.group() { "群聊" } else { "私聊" }
    ));
    out.push_str(&format!("- 消息数: {}\n", context.messages.len()));
    out.push_str(&format!(
        "- 导出时间: {}\n\n",
        format_timestamp(context.exported_at_sec())
    ));
    for message in context.messages {
        out.push_str(&format!(
            "## {} {}\n\n",
            format_timestamp(message.create_time),
            message.sender_name
        ));
        if let Some(path) = &message.media_path {
            out.push_str(&format!("![媒体]({path})\n\n"));
        } else if let Some(media) = &message.media {
            if !media.file_name.is_empty() {
                out.push_str(&format!("*[{}: {} (未索引，空缺)]*\n\n", media.kind.label(), media.file_name));
            } else {
                out.push_str(&format!("*[{}: 未索引，空缺]*\n\n", media.kind.label()));
            }
        }
        let content = message.content.trim();
        if !content.is_empty() {
            out.push_str(content);
            out.push_str("\n\n");
        }
    }
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))
}

fn write_txt(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut out = String::new();
    for message in context.messages {
        let content = if let Some(path) = &message.media_path {
            path.as_str()
        } else {
            message.content.as_str()
        };
        out.push_str(&format!(
            "{} '{}'\n{}\n\n",
            format_timestamp(message.create_time),
            message.sender_name,
            content
        ));
    }
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))
}

fn write_sql(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut out = String::new();
    out.push_str(
        "BEGIN;\nCREATE TABLE IF NOT EXISTS weflow_messages (\n  session_id TEXT NOT NULL,\n  local_id TEXT,\n  message_id TEXT,\n  create_time BIGINT NOT NULL,\n  sender TEXT,\n  is_send BOOLEAN NOT NULL,\n  local_type INTEGER,\n  media_type TEXT,\n  content TEXT,\n  media_path TEXT\n);\nCREATE INDEX IF NOT EXISTS idx_weflow_messages_session_time ON weflow_messages (session_id, create_time);\n\n",
    );
    let session = crate::util::escape_sql_string(&context.conversation.id);
    for message in context.messages {
        let media_type = message.media.as_ref().map(|media| media.kind.as_str()).unwrap_or("");
        let media_path = message.media_path.as_deref().unwrap_or("");
        out.push_str(&format!(
            "INSERT INTO weflow_messages (session_id, local_id, message_id, create_time, sender, is_send, local_type, media_type, content, media_path) VALUES ('{}', '{}', '{}', {}, '{}', {}, {}, '{}', '{}', '{}');\n",
            session,
            message.local_id,
            message
                .platform_message_id()
                .unwrap_or_else(|| message.local_id.to_string()),
            message.create_time,
            crate::util::escape_sql_string(&message.sender_id),
            if message.is_send { 1 } else { 0 },
            message.local_type,
            crate::util::escape_sql_string(media_type),
            crate::util::escape_sql_string(&message.content),
            crate::util::escape_sql_string(media_path),
        ));
    }
    out.push_str("COMMIT;\n");
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))
}

fn weclone_type(message: &ExportMessage) -> &'static str {
    match message.base_type {
        1 => "text",
        3 => "image",
        34 => "voice",
        43 => "video",
        47 => "sticker",
        48 => "location",
        49 if message.app_subtype == Some(6) => "file",
        _ => "text",
    }
}

fn write_weclone(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut out = String::from("\u{FEFF}id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n");
    for message in context.messages {
        let src = message
            .media_path
            .clone()
            .or_else(|| message.media.as_ref().map(|media| media.file_name.clone()))
            .unwrap_or_default();
        let talker = if message.is_send {
            if context.self_name.is_empty() {
                "我".to_string()
            } else {
                context.self_name.clone()
            }
        } else {
            message.sender_name.clone()
        };
        let row = [
            message.index.to_string(),
            message
                .platform_message_id()
                .unwrap_or_else(|| message.local_id.to_string()),
            weclone_type(message).to_string(),
            if message.is_send { "1".to_string() } else { "0".to_string() },
            talker,
            message.content.clone(),
            src,
            format_iso_timestamp(message.create_time),
        ];
        out.push_str(
            &row.iter()
                .map(|value| crate::util::escape_csv(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))
}

const CHAT_TEMPLATE: &str = include_str!("chat_template.html");

fn write_html(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut by_server: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut by_local: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for previous in context.messages {
        by_local.entry(previous.local_id.to_string()).or_insert(previous.index);
        if previous.server_id > 0 {
            by_server.entry(previous.server_id.to_string()).or_insert(previous.index);
        }
    }

    let mut messages = Vec::with_capacity(context.messages.len());
    for message in context.messages {
        let timestamp = format_timestamp(message.create_time);
        let day = timestamp.get(..10).unwrap_or(&timestamp).to_string();
        let mut body = message.content.clone();
        let quote = message.quote.as_ref().map(|quote| {
            body = quote.reply.clone();
            let quoted_label = match quote.quoted_type {
                Some(3) => "[图片]",
                Some(34) => "[语音]",
                Some(43) => "[视频]",
                Some(47) => "[表情]",
                Some(49) => "[文件]",
                _ => "[消息]",
            };
            let text = if quote.content.is_empty() {
                quoted_label.to_string()
            } else {
                quote.content.clone()
            };
            let target = quote.server_id.and_then(|id| {
                let key = id.to_string();
                by_server
                    .get(&key)
                    .or_else(|| by_local.get(&key))
                    .copied()
            });
            serde_json::json!({
                "sender": quote.sender,
                "text": text,
                "target": target,
            })
        });
        let media = message.media_path.as_ref().and_then(|path| {
            message.media.as_ref().map(|media| {
                serde_json::json!({
                    "path": path,
                    "kind": media.kind.as_str(),
                    "name": media.file_name,
                })
            })
        });
        let missing = if message.media_path.is_none() {
            message.media.as_ref().map(|media| {
                serde_json::json!({
                    "kind": media.kind.as_str(),
                    "name": media.file_name,
                })
            })
        } else {
            None
        };
        messages.push(serde_json::json!({
            "index": message.index,
            "sender": message.sender_name,
            "time": timestamp,
            "day": day,
            "mine": message.is_send,
            "content": body,
            "quote": quote,
            "media": media,
            "missing": missing,
        }));
    }

    let payload = serde_json::json!({
        "title": context.display_name(),
        "kind": if context.group() { "群聊" } else { "私聊" },
        "id": context.conversation.id,
        "exportedAt": format_timestamp(context.exported_at_sec()),
        "messages": messages,
    });
    let data = serde_json::to_string(&payload).context("serialize chat data")?;
    // `<` keeps `</script>` out of the JSON island.
    let data = data.replace('<', "\\u003c");
    let html = CHAT_TEMPLATE.replace("__CHAT_DATA__", &data);
    fs::write(path, html).with_context(|| format!("failed to write {}", path.display()))
}

fn write_xlsx(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    use rust_xlsxwriter::{Format as XlsxFormat, Workbook};
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let header = XlsxFormat::new().set_bold();
    let headers = [
        "#",
        "时间",
        "发送者",
        "发送者ID",
        "备注",
        "类型",
        "内容",
        "媒体",
    ];
    for (column, value) in headers.iter().enumerate() {
        worksheet
            .write_string_with_format(0, column as u16, *value, &header)
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
    }
    for (row, message) in context.messages.iter().enumerate() {
        let row = row as u32 + 1;
        worksheet
            .write_number(row, 0, message.index as f64)
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 1, format_timestamp(message.create_time))
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 2, message.sender_name.clone())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 3, message.sender_id.clone())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 4, message.remark.clone())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 5, message.type_name.clone())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 6, message.content.clone())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
        worksheet
            .write_string(row, 7, message.media_path.clone().unwrap_or_default())
            .map_err(|error| anyhow::anyhow!("xlsx write error: {error}"))?;
    }
    workbook
        .save(path)
        .map_err(|error| anyhow::anyhow!("failed to write {}: {error}", path.display()))
}

pub fn write_report(
    output_dir: &Path,
    account_id: Option<&str>,
    exported_at_ms: i64,
    formats: &[Format],
    conversations: &[Value],
) -> Result<PathBuf> {
    let path = output_dir.join("export-report.json");
    let value = json!({
        "schema": SCHEMA,
        "generator": GENERATOR,
        "account_id": account_id,
        "exported_at_unix_ms": exported_at_ms,
        "formats": formats.iter().map(|format| format.name()).collect::<Vec<_>>(),
        "conversations": conversations,
    });
    fs::write(&path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}
