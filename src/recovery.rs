use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::cli::{Format, ValidatedCli};
use crate::export::{self, ExportContext};
use crate::util;
use crate::wechat::model::{
    ChatRecord, ChatRecordItem, Conversation, ExportMessage, MediaKind, MediaRef, MessageSource,
    QuoteRef,
};

fn collect_backups(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
    {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            collect_backups(&child, out)?;
        } else if child.file_name().and_then(|name| name.to_str()) == Some("conversation.sqlite") {
            out.push(child);
        }
    }
    Ok(())
}

fn json_column(row: &rusqlite::Row<'_>, name: &str) -> Result<Option<Value>> {
    let raw: Option<String> = row.get(name)?;
    raw.map(|raw| serde_json::from_str(&raw).with_context(|| format!("invalid {name} JSON")))
        .transpose()
}

fn quote_from(value: &Value) -> QuoteRef {
    QuoteRef {
        server_id: value
            .get("server_id")
            .or_else(|| value.get("serverId"))
            .and_then(Value::as_i64),
        sender: value
            .get("sender")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        content: value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        create_time: value
            .get("create_time")
            .or_else(|| value.get("createTime"))
            .and_then(Value::as_i64),
        quoted_type: value
            .get("quoted_type")
            .or_else(|| value.get("quotedType"))
            .and_then(Value::as_i64),
        reply: value
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

fn record_from(value: &Value) -> ChatRecord {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| ChatRecordItem {
                    data_type: item
                        .get("data_type")
                        .or_else(|| item.get("dataType"))
                        .and_then(Value::as_i64),
                    text: item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    sender: item
                        .get("sender")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    time: item
                        .get("time")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    ChatRecord {
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        desc: value
            .get("desc")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        items,
    }
}

fn media_kind(value: &str) -> MediaKind {
    match value {
        "video" => MediaKind::Video,
        "voice" => MediaKind::Voice,
        "emoji" => MediaKind::Emoji,
        "file" => MediaKind::File,
        _ => MediaKind::Image,
    }
}

fn read_backup(path: &Path) -> Result<(Conversation, Vec<ExportMessage>, PathBuf)> {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open SQLite backup {}", path.display()))?;
    let mut session_statement = connection.prepare(
        "SELECT id,name,nickname,remark,alias,kind,last_time,message_count FROM session LIMIT 1",
    )?;
    let (id, name, nickname, remark, alias, kind, last_time, message_count): (
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
    ) = session_statement.query_row([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ))
    })?;
    let conversation = Conversation {
        id,
        name,
        nickname,
        remark,
        alias,
        is_group: kind == "group",
        unread_count: 0,
        sort_timestamp: last_time,
        last_timestamp: last_time,
        message_count,
    };
    let mut statement = connection.prepare(
        "SELECT id,platform_id,time,type,type_code,content,from_me,sender_id,sender_name,
                nickname,remark,alias,media_kind,media_path,media_md5,media_name,
                quote_json,record_json,raw_json
         FROM messages ORDER BY time, id",
    )?;
    let mut messages = Vec::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let source: Option<String> = row.get("raw_json")?;
        let source: Option<Value> = source
            .map(|raw| serde_json::from_str(&raw).context("invalid raw message JSON"))
            .transpose()?;
        let notes = source
            .as_ref()
            .and_then(|value| value.get("source"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let media_kind_value: String = row.get("media_kind")?;
        let media_md5: String = row.get("media_md5")?;
        let media_name: String = row.get("media_name")?;
        let media_path: String = row.get("media_path")?;
        let media =
            if !media_kind_value.is_empty() || !media_md5.is_empty() || !media_name.is_empty() {
                Some(MediaRef {
                    md5: media_md5,
                    file_name: media_name,
                    kind: media_kind(&media_kind_value),
                    ..Default::default()
                })
            } else {
                None
            };
        let media_path = if media.is_some() && !media_path.is_empty() {
            Some(media_path)
        } else {
            None
        };
        let quote = json_column(row, "quote_json")?.as_ref().map(quote_from);
        let record = json_column(row, "record_json")?.as_ref().map(record_from);
        let app_subtype = source
            .as_ref()
            .and_then(|value| value.get("app"))
            .and_then(|app| app.get("type"))
            .and_then(Value::as_i64);
        messages.push(ExportMessage {
            index: messages.len() + 1,
            local_id: row.get("id")?,
            server_id: row.get::<_, String>("platform_id")?.parse().unwrap_or(0),
            create_time: row.get("time")?,
            sender_id: row.get("sender_id")?,
            sender_name: row.get("sender_name")?,
            nickname: row.get("nickname")?,
            remark: row.get("remark")?,
            alias: row.get("alias")?,
            is_send: row.get::<_, i64>("from_me")? != 0,
            local_type: 0,
            base_type: row.get("type_code")?,
            type_name: row.get("type")?,
            content: row.get("content")?,
            source: MessageSource { notes },
            app_subtype,
            media,
            media_path,
            quote,
            record,
        });
    }
    Ok((
        conversation,
        messages,
        path.parent().unwrap_or(path).to_path_buf(),
    ))
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn run(options: &ValidatedCli) -> Result<()> {
    let input = options
        .recovery_input
        .as_deref()
        .context("recovery input path is missing")?;
    let output = options
        .output_dir
        .as_deref()
        .context("recovery output directory is missing")?;
    if output.exists() {
        bail!(
            "recovery output directory already exists: {}",
            output.display()
        );
    }
    std::fs::create_dir_all(output)?;
    let mut backups = Vec::new();
    collect_backups(input, &mut backups)?;
    backups.sort();
    if backups.is_empty() {
        bail!(
            "no conversation.sqlite backups found under {}",
            input.display()
        );
    }
    let formats = if options.formats.is_empty() {
        vec![Format::Json]
    } else {
        options.formats.clone()
    };
    let exported_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let mut recovered = Vec::new();
    let mut failed = Vec::new();
    for backup in backups {
        let result = (|| -> Result<()> {
            let (conversation, messages, root) = read_backup(&backup)?;
            let directory = output.join(conversation.folder_name());
            std::fs::create_dir_all(&directory)?;
            if options.include_media {
                copy_dir(&root.join("media"), &directory.join("media"))?;
            }
            let context = ExportContext {
                conversation: &conversation,
                messages: &messages,
                self_username: None,
                self_name: "我".to_string(),
                exported_at_ms,
                include_media: options.include_media,
                avatar_path: None,
            };
            let mut documents = serde_json::Map::new();
            for format in &formats {
                let path = directory.join(format.file_name());
                export::write_format(&context, *format, &path)?;
                documents.insert(format.name().to_string(), serde_json::json!(path));
            }
            recovered.push(serde_json::json!({
                "backup": backup,
                "output": directory,
                "id": conversation.id,
                "name": conversation.name,
                "messages": messages.len(),
                "documents": documents,
            }));
            Ok(())
        })();
        if let Err(error) = result {
            failed.push(format!("{}: {error:#}", backup.display()));
        }
    }
    let report = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "input": input,
        "output_directory": output,
        "recovered": recovered,
        "failed": failed,
    });
    let report_path = output.join("recovery-report.json");
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !failed.is_empty() {
        bail!("{} backup(s) could not be recovered", failed.len());
    }
    Ok(())
}
