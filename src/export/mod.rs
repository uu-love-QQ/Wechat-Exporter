use anyhow::{Context, Result};
use chrono::{Datelike, Local, TimeZone, Timelike};
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
    pub avatar_path: Option<String>,
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
    report: bool,
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
    if report {
        let path = output_dir.join(REPORT_FILE_NAME);
        write_analytics_report(context, &path)?;
        written.insert("report".to_string(), path);
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

pub const REPORT_FILE_NAME: &str = "conversation.report.html";

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
                    "width": media.width,
                    "height": media.height,
                    "durationMs": media.duration_ms,
                })
            })
        });
        let record = message.record.as_ref().map(|record| {
            serde_json::json!({
                "title": record.title,
                "desc": record.desc,
                "items": record.items.iter().map(|item| {
                    serde_json::json!({
                        "type": item.data_type,
                        "text": item.text,
                        "sender": item.sender,
                        "time": item.time,
                    })
                }).collect::<Vec<_>>(),
            })
        });
        let missing = if message.media_path.is_none() {
            message.media.as_ref().map(|media| {
                serde_json::json!({
                    "kind": media.kind.as_str(),
                    "name": media.file_name,
                    "durationMs": media.duration_ms,
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
            "record": record,
            "media": media,
            "missing": missing,
        }));
    }

    let payload = serde_json::json!({
        "title": context.display_name(),
        "kind": if context.group() { "群聊" } else { "私聊" },
        "isGroup": context.group(),
        "id": context.conversation.id,
        "exportedAt": format_timestamp(context.exported_at_sec()),
        "avatar": context.avatar_path,
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

const REPORT_CSS: &str = r#"
:root {
  --bg: #eef3fb;
  --panel: #ffffff;
  --panel-2: #f5f8fe;
  --fg: #0f172a;
  --muted: #64748b;
  --line: #e2e8f0;
  --accent: #2563eb;
  --accent-soft: #dbeafe;
  --me: #2563eb;
  --other: #10b981;
  --c1: #2563eb;
  --c2: #10b981;
  --c3: #f59e0b;
  --c4: #8b5cf6;
  --c5: #ec4899;
  --c6: #06b6d4;
  --c7: #ef4444;
  --c8: #84cc16;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #080d18;
    --panel: #111827;
    --panel-2: #182235;
    --fg: #e5e7eb;
    --muted: #94a3b8;
    --line: #263244;
    --accent: #60a5fa;
    --accent-soft: #1e3a5f;
    --me: #60a5fa;
    --other: #34d399;
    --c1: #60a5fa;
    --c2: #34d399;
    --c3: #fbbf24;
    --c4: #a78bfa;
    --c5: #f472b6;
    --c6: #22d3ee;
    --c7: #f87171;
    --c8: #a3e635;
  }
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background: radial-gradient(1200px 600px at 50% -10%, rgba(37, 99, 235, .12), transparent 60%), var(--bg);
  color: var(--fg);
  font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
    "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
  -webkit-font-smoothing: antialiased;
}
main { max-width: 1020px; margin: 0 auto; padding: 22px 16px 56px; }
h1, h2, h3, p { margin: 0; }
h1 { font-size: 26px; line-height: 1.2; letter-spacing: -.02em; font-weight: 700; }
h2 { font-size: 16px; line-height: 1.35; letter-spacing: -.01em; }
h3 { font-size: 13px; line-height: 1.35; color: var(--muted); font-weight: 600; }
.muted { color: var(--muted); }
.hero {
  position: relative;
  overflow: hidden;
  border-radius: 24px;
  padding: 24px;
  color: #fff;
  background: linear-gradient(135deg, #312e81 0%, #2563eb 52%, #0284c7 100%);
  box-shadow: 0 18px 45px -22px rgba(37, 99, 235, .75);
}
.hero::after {
  content: "";
  position: absolute;
  right: -50px;
  top: -70px;
  width: 220px;
  height: 220px;
  border-radius: 50%;
  border: 1px solid rgba(255, 255, 255, .18);
  box-shadow: 0 0 0 28px rgba(255, 255, 255, .045), 0 0 0 58px rgba(255, 255, 255, .03);
  pointer-events: none;
}
.eyebrow {
  color: rgba(255, 255, 255, .72);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: .16em;
  text-transform: uppercase;
}
.hero h1 { margin: 10px 0 16px; max-width: 34ch; }
.hero-total { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
.hero-total strong { font-size: 42px; line-height: 1; letter-spacing: -.04em; font-variant-numeric: tabular-nums; }
.hero-total span { color: rgba(255, 255, 255, .8); font-size: 14px; }
.hero-meta { margin-top: 14px; color: rgba(255, 255, 255, .72); font-size: 12px; display: flex; flex-wrap: wrap; gap: 6px 16px; }
.kpis {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  margin: 14px 0;
}
.kpi, .card {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 18px;
}
.kpi { padding: 14px 16px; min-width: 0; }
.kpi span { display: block; color: var(--muted); font-size: 11px; }
.kpi strong { display: block; margin-top: 3px; font-size: 25px; line-height: 1.2; letter-spacing: -.03em; font-variant-numeric: tabular-nums; }
.kpi small { display: block; margin-top: 2px; color: var(--muted); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card { padding: 18px; margin-bottom: 14px; min-width: 0; }
.section-head { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin-bottom: 14px; }
.badge { color: var(--muted); font-size: 11px; border: 1px solid var(--line); border-radius: 999px; padding: 3px 9px; white-space: nowrap; }
.activity-grid { display: grid; grid-template-columns: minmax(120px, .7fr) minmax(0, 1.3fr); gap: 18px; align-items: start; }
.big-number { font-size: 34px; line-height: 1; letter-spacing: -.04em; font-weight: 700; font-variant-numeric: tabular-nums; }
.big-number small { font-size: 12px; font-weight: 500; color: var(--muted); letter-spacing: 0; margin-left: 4px; }
.split { display: flex; justify-content: space-between; gap: 10px; margin-top: 7px; color: var(--muted); font-size: 11px; }
.track { height: 8px; overflow: hidden; border-radius: 999px; background: var(--panel-2); border: 1px solid var(--line); }
.fill { height: 100%; border-radius: inherit; background: linear-gradient(90deg, var(--accent), #06b6d4); }
.fill.teal { background: linear-gradient(90deg, var(--other), #06b6d4); }
.rows { display: flex; flex-direction: column; gap: 9px; }
.row { display: grid; grid-template-columns: minmax(70px, 1fr) 1.7fr auto; gap: 9px; align-items: center; min-width: 0; }
.row .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
.row .value { color: var(--muted); font-size: 11px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.legend { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; margin-top: 10px; color: var(--muted); font-size: 11px; }
.legend span { display: inline-flex; align-items: center; gap: 5px; }
.dot { width: 8px; height: 8px; border-radius: 50%; background: var(--accent); display: inline-block; }
.dot.other { background: var(--other); }
.legend .line { width: 16px; height: 2px; border-radius: 2px; background: var(--accent); }
.legend .line.other { background: var(--other); }
.chart-wrap { margin-top: 8px; overflow: hidden; }
svg { display: block; width: 100%; height: auto; overflow: visible; }
.chart-caption { display: flex; justify-content: space-between; gap: 8px; margin-top: 7px; color: var(--muted); font-size: 10px; }
.chart-note { margin-top: 8px; color: var(--muted); font-size: 11px; }
.grouped { display: grid; grid-template-columns: repeat(24, minmax(0, 1fr)); gap: 2px; align-items: end; height: 142px; padding-top: 8px; }
.grouped.week { grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 8px; }
.bar-group { display: flex; flex-direction: column; justify-content: flex-end; align-items: stretch; gap: 4px; height: 100%; min-width: 0; }
.bar-pair { display: flex; align-items: end; justify-content: center; gap: 2px; height: 100%; }
.bar-pair i { display: block; width: 45%; max-width: 7px; min-height: 2px; border-radius: 3px 3px 1px 1px; background: var(--me); opacity: .9; }
.bar-pair i.other { background: var(--other); opacity: .75; }
.bar-group small { color: var(--muted); font-size: 8px; line-height: 1; text-align: center; font-variant-numeric: tabular-nums; white-space: nowrap; }
.grouped.week .bar-group small { font-size: 10px; }
.chart-title { margin: 16px 0 0; }
.chart-title:first-of-type { margin-top: 0; }
.donut-layout { display: flex; align-items: center; gap: 22px; flex-wrap: wrap; }
.donut {
  width: 168px;
  height: 168px;
  flex: 0 0 auto;
  border-radius: 50%;
  position: relative;
  display: grid;
  place-items: center;
  background: conic-gradient(var(--c1) 0% 100%);
}
.donut::after {
  content: "";
  position: absolute;
  inset: 14px;
  border-radius: 50%;
  background: var(--panel);
  border: 1px solid var(--line);
}
.donut-center { position: relative; z-index: 1; text-align: center; }
.donut-center strong { display: block; font-size: 22px; line-height: 1.1; font-variant-numeric: tabular-nums; }
.donut-center span { display: block; color: var(--muted); font-size: 10px; }
.legend-list { flex: 1 1 180px; min-width: 0; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 7px 10px; }
.legend-item { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: 11px; }
.legend-item .swatch { width: 8px; height: 8px; border-radius: 2px; flex: 0 0 auto; }
.legend-item .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.legend-item .count { color: var(--muted); margin-left: auto; font-variant-numeric: tabular-nums; }
.top-days { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
.top-days li { display: grid; grid-template-columns: 22px 82px minmax(0, 1fr) auto; gap: 8px; align-items: center; }
.top-days .rank { color: var(--muted); font-size: 10px; font-variant-numeric: tabular-nums; }
.top-days .date { font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.top-days .count { color: var(--muted); font-size: 10px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.top-days .track { height: 6px; }
.words-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 20px; }
.cloud { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin-top: 10px; }
.word {
  display: inline-block;
  padding: 3px 8px;
  border-radius: 8px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--fg);
  line-height: 1.3;
  white-space: nowrap;
}
.word.hot { color: #fff; background: var(--accent); border-color: transparent; }
details { margin-top: 12px; }
summary { cursor: pointer; color: var(--muted); font-size: 11px; }
.table-wrap { overflow-x: auto; margin-top: 8px; }
table { width: 100%; border-collapse: collapse; font-size: 11px; }
th, td { text-align: left; padding: 5px 6px; border-bottom: 1px solid var(--line); }
th { color: var(--muted); font-weight: 600; }
td:last-child, th:last-child { text-align: right; font-variant-numeric: tabular-nums; }
footer { margin-top: 22px; color: var(--muted); font-size: 10px; text-align: center; line-height: 1.8; }
.sr-only {
  position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
}
@media (max-width: 640px) {
  main { padding: 12px 10px 40px; }
  .hero { padding: 20px; border-radius: 20px; }
  h1 { font-size: 23px; }
  .hero-total strong { font-size: 36px; }
  .kpi strong { font-size: 22px; }
  .activity-grid { grid-template-columns: 1fr; gap: 14px; }
  .card { padding: 15px; border-radius: 16px; }
  .words-grid { grid-template-columns: 1fr; gap: 16px; }
  .donut-layout { justify-content: center; }
  .donut { width: 150px; height: 150px; }
  .legend-list { width: 100%; flex-basis: 100%; }
  .top-days li { grid-template-columns: 18px 74px minmax(0, 1fr) auto; gap: 6px; }
  .grouped { gap: 1px; }
  .bar-pair i { width: 48%; }
}
"#;

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn format_count(value: usize) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        let remaining = text.len() - index;
        if index > 0 && remaining % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn percent(part: usize, total: usize) -> String {
    if total == 0 {
        "0%".to_string()
    } else {
        format!("{:.0}%", part as f64 * 100.0 / total as f64)
    }
}

fn svg_line(points: &[(f64, f64)]) -> String {
    points
        .iter()
        .enumerate()
        .map(|(index, (x, y))| {
            format!(
                "{}{:.1},{:.1}",
                if index == 0 { "M" } else { " L" },
                x,
                y
            )
        })
        .collect()
}

fn add_cjk_run(run: &str, words: &mut BTreeMap<String, usize>) {
    let chars: Vec<char> = run.chars().collect();
    if chars.len() == 1 {
        let value = chars[0].to_string();
        if !matches!(chars[0], '的' | '了' | '是' | '我' | '你' | '他' | '在' | '有' | '和' | '就' | '不' | '也' | '都' | '说' | '要' | '这' | '那' | '啊' | '吧' | '呢' | '吗' | '哦' | '嗯' | '个' | '上' | '下') {
            *words.entry(value).or_default() += 1;
        }
        return;
    }
    for pair in chars.windows(2) {
        let word: String = pair.iter().collect();
        if !matches!(word.as_str(), "我们" | "你们" | "他们" | "她们" | "它们" | "一个" | "就是" | "然后" | "但是" | "因为" | "所以" | "如果" | "还是" | "已经" | "自己") {
            *words.entry(word).or_default() += 1;
        }
    }
}

fn collect_words(text: &str, words: &mut BTreeMap<String, usize>) {
    let mut cjk = String::new();
    let mut ascii = String::new();
    let flush_cjk = |run: &mut String, words: &mut BTreeMap<String, usize>| {
        if !run.is_empty() {
            add_cjk_run(run, words);
            run.clear();
        }
    };
    let flush_ascii = |run: &mut String, words: &mut BTreeMap<String, usize>| {
        let value = run.to_ascii_lowercase();
        run.clear();
        if value.len() >= 2 && !value.contains("http") {
            *words.entry(value).or_default() += 1;
        }
    };
    for ch in text.chars().take(3000) {
        let is_cjk = matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff);
        if is_cjk {
            flush_ascii(&mut ascii, words);
            cjk.push(ch);
        } else if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, words);
            ascii.push(ch);
        } else {
            flush_cjk(&mut cjk, words);
            flush_ascii(&mut ascii, words);
        }
    }
    flush_cjk(&mut cjk, words);
    flush_ascii(&mut ascii, words);
}

fn word_cloud(words: &BTreeMap<String, usize>) -> String {
    let mut rows: Vec<(&String, &usize)> = words.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows.truncate(18);
    let max = rows.first().map(|(_, count)| **count).unwrap_or(1);
    let mut html = String::new();
    for (index, (word, count)) in rows.iter().enumerate() {
        let size = 12.0 + (**count as f64 / max as f64) * 11.0;
        let hot = if index < 4 { " hot" } else { "" };
        use std::fmt::Write as _;
        write!(
            &mut html,
            "<span class=\"word{}\" style=\"font-size:{:.0}px\" title=\"{} · {} 次\">{}</span>",
            hot,
            size,
            escape_html(word),
            count,
            escape_html(word)
        )
        .ok();
    }
    if html.is_empty() {
        html.push_str("<p class=\"muted\" style=\"font-size:12px\">暂无足够文本生成词云。</p>");
    }
    html
}

fn bar_group(label: &str, me: usize, other: usize, max: usize, title: &str) -> String {
    let me_height = if me == 0 { 2.0 } else { (me as f64 * 100.0 / max.max(1) as f64).max(3.0) };
    let other_height = if other == 0 { 2.0 } else { (other as f64 * 100.0 / max.max(1) as f64).max(3.0) };
    format!(
        "<div class=\"bar-group\" title=\"{}\"><div class=\"bar-pair\"><i style=\"height:{:.1}%\"></i><i class=\"other\" style=\"height:{:.1}%\"></i></div><small>{}</small></div>",
        escape_html(title),
        me_height,
        other_height,
        escape_html(label)
    )
}

fn write_analytics_report(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    use std::fmt::Write as _;

    let mut hours_me = [0usize; 24];
    let mut hours_other = [0usize; 24];
    let mut week_me = [0usize; 7];
    let mut week_other = [0usize; 7];
    let mut daily: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut senders: BTreeMap<String, usize> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut media: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut missing_media = 0usize;
    let mut chars = 0usize;
    let mut me_chars = 0usize;
    let mut other_chars = 0usize;
    let mut first = i64::MAX;
    let mut last = i64::MIN;
    let mut me_words: BTreeMap<String, usize> = BTreeMap::new();
    let mut other_words: BTreeMap<String, usize> = BTreeMap::new();
    let mut word_budget = 2_000_000usize;

    for message in context.messages {
        first = first.min(message.create_time);
        last = last.max(message.create_time);
        let message_chars = message.content.chars().count();
        chars += message_chars;
        let sender = if message.sender_name.trim().is_empty() {
            if message.is_send {
                if context.self_name.is_empty() { "我" } else { &context.self_name }
            } else {
                "未知"
            }
        } else {
            message.sender_name.as_str()
        };
        *senders.entry(sender.to_string()).or_default() += 1;
        let type_name = if message.type_name.trim().is_empty() {
            "其他消息".to_string()
        } else {
            message.type_name.clone()
        };
        *types.entry(type_name).or_default() += 1;
        if let Some(item) = &message.media {
            *media.entry(item.kind.label()).or_default() += 1;
            if message.media_path.is_none() {
                missing_media += 1;
            }
        }
        if message.media.is_none() && message.record.is_none() {
            if message.is_send {
                me_chars += message_chars;
            } else {
                other_chars += message_chars;
            }
            if word_budget > 0 {
                if message.is_send {
                    collect_words(&message.content, &mut me_words);
                } else {
                    collect_words(&message.content, &mut other_words);
                }
                word_budget = word_budget.saturating_sub(message_chars.min(3000));
            }
        }
        if let Some(dt) = Local.timestamp_opt(message.create_time, 0).single() {
            let day = dt.format("%Y-%m-%d").to_string();
            let bucket = daily.entry(day).or_insert((0, 0));
            let hour = dt.hour() as usize;
            let weekday = dt.weekday().num_days_from_monday() as usize;
            if message.is_send {
                bucket.0 += 1;
                if hour < 24 { hours_me[hour] += 1; }
                if weekday < 7 { week_me[weekday] += 1; }
            } else {
                bucket.1 += 1;
                if hour < 24 { hours_other[hour] += 1; }
                if weekday < 7 { week_other[weekday] += 1; }
            }
        }
    }

    let total = context.messages.len();
    let days_count = daily.len().max(1);
    let span_days = if total > 0 {
        if let (Some(start), Some(end)) = (
            Local.timestamp_opt(first, 0).single(),
            Local.timestamp_opt(last, 0).single(),
        ) {
            (end.date_naive() - start.date_naive()).num_days().max(0) as usize + 1
        } else {
            days_count
        }
    } else {
        0
    };
    let day_rows: Vec<(String, usize, usize)> = daily
        .iter()
        .map(|(day, (me, other))| (day.clone(), *me, *other))
        .collect();
    let max_daily = day_rows
        .iter()
        .map(|(_, me, other)| me + other)
        .max()
        .unwrap_or(1)
        .max(1);
    let peak = day_rows
        .iter()
        .max_by_key(|(_, me, other)| me + other)
        .map(|(day, me, other)| (day.clone(), me + other))
        .unwrap_or_else(|| ("-".to_string(), 0));
    let avg_day = total as f64 / days_count as f64;

    let mut sender_rows: Vec<(String, usize)> = senders.into_iter().collect();
    sender_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let max_sender = sender_rows.first().map(|(_, count)| *count).unwrap_or(1);

    let mut type_rows: Vec<(String, usize)> = types.into_iter().collect();
    type_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut type_slices = type_rows.clone();
    if type_slices.len() > 8 {
        let rest: usize = type_slices.iter().skip(7).map(|(_, count)| *count).sum();
        type_slices.truncate(7);
        type_slices.push(("其他".to_string(), rest));
    }
    let type_colors = ["var(--c1)", "var(--c2)", "var(--c3)", "var(--c4)", "var(--c5)", "var(--c6)", "var(--c7)", "var(--c8)"];
    let mut donut_stops = String::new();
    let mut cursor = 0.0f64;
    for (index, (_, count)) in type_slices.iter().enumerate() {
        let start = cursor;
        cursor += if total == 0 { 0.0 } else { *count as f64 * 100.0 / total as f64 };
        let end = if index + 1 == type_slices.len() { 100.0 } else { cursor };
        if index > 0 {
            donut_stops.push(',');
        }
        write!(
            &mut donut_stops,
            "{} {:.2}% {:.2}%",
            type_colors[index.min(type_colors.len() - 1)],
            start,
            end
        ).ok();
    }
    if donut_stops.is_empty() {
        donut_stops.push_str("var(--line) 0% 100%");
    }

    let max_hour = hours_me.iter().chain(hours_other.iter()).copied().max().unwrap_or(1).max(1);
    let max_week = week_me.iter().chain(week_other.iter()).copied().max().unwrap_or(1).max(1);
    let max_sender_pct = max_sender.max(1);
    let top_days: Vec<(String, usize)> = {
        let mut rows: Vec<(String, usize)> = day_rows
            .iter()
            .map(|(day, me, other)| (day.clone(), me + other))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows.truncate(10);
        rows
    };
    let top_day_max = top_days.first().map(|(_, count)| *count).unwrap_or(1).max(1);

    let chart_width = 820.0;
    let pad_left = 24.0;
    let pad_right = 24.0;
    let base_y = 158.0;
    let top_y = 26.0;
    let usable = chart_width - pad_left - pad_right;
    let series_points = |selector: fn(&(String, usize, usize)) -> usize| -> Vec<(f64, f64)> {
        day_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let x = if day_rows.len() <= 1 {
                    pad_left + usable / 2.0
                } else {
                    pad_left + usable * index as f64 / (day_rows.len() - 1) as f64
                };
                let value = selector(row);
                let y = base_y - (base_y - top_y) * (value as f64 / max_daily as f64);
                (x, y)
            })
            .collect()
    };
    let me_points = series_points(|row| row.1);
    let other_points = series_points(|row| row.2);
    let me_path = svg_line(&me_points);
    let other_path = svg_line(&other_points);
    let peak_index = day_rows
        .iter()
        .enumerate()
        .max_by_key(|(_, row)| row.1 + row.2)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let peak_point = me_points.get(peak_index).copied().unwrap_or((chart_width / 2.0, base_y));
    let mid_index = day_rows.len() / 2;
    let mid_label = day_rows.get(mid_index).map(|(day, _, _)| day.clone()).unwrap_or_else(|| "-".to_string());

    let first_stamp = if total > 0 { format_timestamp(first) } else { "-".to_string() };
    let last_stamp = if total > 0 { format_timestamp(last) } else { "-".to_string() };
    let first_day = day_rows.first().map(|(day, _, _)| day.as_str()).unwrap_or("-");
    let last_day = day_rows.last().map(|(day, _, _)| day.as_str()).unwrap_or("-");
    let media_summary = if media.is_empty() {
        "无媒体".to_string()
    } else {
        let mut summary = media
            .iter()
            .map(|(label, count)| format!("{label} {count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        if missing_media > 0 {
            summary.push_str(&format!(" · {missing_media} 个缺失"));
        }
        summary
    };

    let mut html = String::with_capacity(32_000);
    html.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>");
    html.push_str(&escape_html(&format!("聊天分析报告 - {}", context.display_name())));
    html.push_str("</title><style>");
    html.push_str(REPORT_CSS);
    html.push_str("</style></head><body><main>");

    html.push_str("<header class=\"hero\"><div class=\"eyebrow\">");
    html.push_str(if context.group() { "微信群聊天记录 · 可视化报告" } else { "微信聊天记录 · 可视化报告" });
    html.push_str("</div><h1>");
    html.push_str(&escape_html(context.display_name()));
    html.push_str("</h1><div class=\"hero-total\"><strong>");
    html.push_str(&format_count(total));
    html.push_str("</strong><span>条聊天记录</span></div><div class=\"hero-meta\"><span>");
    html.push_str(if context.group() { "群聊" } else { "私聊" });
    write!(
        &mut html,
        "</span><span>{} 至 {}</span><span>{} 天</span><span>导出于 {}</span></div></header>",
        escape_html(&first_stamp),
        escape_html(&last_stamp),
        span_days,
        escape_html(&format_timestamp(context.exported_at_sec()))
    ).ok();

    html.push_str("<section class=\"kpis\" aria-label=\"核心指标\">");
    let metrics = [
        ("互动人数", format_count(sender_rows.len()), "位发送者".to_string()),
        ("聊天总条数", format_count(total), format!("人均 {:.0} 条", total as f64 / sender_rows.len().max(1) as f64)),
        ("总字数", format_count(chars), format!("人均 {:.0} 字", chars as f64 / sender_rows.len().max(1) as f64)),
        ("单日发消息", format_count(peak.1), peak.0.clone()),
    ];
    for (label, value, note) in metrics {
        write!(
            &mut html,
            "<div class=\"kpi\"><span>{}</span><strong>{}</strong><small>{}</small></div>",
            escape_html(label),
            escape_html(&value),
            escape_html(&note)
        ).ok();
    }
    html.push_str("</section>");

    write!(
        &mut html,
        "<section class=\"card\"><div class=\"section-head\"><h2>活跃度概览</h2><span class=\"badge\">日均 {:.0} 条</span></div><div class=\"activity-grid\"><div><div class=\"big-number\">{:.0}<small>条/天</small></div><div class=\"muted\" style=\"font-size:11px;margin-top:5px\">共 {} 天 · 活跃 {} 天</div></div><div><div class=\"split\"><span>日均</span><span>峰值 {} 条 · {}</span></div><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><div class=\"split\"><span>活跃 {} 天</span><span>共 {} 天</span></div></div></div><div class=\"muted\" style=\"font-size:11px;margin-top:12px\">媒体：{}</div><h3 style=\"margin:16px 0 9px\">互动排行</h3>",
        avg_day,
        avg_day,
        span_days,
        days_count,
        peak.1,
        escape_html(&peak.0),
        (avg_day / max_daily as f64 * 100.0).min(100.0),
        days_count,
        span_days,
        escape_html(&media_summary)
    ).ok();

    html.push_str("<div class=\"rows\">");
    for (name, count) in sender_rows.iter().take(5) {
        write!(
            &mut html,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill teal\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · {}</span></div>",
            escape_html(name),
            *count as f64 * 100.0 / max_sender_pct as f64,
            format_count(*count),
            percent(*count, total)
        ).ok();
    }
    html.push_str("</div></section>");

    html.push_str("<section class=\"card\"><div class=\"section-head\"><h2>消息趋势</h2><span class=\"badge\">每日</span></div><div class=\"chart-wrap\"><svg viewBox=\"0 0 820 190\" role=\"img\" aria-label=\"每日消息趋势，我和其他人的消息数\">");
    write!(
        &mut html,
        "<line x1=\"{pad_left}\" y1=\"{base_y}\" x2=\"{}\" y2=\"{base_y}\" stroke=\"var(--line)\" stroke-width=\"1\" />",
        chart_width - pad_right
    ).ok();
    if !me_path.is_empty() {
        write!(&mut html, "<path d=\"{}\" fill=\"none\" stroke=\"var(--me)\" stroke-width=\"2.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\" />", me_path).ok();
        write!(&mut html, "<path d=\"{}\" fill=\"none\" stroke=\"var(--other)\" stroke-width=\"2\" stroke-dasharray=\"5 4\" stroke-linecap=\"round\" stroke-linejoin=\"round\" />", other_path).ok();
        write!(
            &mut html,
            "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\" fill=\"var(--accent)\" stroke=\"var(--panel)\" stroke-width=\"2\"><title>{} · {} 条</title></circle>",
            peak_point.0,
            peak_point.1,
            escape_html(&peak.0),
            peak.1
        ).ok();
    }
    write!(
        &mut html,
        "<text x=\"{pad_left}\" y=\"184\" fill=\"var(--muted)\" font-size=\"11\">{}</text><text x=\"{}\" y=\"184\" fill=\"var(--muted)\" font-size=\"11\" text-anchor=\"middle\">{}</text><text x=\"{}\" y=\"184\" fill=\"var(--muted)\" font-size=\"11\" text-anchor=\"end\">{}</text>",
        escape_html(first_day),
        chart_width / 2.0,
        escape_html(&mid_label),
        chart_width - pad_right,
        escape_html(last_day)
    ).ok();
    html.push_str("</svg></div><div class=\"legend\">");
    write!(
        &mut html,
        "<span><i class=\"line\"></i>我</span><span><i class=\"line other\"></i>其他人</span><span>峰值 {} 条 · {}</span></div><p class=\"chart-note\">纵轴最高为单日 {} 条；虚线为其他人。</p></section>",
        peak.1,
        escape_html(&peak.0),
        max_daily
    ).ok();

    html.push_str("<section class=\"card\"><div class=\"section-head\"><h2>我通常在几点发消息</h2><span class=\"badge\">按发送时间</span></div><div class=\"chart-title\"><h3>一天中的分布</h3></div><div class=\"grouped\" role=\"img\" aria-label=\"24 小时我与其他人的消息分布\">");
    for hour in 0..24 {
        let label = format!("{hour:02}");
        let title = format!("{hour:02}:00 · 我 {} 条 · 其他人 {} 条", hours_me[hour], hours_other[hour]);
        html.push_str(&bar_group(&label, hours_me[hour], hours_other[hour], max_hour, &title));
    }
    html.push_str("</div><div class=\"chart-caption\"><span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>23:00</span></div><div class=\"legend\">");
    write!(
        &mut html,
        "<span><i class=\"dot\"></i>我</span><span><i class=\"dot other\"></i>其他人</span><span>峰值 {} 条</span></div>",
        max_hour
    ).ok();
    html.push_str("<div class=\"chart-title\"><h3>一周中的分布</h3></div><div class=\"grouped week\" role=\"img\" aria-label=\"一周内我与其他人的消息分布\">");
    let weekdays = ["一", "二", "三", "四", "五", "六", "日"];
    for weekday in 0..7 {
        let title = format!("周{} · 我 {} 条 · 其他人 {} 条", weekdays[weekday], week_me[weekday], week_other[weekday]);
        html.push_str(&bar_group(weekdays[weekday], week_me[weekday], week_other[weekday], max_week, &title));
    }
    html.push_str("</div><div class=\"chart-caption\"><span>周一</span><span>周日</span></div></section>");

    write!(
        &mut html,
        "<div class=\"grid\" style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:14px;align-items:start\"><section class=\"card\" style=\"margin-bottom:0\"><div class=\"section-head\"><h2>消息类型占比</h2><span class=\"badge\">共 {} 类</span></div><div class=\"donut-layout\"><div class=\"donut\" style=\"background:conic-gradient({})\" role=\"img\" aria-label=\"消息类型占比\"><div class=\"donut-center\"><strong>{}</strong><span>条消息</span></div></div><div class=\"legend-list\">",
        type_rows.len(),
        donut_stops,
        format_count(total)
    ).ok();
    for (index, (name, count)) in type_slices.iter().enumerate() {
        let color = type_colors[index.min(type_colors.len() - 1)];
        write!(
            &mut html,
            "<div class=\"legend-item\" title=\"{} · {} 条\"><i class=\"swatch\" style=\"background:{}\"></i><span class=\"label\">{}</span><span class=\"count\">{}</span></div>",
            escape_html(name),
            count,
            color,
            escape_html(name),
            percent(*count, total)
        ).ok();
    }
    html.push_str("</div></div>");
    html.push_str("<details><summary>查看完整类型数据</summary><div class=\"table-wrap\"><table><thead><tr><th>类型</th><th>条数</th><th>占比</th></tr></thead><tbody>");
    for (name, count) in &type_rows {
        write!(
            &mut html,
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(name),
            format_count(*count),
            percent(*count, total)
        ).ok();
    }
    html.push_str("</tbody></table></div></details></section>");

    html.push_str("<section class=\"card\" style=\"margin-bottom:0\"><div class=\"section-head\"><h2>最活跃的日子</h2><span class=\"badge\">TOP 10</span></div><ol class=\"top-days\">");
    for (index, (day, count)) in top_days.iter().enumerate() {
        write!(
            &mut html,
            "<li><span class=\"rank\">{:02}</span><span class=\"date\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><span class=\"count\">{} 条</span></li>",
            index + 1,
            escape_html(day),
            *count as f64 * 100.0 / top_day_max as f64,
            format_count(*count)
        ).ok();
    }
    html.push_str("</ol><p class=\"chart-note\">按单日消息数排序；条数相同的日期按日期排序。</p></section></div>");

    html.push_str("<section class=\"card\"><div class=\"section-head\"><h2>我们的高频词</h2><span class=\"badge\">文本消息</span></div><div class=\"words-grid\"><div><h3>我最常说的</h3><div class=\"cloud\">");
    html.push_str(&word_cloud(&me_words));
    html.push_str("</div><p class=\"chart-note\">统计我发出的文本，共 ");
    html.push_str(&format_count(me_chars));
    html.push_str(" 字（词云基于文本消息样本）</p></div><div><h3>大家最常说的</h3><div class=\"cloud\">");
    html.push_str(&word_cloud(&other_words));
    html.push_str("</div><p class=\"chart-note\">统计其他人发出的文本，共 ");
    html.push_str(&format_count(other_chars));
    html.push_str(" 字</p></div></div></section>");

    write!(
        &mut html,
        "<footer>由 {} 生成 · 会话 ID: {} · {} 条消息 · {} 个发送者<br>报告为静态离线文件</footer></main></body></html>",
        escape_html(GENERATOR),
        escape_html(&context.conversation.id),
        format_count(total),
        sender_rows.len()
    ).ok();

    fs::write(path, html).with_context(|| format!("failed to write {}", path.display()))
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

#[cfg(test)]
mod analytics_tests {
    use super::{collect_words, format_count};
    use std::collections::BTreeMap;

    #[test]
    fn extracts_chinese_conversation_bigrams() {
        let mut words = BTreeMap::new();
        collect_words("好的，收到。什么？现在没有，不是可以哈哈", &mut words);
        for word in ["好的", "收到", "什么", "现在", "没有", "不是", "可以", "哈哈"] {
            assert!(words.contains_key(word), "missing {word}: {words:?}");
        }
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(1234), "1,234");
        assert_eq!(format_count(297425), "297,425");
    }
}
