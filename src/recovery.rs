use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::cli::{Format, ValidatedCli};
use crate::db::Db;
use crate::export::{self, ExportContext};
use crate::util;
use crate::wechat::model::{
    ChatRecord, ChatRecordItem, Conversation, ExportMessage, MediaKind, MediaRef, MessageSource,
    QuoteRef, RawSqlValue,
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
    if options.target_database.is_some() {
        return run_restore(options);
    }
    run_offline(options)
}

fn run_offline(options: &ValidatedCli) -> Result<()> {
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


fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn schema_tables(connection: &rusqlite::Connection, schema: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!(
        "SELECT name FROM {schema}.sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ))?;
    let mut rows = statement.query([])?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next()? {
        tables.push(row.get(0)?);
    }
    Ok(tables)
}

fn schema_columns(
    connection: &rusqlite::Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!(
        "PRAGMA {schema}.table_info({})",
        quote_string(table)
    ))?;
    let mut rows = statement.query([])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        columns.push(row.get::<_, String>(1)?);
    }
    Ok(columns)
}

fn table_is_virtual(connection: &rusqlite::Connection, table: &str) -> Result<bool> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM backup.sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(sql
        .as_deref()
        .is_some_and(|sql| sql.to_ascii_uppercase().contains("VIRTUAL TABLE")))
}

fn collect_database_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if root.is_file() {
        out.push(root.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_database_files(&path, out)?;
        } else if path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("db")) {
            out.push(path);
        }
    }
    Ok(())
}

fn restore_one(
    source: &Path,
    target: &Path,
    key: &[u8; 32],
) -> Result<(usize, i64)> {
    let db = Db::open_for_decrypt(target, Some(key), false)
        .with_context(|| format!("failed to open target database {}", target.display()))?;
    let connection = db.conn();
    connection.execute_batch("PRAGMA foreign_keys=OFF;")?;
    connection.execute_batch(&format!(
        "ATTACH DATABASE {} AS backup KEY '';",
        quote_string(&source.to_string_lossy())
    ))?;
    let result = (|| -> Result<(usize, i64)> {
        let backup_tables = schema_tables(connection, "backup")?;
        if backup_tables.is_empty() {
            bail!("backup database has no tables: {}", source.display());
        }
        let target_tables = schema_tables(connection, "main")?;
        let mut plans = Vec::new();
        for table in &backup_tables {
            if !target_tables.iter().any(|name| name == table) {
                bail!("target database is missing table {table}");
            }
            if table_is_virtual(connection, table)? {
                bail!("restoring virtual table {table} is not supported");
            }
            let backup_columns = schema_columns(connection, "backup", table)?;
            let target_columns = schema_columns(connection, "main", table)?;
            for column in &backup_columns {
                if !target_columns.iter().any(|name| name == column) {
                    bail!("target table {table} is missing column {column}");
                }
            }
            plans.push((table.clone(), backup_columns));
        }
        connection.execute_batch("BEGIN IMMEDIATE;")?;
        let mut rows_restored = 0i64;
        for (table, columns) in &plans {
            let quoted_columns = columns
                .iter()
                .map(|column| quote_ident(column))
                .collect::<Vec<_>>()
                .join(", ");
            let table_name = quote_ident(table);
            connection.execute(&format!("DELETE FROM main.{table_name};"), [])?;
            connection.execute(
                &format!(
                    "INSERT INTO main.{table_name} ({quoted_columns}) SELECT {quoted_columns} FROM backup.{table_name};"
                ),
                [],
            )?;
            let target_count: i64 = connection.query_row(
                &format!("SELECT count(*) FROM main.{table_name}"),
                [],
                |row| row.get(0),
            )?;
            let backup_count: i64 = connection.query_row(
                &format!("SELECT count(*) FROM backup.{table_name}"),
                [],
                |row| row.get(0),
            )?;
            if target_count != backup_count {
                bail!("row count mismatch for {table}: target={target_count} backup={backup_count}");
            }
            rows_restored += target_count;
        }
        connection.execute_batch("COMMIT;")?;
        Ok((plans.len(), rows_restored))
    })();
    if result.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    let _ = connection.execute_batch("DETACH DATABASE backup;");
    drop(db);
    result
}

fn make_restore_backup(
    target_root: &Path,
    files: &[PathBuf],
    requested: Option<&Path>,
) -> Result<PathBuf> {
    let backup_root = match requested {
        Some(path) => path.to_path_buf(),
        None => {
            let parent = target_root.parent().unwrap_or(target_root);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            let base = format!(".wechat-exporter-restore-{stamp}");
            let mut candidate = parent.join(&base);
            let mut index = 1u32;
            while candidate.exists() {
                candidate = parent.join(format!("{base}-{index}"));
                index += 1;
            }
            candidate
        }
    };
    std::fs::create_dir_all(&backup_root)?;
    for target in files {
        let relative = target.strip_prefix(target_root).unwrap_or(target);
        let destination = backup_root.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(target, &destination)
            .with_context(|| format!("failed to back up {}", target.display()))?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", target.display()));
            if sidecar.is_file() {
                let sidecar_destination = PathBuf::from(format!("{}{suffix}", destination.display()));
                std::fs::copy(&sidecar, sidecar_destination)?;
            }
        }
    }
    Ok(backup_root)
}

fn run_restore(options: &ValidatedCli) -> Result<()> {
    let input = options
        .recovery_input
        .as_deref()
        .context("recovery input path is missing")?;
    if input.is_file()
        && input
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("zip"))
    {
        return run_restore_archive(options);
    }
    let mut databases = Vec::new();
    if input.is_file() {
        let extension = input.extension().and_then(|value| value.to_str()).unwrap_or("");
        if extension.eq_ignore_ascii_case("db") {
            databases.push(input.to_path_buf());
        }
    } else {
        collect_database_files(input, &mut databases)?;
    }
    if databases.is_empty() {
        return run_restore_partial(options);
    }
    run_restore_full(options)
}

fn run_restore_full(options: &ValidatedCli) -> Result<()> {
    let input = options
        .recovery_input
        .as_deref()
        .context("recovery input path is missing")?;
    let target = options
        .target_database
        .as_deref()
        .context("target database is missing")?;
    let key = options.key.as_ref().context("database key is missing")?;
    let layout = crate::wechat::discover::discover(target)?;
    let target_root = layout
        .db_storage
        .context("target db_storage path is missing")?;
    if !target_root.is_dir() {
        bail!("target database is not a directory: {}", target_root.display());
    }
    let input_root = if input.is_dir() {
        input.to_path_buf()
    } else {
        input
            .parent()
            .context("recovery input has no parent directory")?
            .to_path_buf()
    };
    let input_canonical = std::fs::canonicalize(&input_root).unwrap_or_else(|_| input_root.clone());
    let target_canonical = std::fs::canonicalize(&target_root).unwrap_or_else(|_| target_root.clone());
    if target_canonical.starts_with(&input_canonical) || input_canonical.starts_with(&target_canonical) {
        bail!("recovery input and target db_storage must be different directories");
    }
    let mut sources = Vec::new();
    collect_database_files(input, &mut sources)?;
    sources.sort();
    if sources.is_empty() {
        bail!(
            "recovery input contains no .db files; a decrypt backup is required (conversation.sqlite is an offline export, not a full database backup)"
        );
    }
    let mut plans = Vec::new();
    for source in sources {
        let relative = source
            .strip_prefix(&input_root)
            .unwrap_or(&source)
            .to_path_buf();
        let target_path = target_root.join(&relative);
        if !target_path.is_file() {
            bail!(
                "target database is missing for {} (expected {})",
                relative.display(),
                target_path.display()
            );
        }
        plans.push((source, target_path));
    }
    let backup_root = make_restore_backup(
        &target_root,
        &plans.iter().map(|(_, target)| target.clone()).collect::<Vec<_>>(),
        options.backup_dir.as_deref(),
    )?;
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for (source, target_path) in &plans {
        match restore_one(source, target_path, key) {
            Ok((tables, rows)) => restored.push(serde_json::json!({
                "database": target_path.strip_prefix(&target_root).unwrap_or(target_path),
                "target": target_path,
                "tables": tables,
                "rows": rows,
            })),
            Err(error) => failed.push(format!("{}: {error:#}", target_path.display())),
        }
    }
    let report = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "input": input,
        "target_database": target_root,
        "target_backup": backup_root,
        "restored": restored,
        "failed": failed,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !failed.is_empty() {
        bail!("{} database(s) could not be restored; target backup: {}", failed.len(), backup_root.display());
    }
    Ok(())
}


fn sql_value(value: rusqlite::types::ValueRef<'_>) -> RawSqlValue {
    match value {
        rusqlite::types::ValueRef::Null => RawSqlValue::Null,
        rusqlite::types::ValueRef::Integer(value) => RawSqlValue::Integer(value),
        rusqlite::types::ValueRef::Real(value) => RawSqlValue::Real(value),
        rusqlite::types::ValueRef::Text(value) => {
            RawSqlValue::Text(String::from_utf8_lossy(value).to_string())
        }
        rusqlite::types::ValueRef::Blob(value) => RawSqlValue::Blob(value.to_vec()),
    }
}

fn raw_value(value: &RawSqlValue) -> rusqlite::types::Value {
    match value {
        RawSqlValue::Null => rusqlite::types::Value::Null,
        RawSqlValue::Integer(value) => rusqlite::types::Value::Integer(*value),
        RawSqlValue::Real(value) => rusqlite::types::Value::Real(*value),
        RawSqlValue::Text(value) => rusqlite::types::Value::Text(value.clone()),
        RawSqlValue::Blob(value) => rusqlite::types::Value::Blob(value.clone()),
    }
}

fn json_value(value: &Value) -> rusqlite::types::Value {
    match value {
        Value::Null => rusqlite::types::Value::Null,
        Value::Bool(value) => rusqlite::types::Value::Integer(if *value { 1 } else { 0 }),
        Value::Number(value) => value
            .as_i64()
            .map(rusqlite::types::Value::Integer)
            .or_else(|| value.as_f64().map(rusqlite::types::Value::Real))
            .unwrap_or(rusqlite::types::Value::Null),
        Value::String(value) => rusqlite::types::Value::Text(value.clone()),
        _ => rusqlite::types::Value::Null,
    }
}

struct RawMeta {
    conversation_id: String,
    table: String,
    shard: PathBuf,
    table_sql: Option<String>,
    session: Option<Value>,
}

struct RawRestoreMessage {
    local_id: i64,
    server_id: i64,
    local_type: i64,
    sort_seq: i64,
    real_sender_id: i64,
    create_time: i64,
    status: i64,
    upload_status: i64,
    download_status: i64,
    server_seq: i64,
    origin_source: i64,
    source: RawSqlValue,
    message_content: RawSqlValue,
    compress_content: RawSqlValue,
    packed_info_data: RawSqlValue,
    wcdb_ct_message_content: i64,
    wcdb_ct_source: i64,
    sender_id: String,
}

fn read_raw_backup(path: &Path) -> Result<(RawMeta, Vec<RawRestoreMessage>, Vec<(i64, String)>)> {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open backup {}", path.display()))?;
    let has_meta: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='wcdb_meta'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if has_meta.is_none() {
        bail!(
            "{} is not a raw WCDB backup; export it with `--backup`",
            path.display()
        );
    }
    let (conversation_id, table, shard, table_sql, session_json): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = connection.query_row(
        "SELECT conversation_id,table_name,shard_path,table_sql,session_json FROM wcdb_meta LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    let session = session_json
        .map(|raw| serde_json::from_str(&raw).context("invalid session_json"))
        .transpose()?;
    let mut statement = connection.prepare(
        "SELECT local_id,server_id,local_type,sort_seq,real_sender_id,create_time,
                status,upload_status,download_status,server_seq,origin_source,
                source,message_content,compress_content,packed_info_data,
                wcdb_ct_message_content,wcdb_ct_source,sender_id
         FROM wcdb_messages ORDER BY create_time, local_id",
    )?;
    let mut rows = statement.query([])?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next()? {
        messages.push(RawRestoreMessage {
            local_id: row.get(0)?,
            server_id: row.get(1)?,
            local_type: row.get(2)?,
            sort_seq: row.get(3)?,
            real_sender_id: row.get(4)?,
            create_time: row.get(5)?,
            status: row.get(6)?,
            upload_status: row.get(7)?,
            download_status: row.get(8)?,
            server_seq: row.get(9)?,
            origin_source: row.get(10)?,
            source: sql_value(row.get_ref(11)?),
            message_content: sql_value(row.get_ref(12)?),
            compress_content: sql_value(row.get_ref(13)?),
            packed_info_data: sql_value(row.get_ref(14)?),
            wcdb_ct_message_content: row.get(15)?,
            wcdb_ct_source: row.get(16)?,
            sender_id: row.get(17)?,
        });
    }
    let mut name2id = Vec::new();
    if connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='wcdb_name2id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some()
    {
        let mut statement = connection.prepare("SELECT real_sender_id,user_name FROM wcdb_name2id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            name2id.push((row.get(0)?, row.get(1)?));
        }
    }
    Ok((
        RawMeta {
            conversation_id,
            table,
            shard: PathBuf::from(shard),
            table_sql,
            session,
        },
        messages,
        name2id,
    ))
}

fn find_target_shard(
    target_root: &Path,
    preferred: &Path,
    table: &str,
    key: &[u8; 32],
) -> Result<Option<PathBuf>> {
    if !preferred.is_absolute() {
        let direct = target_root.join(preferred);
        if direct.is_file() {
            return Ok(Some(direct));
        }
    }
    let mut databases = Vec::new();
    collect_database_files(target_root, &mut databases)?;
    for path in databases {
        let Ok(db) = Db::open(&path, Some(key), false) else {
            continue;
        };
        if db
            .tables()?
            .iter()
            .any(|name| name.eq_ignore_ascii_case(table))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn ensure_name2id(
    connection: &rusqlite::Connection,
    sender_id: &str,
) -> Result<i64> {
    if sender_id.is_empty() {
        return Ok(0);
    }
    let table = schema_tables(connection, "main")?
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case("Name2Id"));
    let Some(table) = table else {
        return Ok(0);
    };
    let columns = schema_columns(connection, "main", &table)?;
    let user_column = columns
        .iter()
        .find(|name| name.eq_ignore_ascii_case("user_name") || name.eq_ignore_ascii_case("username"))
        .cloned()
        .unwrap_or_else(|| "user_name".to_string());
    let id: Option<i64> = connection
        .query_row(
            &format!(
                "SELECT rowid FROM {} WHERE {} = ?1 LIMIT 1",
                quote_ident(&table),
                quote_ident(&user_column)
            ),
            [sender_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        return Ok(id);
    }
    if columns.iter().any(|name| name.eq_ignore_ascii_case("is_group")) {
        connection.execute(
            &format!(
                "INSERT INTO {} ({}, is_group) VALUES (?1, 0)",
                quote_ident(&table),
                quote_ident(&user_column)
            ),
            [sender_id],
        )?;
    } else {
        connection.execute(
            &format!(
                "INSERT INTO {} ({}) VALUES (?1)",
                quote_ident(&table),
                quote_ident(&user_column)
            ),
            [sender_id],
        )?;
    }
    Ok(connection.last_insert_rowid())
}

fn restore_partial_one(
    backup: &Path,
    target_root: &Path,
    session_db: Option<&Path>,
    key: &[u8; 32],
) -> Result<(String, usize, usize)> {
    let (meta, messages, name2id) = read_raw_backup(backup)?;
    let shard = find_target_shard(target_root, &meta.shard, &meta.table, key)?
        .with_context(|| format!("target message shard for table {} was not found", meta.table))?;
    let db = Db::open_for_decrypt(&shard, Some(key), false)
        .with_context(|| format!("failed to open target shard {}", shard.display()))?;
    let connection = db.conn();
    connection.execute_batch("PRAGMA foreign_keys=OFF;")?;
    connection.execute_batch("BEGIN IMMEDIATE;")?;
    let result = (|| -> Result<(usize, usize)> {
        let target_tables = schema_tables(connection, "main")?;
        if !target_tables.iter().any(|name| name.eq_ignore_ascii_case(&meta.table)) {
            let Some(table_sql) = &meta.table_sql else {
                bail!("target table {} is missing and no CREATE SQL was backed up", meta.table);
            };
            connection.execute_batch(table_sql)?;
        }
        let target_columns = schema_columns(connection, "main", &meta.table)?;
        for column in [
            "local_id", "server_id", "local_type", "sort_seq", "real_sender_id",
            "create_time", "status", "upload_status", "download_status", "server_seq",
            "origin_source", "source", "message_content", "compress_content",
            "packed_info_data", "WCDB_CT_message_content", "WCDB_CT_source",
        ] {
            if !target_columns.iter().any(|name| name.eq_ignore_ascii_case(column)) {
                bail!("target table {} is missing column {column}", meta.table);
            }
        }
        for (id, user) in &name2id {
            if *id > 0 && !user.is_empty() {
                ensure_name2id(connection, user)?;
            }
        }
        let table_name = quote_ident(&meta.table);
        let mut inserted = 0usize;
        let mut skipped = 0usize;
        for row in &messages {
            let target_sender = match ensure_name2id(connection, &row.sender_id) {
                Ok(0) => row.real_sender_id,
                Ok(id) => id,
                Err(_) => row.real_sender_id,
            };
            let exists: bool = connection.query_row(
                &format!(
                    "SELECT 1 FROM {table_name} WHERE local_id = ?1 OR (server_id != 0 AND server_id = ?2) LIMIT 1"
                ),
                rusqlite::params![row.local_id, row.server_id],
                |_| Ok(true),
            ).optional()?.unwrap_or(false);
            if exists {
                skipped += 1;
                continue;
            }
            connection.execute(
                &format!(
                    "INSERT INTO {table_name}
                     (local_id,server_id,local_type,sort_seq,real_sender_id,create_time,status,
                      upload_status,download_status,server_seq,origin_source,source,message_content,
                      compress_content,packed_info_data,WCDB_CT_message_content,WCDB_CT_source)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"
                ),
                rusqlite::params![
                    row.local_id,
                    row.server_id,
                    row.local_type,
                    row.sort_seq,
                    target_sender,
                    row.create_time,
                    row.status,
                    row.upload_status,
                    row.download_status,
                    row.server_seq,
                    row.origin_source,
                    raw_value(&row.source),
                    raw_value(&row.message_content),
                    raw_value(&row.compress_content),
                    raw_value(&row.packed_info_data),
                    row.wcdb_ct_message_content,
                    row.wcdb_ct_source,
                ],
            )?;
            inserted += 1;
        }
        connection.execute_batch("COMMIT;")?;
        Ok((inserted, skipped))
    })();
    if result.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    drop(db);
    let (inserted, skipped) = result?;
    if let Some(session) = &meta.session {
        restore_session(session_db, key, &meta.conversation_id, session)?;
    }
    Ok((meta.conversation_id, inserted, skipped))
}

fn restore_session(
    session_db: Option<&Path>,
    key: &[u8; 32],
    conversation_id: &str,
    session: &Value,
) -> Result<()> {
    let Some(path) = session_db else {
        return Ok(());
    };
    if !path.is_file() {
        return Ok(());
    }
    let Some(object) = session.as_object() else {
        return Ok(());
    };
    let db = Db::open_for_decrypt(path, Some(key), false)?;
    let connection = db.conn();
    let target_tables = schema_tables(connection, "main")?;
    let Some(table) = target_tables
        .iter()
        .find(|name| name.eq_ignore_ascii_case("SessionTable"))
        .cloned()
    else {
        return Ok(());
    };
    let target_columns = schema_columns(connection, "main", &table)?;
    let exists: Option<i64> = connection
        .query_row(
            &format!(
                "SELECT rowid FROM {} WHERE username = ?1 LIMIT 1",
                quote_ident(&table)
            ),
            [conversation_id],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_some() {
        return Ok(());
    }
    let mut columns = Vec::new();
    let mut values = Vec::new();
    for (column, value) in object {
        if target_columns.iter().any(|name| name.eq_ignore_ascii_case(column)) {
            columns.push(quote_ident(column));
            values.push(json_value(value));
        }
    }
    let username_column = target_columns
        .iter()
        .find(|name| name.eq_ignore_ascii_case("username"))
        .cloned();
    if !columns.iter().any(|column| column.eq_ignore_ascii_case(&quote_ident("username"))) {
        if let Some(username) = username_column {
            columns.push(quote_ident(&username));
            values.push(rusqlite::types::Value::Text(conversation_id.to_string()));
        }
    }
    if columns.is_empty() {
        return Ok(());
    }
    let placeholders = (1..=values.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = connection.prepare(&format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_ident(&table),
        columns.join(","),
        placeholders
    ))?;
    statement.execute(rusqlite::params_from_iter(values))?;
    Ok(())
}

fn run_restore_partial(options: &ValidatedCli) -> Result<()> {
    let input = options
        .recovery_input
        .as_deref()
        .context("recovery input path is missing")?;
    let target = options
        .target_database
        .as_deref()
        .context("target database is missing")?;
    let key = options.key.as_ref().context("database key is missing")?;
    let layout = crate::wechat::discover::discover(target)?;
    let target_root = layout
        .db_storage
        .context("target db_storage path is missing")?;
    let mut backups = Vec::new();
    if input.is_file() {
        backups.push(input.to_path_buf());
    } else {
        for entry in walk_files(input)? {
            let name = entry.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if entry
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("sqlite"))
                || name.eq_ignore_ascii_case("backup.db")
            {
                backups.push(entry);
            }
        }
    }
    backups.sort();
    if backups.is_empty() {
        bail!(
            "recovery input contains no raw SQLite backup; run `conversation --export --backup` first"
        );
    }
    let target_databases = {
        let mut files = Vec::new();
        collect_database_files(&target_root, &mut files)?;
        files
    };
    let backup_root = make_restore_backup(&target_root, &target_databases, options.backup_dir.as_deref())?;
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for backup in &backups {
        match restore_partial_one(
            backup,
            &target_root,
            layout.session_db.as_deref(),
            key,
        ) {
            Ok((conversation_id, inserted, skipped)) => restored.push(serde_json::json!({
                "backup": backup,
                "conversation_id": conversation_id,
                "inserted": inserted,
                "skipped": skipped,
            })),
            Err(error) => failed.push(format!("{}: {error:#}", backup.display())),
        }
    }
    let report = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "input": input,
        "target_database": target_root,
        "target_backup": backup_root,
        "restored": restored,
        "failed": failed,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !failed.is_empty() {
        bail!("{} backup(s) could not be restored; target backup: {}", failed.len(), backup_root.display());
    }
    Ok(())
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return Ok(out);
    }
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(walk_files(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}


fn run_restore_archive(options: &ValidatedCli) -> Result<()> {
    let archive_path = options
        .recovery_input
        .as_deref()
        .context("recovery input path is missing")?;
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("failed to open {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive {}", archive_path.display()))?;
    let staging = tempfile::tempdir().context("failed to create archive extraction directory")?;
    let mut has_backup = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            bail!("archive contains an unsafe path: {}", entry.name());
        };
        let destination = staging.path().join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&destination)?;
        std::io::copy(&mut entry, &mut output)?;
        if relative
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("backup.db"))
        {
            has_backup = true;
        }
    }
    if !has_backup {
        bail!("archive {} contains no backup.db", archive_path.display());
    }
    let mut extracted = options.clone();
    extracted.recovery_input = Some(staging.path().to_path_buf());
    run_restore_partial(&extracted)
}
