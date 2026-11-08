use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::Format;
use crate::util::{format_iso_timestamp, format_timestamp, GENERATOR, SCHEMA};
use crate::wechat::model::{Conversation, ExportMessage, MediaKind};

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

fn write_html(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    let mut out = String::new();
    let display_name = crate::util::escape_html(context.display_name());
    let initial: String = context
        .display_name()
        .chars()
        .next()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "微".to_string());
    let kind = if context.group() { "群聊" } else { "私聊" };
    out.push_str("<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"UTF-8\" />\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n");
    out.push_str(&format!("<title>{display_name} - 聊天记录</title>\n"));
    out.push_str(
        r#"<script>try{const t=localStorage.getItem('wechat-theme');document.documentElement.dataset.theme=t||(window.matchMedia&&matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light')}catch(e){}</script>
<style>
:root{color-scheme:light;--bg:#f5f6f8;--fg:#111827;--card:#ffffff;--muted:#6b7280;--muted-bg:#f3f4f6;--border:#e5e7eb;--primary:#2563eb;--primary-fg:#ffffff;--me-bg:#eff6ff;--me-border:#dbeafe;--danger:#b91c1c;--danger-bg:#fef2f2;--danger-border:#fecaca;--radius:16px;--shadow:0 1px 2px rgba(16,24,40,.06),0 1px 3px rgba(16,24,40,.08)}
[data-theme="dark"]{color-scheme:dark;--bg:#0b0f17;--fg:#e5e7eb;--card:#121826;--muted:#9aa4b2;--muted-bg:#1a2233;--border:#273244;--primary:#60a5fa;--primary-fg:#0b1220;--me-bg:#16233a;--me-border:#243b5f;--danger:#fca5a5;--danger-bg:#2a1518;--danger-border:#5b232a;--shadow:0 1px 2px rgba(0,0,0,.4)}
*{box-sizing:border-box}
html{scroll-behavior:smooth}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;-webkit-font-smoothing:antialiased}
button,input{font:inherit}
.topbar{position:sticky;top:0;z-index:20;background:var(--bg);border-bottom:1px solid var(--border)}
.topbar-inner{max-width:960px;margin:0 auto;padding:14px 20px;display:flex;align-items:center;gap:16px}
.identity{display:flex;align-items:center;gap:12px;min-width:0;flex:1}
.logo{width:42px;height:42px;border-radius:14px;background:var(--primary);color:var(--primary-fg);display:grid;place-items:center;font-weight:700;font-size:17px;flex:none}
.identity-text{min-width:0}
.title{margin:0;font-size:17px;line-height:1.3;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.meta{display:flex;gap:10px;flex-wrap:wrap;color:var(--muted);font-size:12px;margin-top:2px}
.meta .mono{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.actions{display:flex;align-items:center;gap:10px;flex:none}
.search{position:relative;display:flex;align-items:center}
.search svg{position:absolute;left:11px;width:16px;height:16px;color:var(--muted);pointer-events:none}
.search input{width:250px;padding:9px 12px 9px 34px;border:1px solid var(--border);border-radius:10px;background:var(--card);color:var(--fg);outline:none;transition:border-color .15s,box-shadow .15s}
.search input:focus{border-color:var(--primary);box-shadow:0 0 0 3px color-mix(in srgb,var(--primary) 18%,transparent)}
.icon-btn{width:38px;height:38px;border-radius:10px;border:1px solid var(--border);background:var(--card);color:var(--muted);display:grid;place-items:center;cursor:pointer;transition:border-color .15s,color .15s}
.icon-btn:hover{border-color:var(--primary);color:var(--primary)}
.icon-btn svg{width:18px;height:18px}
.icon-sun{display:none}
[data-theme="dark"] .icon-sun{display:block}
[data-theme="dark"] .icon-moon{display:none}
.page{max-width:960px;margin:0 auto;padding:20px 20px 96px}
.toolbar{display:flex;justify-content:space-between;gap:12px;margin:0 4px 6px;color:var(--muted);font-size:12px}
.messages{display:flex;flex-direction:column;gap:12px}
.day{display:flex;justify-content:center;margin:18px 0 4px}
.day span{background:var(--muted-bg);color:var(--muted);border:1px solid var(--border);border-radius:999px;padding:3px 12px;font-size:12px}
.message{display:flex;scroll-margin-top:84px}
.message.me{justify-content:flex-end}
.message[hidden]{display:none}
.bubble{max-width:min(78%,700px);background:var(--card);border:1px solid var(--border);border-radius:var(--radius);padding:11px 14px;box-shadow:var(--shadow);overflow-wrap:anywhere}
.message.me .bubble{background:var(--me-bg);border-color:var(--me-border)}
.head{display:flex;align-items:baseline;gap:8px;margin-bottom:6px;font-size:12px;color:var(--muted)}
.sender{font-weight:600;color:var(--fg)}
.message.me .head{justify-content:flex-end}
.content{white-space:pre-wrap;word-break:break-word}
.content:empty{display:none}
.quote{margin:0 0 8px;padding:8px 10px;border:1px solid var(--border);border-left:3px solid var(--primary);border-radius:10px;background:var(--muted-bg);font-size:13px;color:var(--muted)}
.quote.quote-missing{border-left-color:var(--danger);background:var(--danger-bg)}
.quote-head{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:12px;margin-bottom:3px}
.quote-sender{font-weight:600;color:var(--fg)}
.quote a{color:var(--primary);text-decoration:none}
.quote a:hover{text-decoration:underline}
.quote-text{white-space:pre-wrap;word-break:break-word}
.quote-note{color:var(--danger);font-size:12px}
.media{margin-top:10px}
.media img{display:block;max-width:min(100%,420px);height:auto;border-radius:12px;border:1px solid var(--border)}
.media video{display:block;max-width:100%;border-radius:12px}
.media audio{display:block;width:min(100%,360px)}
.file{display:inline-flex;align-items:center;gap:8px;padding:8px 11px;border:1px solid var(--border);border-radius:10px;background:var(--muted-bg);color:var(--fg);text-decoration:none;font-size:13px}
.file:hover{border-color:var(--primary)}
.file svg{width:16px;height:16px;color:var(--muted);flex:none}
.media-missing{display:flex;gap:8px;align-items:flex-start;margin-top:10px;padding:9px 11px;border:1px solid var(--danger-border);border-radius:10px;background:var(--danger-bg);color:var(--danger);font-size:13px}
.media-missing svg{width:16px;height:16px;flex:none;margin-top:1px}
.message.flash .bubble,.message:target .bubble{box-shadow:0 0 0 2px var(--primary),var(--shadow)}
.empty{display:flex;justify-content:center;padding:56px 0;color:var(--muted);font-size:14px}
.empty[hidden]{display:none}
.to-bottom{position:fixed;right:22px;bottom:22px;width:42px;height:42px;border-radius:999px;border:1px solid var(--border);background:var(--card);color:var(--muted);display:grid;place-items:center;cursor:pointer;box-shadow:var(--shadow);opacity:0;pointer-events:none;transform:translateY(8px);transition:opacity .18s,transform .18s,border-color .15s,color .15s;z-index:20}
.to-bottom.show{opacity:1;pointer-events:auto;transform:none}
.to-bottom:hover{border-color:var(--primary);color:var(--primary)}
.to-bottom svg{width:18px;height:18px}
@media(max-width:680px){.topbar-inner{flex-wrap:wrap;gap:12px}.identity{flex-basis:100%}.actions{width:100%}.search{flex:1}.search input{width:100%}.bubble{max-width:92%}.page{padding:16px 12px 88px}}
@media print{.topbar,.to-bottom,.toolbar{display:none}.page{padding:0}.bubble{box-shadow:none;break-inside:avoid}}
</style>
"#,
    );
    out.push_str("</head>\n<body>\n");
    out.push_str(&format!(
        "<header class=\"topbar\"><div class=\"topbar-inner\"><div class=\"identity\"><div class=\"logo\" aria-hidden=\"true\">{}</div><div class=\"identity-text\"><h1 class=\"title\">{}</h1><div class=\"meta\"><span>{} 条消息</span><span>{}</span><span class=\"mono\">{}</span><span>导出于 {}</span></div></div></div><div class=\"actions\"><label class=\"search\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><circle cx=\"11\" cy=\"11\" r=\"7\"></circle><path d=\"m20 20-3.5-3.5\"></path></svg><input id=\"searchInput\" type=\"search\" placeholder=\"搜索消息…\" autocomplete=\"off\" /></label><button id=\"themeToggle\" class=\"icon-btn\" type=\"button\" title=\"切换主题\" aria-label=\"切换主题\"><svg class=\"icon-moon\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><path d=\"M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8Z\"></path></svg><svg class=\"icon-sun\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" r=\"4\"></circle><path d=\"M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4\"></path></svg></button></div></div></header>\n",
        crate::util::escape_html(&initial),
        display_name,
        context.messages.len(),
        kind,
        crate::util::escape_html(&context.conversation.id),
        crate::util::escape_html(&format_timestamp(context.exported_at_sec())),
    ));
    out.push_str("<main class=\"page\"><div class=\"toolbar\"><span id=\"resultCount\">");
    out.push_str(&format!("共 {} 条消息", context.messages.len()));
    out.push_str("</span><span>点击引用可跳转原消息</span></div>\n<div id=\"messages\" class=\"messages\">\n");

    let mut by_server: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut by_local: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for previous in context.messages {
        by_local.entry(previous.local_id.to_string()).or_insert(previous.index);
        if previous.server_id > 0 {
            by_server.entry(previous.server_id.to_string()).or_insert(previous.index);
        }
    }

    let mut last_day = String::new();
    for message in context.messages {
        let timestamp = format_timestamp(message.create_time);
        let day = timestamp.get(..10).unwrap_or(&timestamp).to_string();
        if day != last_day {
            out.push_str(&format!(
                "<div class=\"day\"><span>{}</span></div>\n",
                crate::util::escape_html(&day)
            ));
            last_day = day;
        }
        out.push_str(&format!(
            "<div class=\"message{}\" id=\"msg-{}\">\n<div class=\"bubble\">\n<div class=\"head\"><span class=\"sender\">{}</span><span class=\"time\">{}</span></div>\n",
            if message.is_send { " me" } else { "" },
            message.index,
            crate::util::escape_html(&message.sender_name),
            crate::util::escape_html(&timestamp),
        ));
        let mut body = message.content.as_str();
        if let Some(quote) = &message.quote {
            let quoted_label = match quote.quoted_type {
                Some(3) => "[图片]",
                Some(34) => "[语音]",
                Some(43) => "[视频]",
                Some(47) => "[表情]",
                Some(49) => "[文件]",
                _ => "[消息]",
            };
            let quoted_text = if quote.content.is_empty() {
                quoted_label.to_string()
            } else {
                quote.content.clone()
            };
            let sender = if quote.sender.is_empty() {
                "未知".to_string()
            } else {
                quote.sender.clone()
            };
            let target = quote.server_id.and_then(|id| {
                let key = id.to_string();
                by_server
                    .get(&key)
                    .or_else(|| by_local.get(&key))
                    .copied()
            });
            match target {
                Some(index) => {
                    out.push_str(&format!(
                        "<blockquote class=\"quote\"><div class=\"quote-head\"><span class=\"quote-sender\">{}</span><a href=\"#msg-{}\">查看原消息</a></div><div class=\"quote-text\"><a href=\"#msg-{}\">{}</a></div></blockquote>\n",
                        crate::util::escape_html(&sender),
                        index,
                        index,
                        crate::util::escape_html(&quoted_text),
                    ));
                }
                None => {
                    out.push_str(&format!(
                        "<blockquote class=\"quote quote-missing\"><div class=\"quote-head\"><span class=\"quote-sender\">{}</span><span class=\"quote-note\">原消息未找到（可能已删除或不在导出范围内）</span></div><div class=\"quote-text\">{}</div></blockquote>\n",
                        crate::util::escape_html(&sender),
                        crate::util::escape_html(&quoted_text),
                    ));
                }
            }
            body = quote.reply.as_str();
        }
        if !body.is_empty() {
            out.push_str(&format!(
                "<div class=\"content\">{}</div>\n",
                crate::util::escape_html(body)
            ));
        }
        if let Some(media_path) = &message.media_path {
            let escaped = crate::util::escape_html(media_path);
            let kind = message.media.as_ref().map(|media| media.kind);
            match kind {
                Some(MediaKind::Image) | Some(MediaKind::Emoji) => {
                    out.push_str(&format!(
                        "<div class=\"media\"><a href=\"{escaped}\" target=\"_blank\" rel=\"noopener\"><img src=\"{escaped}\" alt=\"媒体\" loading=\"lazy\" /></a></div>\n"
                    ));
                }
                Some(MediaKind::Video) => {
                    out.push_str(&format!(
                        "<div class=\"media\"><video src=\"{escaped}\" controls preload=\"metadata\"></video></div>\n"
                    ));
                }
                Some(MediaKind::Voice) => {
                    out.push_str(&format!(
                        "<div class=\"media\"><audio src=\"{escaped}\" controls preload=\"metadata\"></audio></div>\n"
                    ));
                }
                _ => {
                    let label = message
                        .media
                        .as_ref()
                        .map(|media| media.file_name.as_str())
                        .filter(|name| !name.is_empty())
                        .unwrap_or("附件");
                    out.push_str(&format!(
                        "<div class=\"media\"><a class=\"file\" href=\"{escaped}\" target=\"_blank\" rel=\"noopener\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><path d=\"M21.4 11.6 12 21l-9-9a5 5 0 0 1 7-7l9 9a3 3 0 0 1-4 4l-9-9\"></path></svg>{}</a></div>\n",
                        crate::util::escape_html(label)
                    ));
                }
            }
        } else if let Some(media) = &message.media {
            let label = if !media.file_name.is_empty() {
                format!("{}: {} (未索引，空缺)", media.kind.label(), media.file_name)
            } else {
                format!("{}: 未索引，空缺", media.kind.label())
            };
            out.push_str(&format!(
                "<div class=\"media-missing\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><path d=\"M12 9v4\"></path><path d=\"M12 17h.01\"></path><path d=\"M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z\"></path></svg><span>{}</span></div>\n",
                crate::util::escape_html(&label)
            ));
        }
        out.push_str("</div>\n</div>\n");
    }
    out.push_str("</div>\n<div id=\"emptyState\" class=\"empty\" hidden>没有匹配的消息</div>\n</main>\n");
    out.push_str("<button id=\"toBottom\" class=\"to-bottom\" type=\"button\" title=\"回到最新\" aria-label=\"回到最新\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><path d=\"M12 5v14\"></path><path d=\"m19 12-7 7-7-7\"></path></svg></button>\n");
    out.push_str(
        r#"<script>
const input=document.getElementById('searchInput');
const nodes=[...document.querySelectorAll('.message')];
const count=document.getElementById('resultCount');
const empty=document.getElementById('emptyState');
const total=nodes.length;
function applyFilter(){
  const q=input.value.trim().toLowerCase();
  let visible=0;
  for(const node of nodes){
    const match=!q||node.textContent.toLowerCase().includes(q);
    node.hidden=!match;
    if(match)visible++;
  }
  count.textContent=q?visible+' / '+total+' 条消息':'共 '+total+' 条消息';
  empty.hidden=visible!==0;
}
input.addEventListener('input',applyFilter);
document.addEventListener('keydown',e=>{
  if(e.key==='/'&&document.activeElement!==input){e.preventDefault();input.focus();}
  if(e.key==='Escape'&&document.activeElement===input){input.value='';input.dispatchEvent(new Event('input'));input.blur();}
});
document.querySelectorAll('.quote a[href]').forEach(a=>{
  const href=a.getAttribute('href')||'';
  if(!href.startsWith('#msg-'))return;
  a.addEventListener('click',e=>{
    e.preventDefault();
    const id=href.slice(1);
    if(input.value){input.value='';input.dispatchEvent(new Event('input'));}
    const el=document.getElementById(id);
    if(el){
      el.scrollIntoView({behavior:'smooth',block:'start'});
      el.classList.add('flash');
      setTimeout(()=>el.classList.remove('flash'),1200);
    }
  });
});
const root=document.documentElement;
const toggle=document.getElementById('themeToggle');
toggle.addEventListener('click',()=>{
  root.dataset.theme=root.dataset.theme==='dark'?'light':'dark';
  try{localStorage.setItem('wechat-theme',root.dataset.theme);}catch(e){}
});
const toBottom=document.getElementById('toBottom');
const onScroll=()=>toBottom.classList.toggle('show',window.scrollY>600);
window.addEventListener('scroll',onScroll,{passive:true});
onScroll();
toBottom.addEventListener('click',()=>window.scrollTo({top:document.body.scrollHeight,behavior:'smooth'}));
</script>
</body>
</html>
"#,
    );
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))
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
