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
  --bg: #f4f6fb;
  --panel: #ffffff;
  --panel-2: #f8fafc;
  --fg: #0f172a;
  --muted: #64748b;
  --line: #e2e8f0;
  --accent: #2563eb;
  --accent-2: #0d9488;
  --warn: #d97706;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0b1120;
    --panel: #111827;
    --panel-2: #172033;
    --fg: #e5e7eb;
    --muted: #94a3b8;
    --line: #263244;
    --accent: #60a5fa;
    --accent-2: #2dd4bf;
    --warn: #fbbf24;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
    "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
}
main { max-width: 980px; margin: 0 auto; padding: 40px 20px 64px; }
.hero { margin-bottom: 24px; }
.eyebrow {
  color: var(--accent);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: .14em;
  text-transform: uppercase;
}
h1 { margin: 6px 0 8px; font-size: 30px; line-height: 1.2; letter-spacing: -.02em; }
h2 { margin: 0 0 14px; font-size: 15px; }
p { margin: 0; }
.muted { color: var(--muted); }
.kpis {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
  gap: 12px;
  margin: 24px 0;
}
.kpi, .card {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 16px;
}
.kpi { padding: 16px 18px; }
.kpi span { display: block; color: var(--muted); font-size: 12px; }
.kpi strong { display: block; margin-top: 2px; font-size: 26px; line-height: 1.25; letter-spacing: -.02em; }
.kpi small { display: block; margin-top: 2px; color: var(--muted); font-size: 11px; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 16px; margin-bottom: 16px; }
.card { padding: 22px; margin-bottom: 16px; }
.rows { display: flex; flex-direction: column; gap: 10px; }
.row { display: grid; grid-template-columns: minmax(80px, 1fr) 2.2fr auto; gap: 10px; align-items: center; }
.row .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row .value { color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.track { height: 8px; overflow: hidden; border-radius: 999px; background: var(--panel-2); border: 1px solid var(--line); }
.fill { height: 100%; border-radius: inherit; background: var(--accent); }
.fill.teal { background: var(--accent-2); }
.hours { display: grid; grid-template-columns: repeat(24, 1fr); gap: 3px; align-items: end; height: 150px; padding-top: 8px; }
.hour { display: flex; flex-direction: column; justify-content: flex-end; align-items: stretch; gap: 4px; height: 100%; min-width: 0; }
.hbar { min-height: 2px; border-radius: 4px 4px 2px 2px; background: linear-gradient(180deg, var(--accent), var(--accent-2)); opacity: .9; }
.hour small { color: var(--muted); font-size: 9px; text-align: center; font-variant-numeric: tabular-nums; }
svg { display: block; width: 100%; height: auto; overflow: visible; }
.chart-caption { display: flex; justify-content: space-between; margin-top: 8px; color: var(--muted); font-size: 11px; }
footer { margin-top: 28px; color: var(--muted); font-size: 11px; text-align: center; }
@media (max-width: 560px) {
  main { padding: 24px 14px 48px; }
  h1 { font-size: 23px; }
  .card { padding: 18px; }
  .row { grid-template-columns: 70px 1fr auto; }
  .hour small { font-size: 7px; }
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

fn percent(part: usize, total: usize) -> String {
    if total == 0 {
        "0%".to_string()
    } else {
        format!("{:.0}%", part as f64 * 100.0 / total as f64)
    }
}

fn write_analytics_report(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    use std::fmt::Write as _;

    let mut hours = [0usize; 24];
    let mut days: BTreeMap<String, usize> = BTreeMap::new();
    let mut senders: BTreeMap<String, usize> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut media: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut missing_media = 0usize;
    let mut chars = 0usize;
    let mut first = i64::MAX;
    let mut last = i64::MIN;

    for message in context.messages {
        first = first.min(message.create_time);
        last = last.max(message.create_time);
        chars += message.content.chars().count();
        let stamp = format_timestamp(message.create_time);
        if let Some(day) = stamp.get(..10) {
            *days.entry(day.to_string()).or_default() += 1;
        }
        if let Some(hour) = stamp.get(11..13).and_then(|value| value.parse::<usize>().ok()) {
            if hour < 24 {
                hours[hour] += 1;
            }
        }
        let sender = if message.sender_name.trim().is_empty() {
            if message.is_send {
                if context.self_name.is_empty() {
                    "我"
                } else {
                    &context.self_name
                }
            } else {
                "未知"
            }
        } else {
            message.sender_name.as_str()
        };
        *senders.entry(sender.to_string()).or_default() += 1;
        let type_name = if message.type_name.trim().is_empty() {
            "其他".to_string()
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
    }

    let total = context.messages.len();
    let days_count = days.len().max(1);
    let span_days = if total > 0 {
        (((last - first).max(0) / 86_400) + 1) as usize
    } else {
        0
    };
    let mut sender_rows: Vec<(String, usize)> = senders.into_iter().collect();
    sender_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut type_rows: Vec<(String, usize)> = types.into_iter().collect();
    type_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let max_sender = sender_rows.first().map(|(_, count)| *count).unwrap_or(1);
    let max_type = type_rows.first().map(|(_, count)| *count).unwrap_or(1);
    let max_hour = hours.iter().copied().max().unwrap_or(1).max(1);
    let media_total: usize = media.values().sum();

    let day_rows: Vec<(String, usize)> = days.iter().map(|(day, count)| (day.clone(), *count)).collect();
    let max_day = day_rows.iter().map(|(_, count)| *count).max().unwrap_or(1).max(1);
    let chart_width = 820.0;
    let pad_left = 24.0;
    let pad_right = 24.0;
    let base_y = 160.0;
    let top_y = 24.0;
    let usable = chart_width - pad_left - pad_right;
    let line_points: Vec<(f64, f64)> = day_rows
        .iter()
        .enumerate()
        .map(|(index, (_, count))| {
            let x = if day_rows.len() <= 1 {
                pad_left + usable / 2.0
            } else {
                pad_left + usable * index as f64 / (day_rows.len() - 1) as f64
            };
            let y = base_y - (base_y - top_y) * (*count as f64 / max_day as f64);
            (x, y)
        })
        .collect();
    let line_path = line_points
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
        .collect::<String>();
    let area_path = if line_points.is_empty() {
        String::new()
    } else {
        format!(
            "{} L{:.1},{:.1} L{:.1},{:.1} Z",
            line_path,
            line_points.last().map(|point| point.0).unwrap_or(base_y),
            base_y,
            line_points.first().map(|point| point.0).unwrap_or(base_y),
            base_y
        )
    };
    let circles: String = line_points
        .iter()
        .map(|(x, y)| format!(r#"<circle cx="{x:.1}" cy="{y:.1}" r="2.4" fill="var(--accent)" />"#))
        .collect();

    let first_stamp = if total > 0 { format_timestamp(first) } else { "-".to_string() };
    let last_stamp = if total > 0 { format_timestamp(last) } else { "-".to_string() };
    let first_day = day_rows.first().map(|(day, _)| day.as_str()).unwrap_or("-");
    let last_day = day_rows.last().map(|(day, _)| day.as_str()).unwrap_or("-");

    let mut html = String::with_capacity(18_000);
    html.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>");
    html.push_str(&escape_html(&format!("聊天分析报告 - {}", context.display_name())));
    html.push_str("</title><style>");
    html.push_str(REPORT_CSS);
    html.push_str("</style></head><body><main>");
    html.push_str("<header class=\"hero\"><div class=\"eyebrow\">CHAT ANALYTICS · 聊天分析</div><h1>");
    html.push_str(&escape_html(context.display_name()));
    html.push_str("</h1><p class=\"muted\">");
    html.push_str(if context.group() { "群聊" } else { "私聊" });
    write!(
        &mut html,
        " · {} 至 {} · 导出于 {}</p></header>",
        escape_html(&first_stamp),
        escape_html(&last_stamp),
        escape_html(&format_timestamp(context.exported_at_sec()))
    ).ok();

    html.push_str("<section class=\"kpis\">");
    let kpis = [
        ("消息总数", total.to_string(), format!("{} 天", days_count)),
        ("参与人数", sender_rows.len().to_string(), "按发送者去重".to_string()),
        ("活跃天数", days_count.to_string(), format!("平均 {:.0} 条/天", total as f64 / days_count as f64)),
        ("总字数", chars.to_string(), format!("平均 {:.0} 字/条", chars as f64 / total.max(1) as f64)),
        (
            "媒体文件",
            media_total.to_string(),
            if missing_media > 0 {
                format!("{} 个已索引 · {} 个缺失", media_total - missing_media, missing_media)
            } else {
                "全部已索引".to_string()
            },
        ),
        ("时间跨度", format!("{} 天", span_days), format!("活跃 {} 天", days_count)),
    ];
    for (label, value, note) in kpis {
        write!(
            &mut html,
            "<div class=\"kpi\"><span>{}</span><strong>{}</strong><small>{}</small></div>",
            escape_html(label),
            escape_html(&value),
            escape_html(&note)
        ).ok();
    }
    html.push_str("</section>");

    html.push_str("<div class=\"grid\">");
    html.push_str("<section class=\"card\"><h2>发送者排行</h2><div class=\"rows\">");
    if sender_rows.is_empty() {
        html.push_str("<p class=\"muted\">没有发送者数据。</p>");
    }
    for (name, count) in sender_rows.iter().take(8) {
        write!(
            &mut html,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · {}</span></div>",
            escape_html(name),
            *count as f64 * 100.0 / max_sender as f64,
            count,
            percent(*count, total)
        ).ok();
    }
    html.push_str("</div></section>");

    html.push_str("<section class=\"card\"><h2>消息类型</h2><div class=\"rows\">");
    if type_rows.is_empty() {
        html.push_str("<p class=\"muted\">没有类型数据。</p>");
    }
    for (name, count) in type_rows.iter().take(9) {
        write!(
            &mut html,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill teal\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · {}</span></div>",
            escape_html(name),
            *count as f64 * 100.0 / max_type as f64,
            count,
            percent(*count, total)
        ).ok();
    }
    if !media.is_empty() {
        html.push_str("</div><p class=\"muted\" style=\"margin-top:12px;font-size:12px\">媒体构成：");
        let mut first_item = true;
        for (label, count) in &media {
            if !first_item {
                html.push_str(" · ");
            }
            first_item = false;
            write!(&mut html, "{} {}", escape_html(label), count).ok();
        }
        html.push_str("</p>");
    }
    html.push_str("</section></div>");

    html.push_str("<section class=\"card\"><h2>24 小时活跃度</h2><div class=\"hours\">");
    for (hour, count) in hours.iter().enumerate() {
        let height = if *count == 0 {
            2.0
        } else {
            (*count as f64 * 100.0 / max_hour as f64).max(4.0)
        };
        write!(
            &mut html,
            "<div class=\"hour\" title=\"{:02}:00 · {} 条\"><div class=\"hbar\" style=\"height:{:.1}%\"></div><small>{:02}</small></div>",
            hour,
            count,
            height,
            hour
        ).ok();
    }
    html.push_str("</div><div class=\"chart-caption\"><span>00:00</span><span>12:00</span><span>23:00</span></div></section>");

    html.push_str("<section class=\"card\"><h2>每日消息趋势</h2><svg viewBox=\"0 0 820 190\" role=\"img\" aria-label=\"每日消息趋势\">");
    write!(
        &mut html,
        "<line x1=\"{pad_left}\" y1=\"{base_y}\" x2=\"{}\" y2=\"{base_y}\" stroke=\"var(--line)\" stroke-width=\"1\" />",
        chart_width - pad_right
    ).ok();
    if !area_path.is_empty() {
        write!(
            &mut html,
            "<path d=\"{}\" fill=\"var(--accent)\" opacity=\"0.08\" />",
            area_path
        ).ok();
        write!(
            &mut html,
            "<path d=\"{}\" fill=\"none\" stroke=\"var(--accent)\" stroke-width=\"2.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" />",
            line_path
        ).ok();
    }
    html.push_str(&circles);
    write!(
        &mut html,
        "<text x=\"{pad_left}\" y=\"184\" fill=\"var(--muted)\" font-size=\"11\">{}</text>",
        escape_html(first_day)
    ).ok();
    write!(
        &mut html,
        "<text x=\"{}\" y=\"184\" fill=\"var(--muted)\" font-size=\"11\" text-anchor=\"end\">{}</text>",
        chart_width - pad_right,
        escape_html(last_day)
    ).ok();
    write!(
        &mut html,
        "<text x=\"{}\" y=\"16\" fill=\"var(--muted)\" font-size=\"11\" text-anchor=\"middle\">峰值 {} 条/天</text>",
        chart_width / 2.0,
        max_day
    ).ok();
    html.push_str("</svg></section>");

    write!(
        &mut html,
        "<footer>由 {} 生成 · 会话 ID: {} · 共 {} 条消息</footer>",
        escape_html(GENERATOR),
        escape_html(&context.conversation.id),
        total
    ).ok();
    html.push_str("</main></body></html>");

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
