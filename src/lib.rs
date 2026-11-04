pub mod cli;
pub mod db;
pub mod export;
pub mod util;
pub mod wechat;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cli::{Cli, Mode, ValidatedCli};
use export::ExportContext;
use wechat::conversations::{build_conversations, select_conversations};
use wechat::discover::{auto_detect_candidates, discover, format_scanned_roots_message};
use wechat::model::Conversation;
use wechat::search::search;
use wechat::shards::Store;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn print_welcome_help() {
    println!(
        r#"================================================================================
  Wechat-Exporter: 微信 4.x 本地数据库解密与多格式导出工具 (Rust 双端实现)
================================================================================

[+] 正在自动检索本机已安装的微信 4.x 数据库..."#
    );

    let candidates = auto_detect_candidates();
    if candidates.is_empty() {
        println!("{}", format_scanned_roots_message());
    } else {
        println!("    检测到 {} 个有效账号：", candidates.len());
        for (idx, c) in candidates.iter().enumerate() {
            let mtime_sec = c
                .modified_time
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            println!(
                "    [{}] 账号: {}\n        最后活跃: {}\n        数据库路径: {}",
                idx + 1,
                c.account_id,
                util::format_timestamp(mtime_sec),
                c.db_storage.display()
            );
        }
    }

    println!(
        r#"
--------------------------------------------------------------------------------
【快速使用指南】
--------------------------------------------------------------------------------
1. 列出会话列表（缺省 --source-database 时自动使用最新活跃账号）：
   wechat-exporter --key <64位Hex密钥> --list-conversations --page-size 20

2. 全文关键词搜索（支持检索 zstd 压缩的聊天记录）：
   wechat-exporter --key <64位Hex密钥> --search "合同" --ascending

3. 导出指定聊天为全部 10 种格式并提取媒体图片/视频：
   wechat-exporter --key <64位Hex密钥> --conversation-id "123456789@chatroom" \
     --format all --include-media --output-dir "/path/to/backup"

4. 查看详细帮助与所有高级参数：
   wechat-exporter --help
================================================================================"
"#
    );
}

fn run_detect_databases() -> Result<()> {
    println!("=== 正在检索本机所有 WeChat 4.x 数据库 ===");
    let candidates = auto_detect_candidates();
    if candidates.is_empty() {
        println!("{}", format_scanned_roots_message());
        return Ok(());
    }
    println!("共发现 {} 个候选账号：\n", candidates.len());
    for (i, c) in candidates.iter().enumerate() {
        let mtime_sec = c
            .modified_time
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        println!("[{}] 账号: {}", i + 1, c.account_id);
        println!("    会话库修改时间: {}", util::format_timestamp(mtime_sec));
        println!("    账号根目录:     {}", c.account_root.display());
        println!("    数据库路径:     {}", c.db_storage.display());
        println!();
    }
    println!("提示：在执行 --list-conversations 或导出时，直接运行即可自动使用最新账号，");
    println!("      亦可通过 --source-database <数据库路径> 手动指定具体账号。");
    Ok(())
}

pub fn run() -> Result<()> {
    if std::env::args().len() <= 1 {
        print_welcome_help();
        return Ok(());
    }

    let cli = Cli::parse();
    let options = cli.validate()?;

    if matches!(options.mode, Mode::DetectDatabases) {
        return run_detect_databases();
    }

    let layout = discover(&options.source_database)?;
    let store = Store::open(&layout.message_dbs, options.key.as_ref(), options.plaintext)?;
    let contacts = wechat::contact::load_contacts(
        layout.contact_db.as_deref(),
        options.key.as_ref(),
        options.plaintext,
    );
    let sessions = wechat::session::load_sessions(
        layout.session_db.as_deref(),
        options.key.as_ref(),
        options.plaintext,
    )
    .unwrap_or_default();
    let self_username = store.detect_self_username(
        layout.account_root.as_deref(),
        &contacts,
        options.self_username.as_deref(),
    );
    let mut conversations = build_conversations(&layout, &store, &contacts, &sessions);
    if let Some(self_id) = &self_username {
        conversations.retain(|conversation| &conversation.id != self_id);
    }
    let account_id = self_username
        .clone()
        .or_else(|| {
            layout
                .account_root
                .as_ref()
                .and_then(|root| root.file_name())
                .and_then(|name| name.to_str())
                .map(util::clean_account_id)
        });

    match &options.mode {
        Mode::DetectDatabases => unreachable!(),
        Mode::List => run_list(&options, &conversations, account_id.as_deref()),
        Mode::Search(query) => run_search(
            &options,
            &store,
            &contacts,
            &conversations,
            self_username.as_deref(),
            account_id.as_deref(),
            query,
        ),
        Mode::Export => run_export(
            &options,
            &layout,
            &store,
            &contacts,
            &conversations,
            self_username.as_deref(),
            account_id.as_deref(),
        ),
    }
}

fn run_list(
    options: &ValidatedCli,
    conversations: &[Conversation],
    account_id: Option<&str>,
) -> Result<()> {
    let offset = match &options.cursor {
        Some(cursor) => util::decode_cursor("list", cursor)?.2,
        None => 0,
    } as usize;
    let page_size = options.page_size as usize;
    let end = offset.saturating_add(page_size).min(conversations.len());
    let page: Vec<_> = if offset < conversations.len() {
        conversations[offset..end]
            .iter()
            .map(Conversation::to_json)
            .collect()
    } else {
        Vec::new()
    };
    let next_cursor = if end < conversations.len() {
        Some(util::encode_cursor("list", 0, "", end as u64))
    } else {
        None
    };
    let value = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "account_id": account_id,
        "total": conversations.len(),
        "conversations": page,
        "next_cursor": next_cursor,
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn run_search(
    options: &ValidatedCli,
    store: &Store,
    contacts: &HashMap<String, wechat::model::Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
    account_id: Option<&str>,
    query: &str,
) -> Result<()> {
    let offset = match &options.cursor {
        Some(cursor) => util::decode_cursor("search", cursor)?.2,
        None => 0,
    };
    let names: HashMap<String, String> = conversations
        .iter()
        .map(|conversation| (conversation.id.clone(), conversation.name.clone()))
        .collect();
    let page = search(
        store,
        contacts,
        &names,
        self_username,
        query,
        &options.conversation_ids,
        options.ascending,
        options.page_size,
        offset,
    );
    let items: Vec<serde_json::Value> = page
        .hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "conversation_id": hit.conversation_id,
                "conversation_name": hit.conversation_name,
                "message": export::message_json(&hit.message, false),
            })
        })
        .collect();
    let value = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "account_id": account_id,
        "query": query,
        "total_matches": page.total_matches,
        "items": items,
        "next_cursor": page.next_cursor,
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn default_output_dir() -> PathBuf {
    let millis = now_ms();
    std::env::temp_dir().join(format!("wechat-exports-{millis}"))
}

fn run_export(
    options: &ValidatedCli,
    layout: &wechat::discover::Layout,
    store: &Store,
    contacts: &HashMap<String, wechat::model::Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
    account_id: Option<&str>,
) -> Result<()> {
    let requested = select_conversations(conversations, &options.conversation_ids)?;
    let selected: Vec<Conversation> = requested
        .into_iter()
        .filter(|conversation| conversation.message_count > 0)
        .collect();
    if selected.is_empty() {
        bail!("no readable conversation returned");
    }
    if !options.conversation_ids.is_empty() && selected.len() != options.conversation_ids.len() {
        bail!("explicit conversation did not return readable messages; no fallback was attempted");
    }

    let output_dir = match &options.output_dir {
        Some(dir) => dir.clone(),
        None => default_output_dir(),
    };
    util::ensure_output_dir(&output_dir)?;

    let exported_at_ms = now_ms();
    let self_name = self_username
        .and_then(|id| contacts.get(id))
        .map(|contact| contact.display_name())
        .filter(|value| !value.is_empty())
        .or_else(|| self_username.map(str::to_string))
        .unwrap_or_default();
    let media_index = if options.include_media {
        Some(wechat::media::MediaIndex::build(layout.account_root.as_deref()))
    } else {
        None
    };
    let single_direct = selected.len() == 1 && !options.conversation_ids.is_empty();

    let mut report_conversations = Vec::new();
    let mut total_messages = 0usize;
    for conversation in &selected {
        let mut messages = store.load_messages(
            &conversation.id,
            self_username,
            contacts,
            options.start_time_ms,
            options.end_time_ms,
        )?;
        if messages.is_empty() {
            continue;
        }
        let conversation_dir = if single_direct {
            output_dir.clone()
        } else {
            output_dir.join(conversation.folder_name())
        };
        std::fs::create_dir_all(&conversation_dir)
            .with_context(|| format!("failed to create {}", conversation_dir.display()))?;
        let media = if let (Some(index), true) = (media_index.as_ref(), options.include_media) {
            Some(wechat::media::export_media(
                &mut messages,
                index,
                &conversation_dir,
                options.image_key,
                account_id.as_deref().unwrap_or(""),
            ))
        } else {
            None
        };
        let context = ExportContext {
            conversation,
            messages: &messages,
            self_username,
            self_name: self_name.clone(),
            exported_at_ms,
            include_media: options.include_media,
        };
        let written = export::write_conversation(&context, &conversation_dir, &options.formats)?;
        total_messages += messages.len();
        let documents: serde_json::Map<String, serde_json::Value> = written
            .iter()
            .map(|(name, path)| {
                let relative = path
                    .strip_prefix(&output_dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                (name.clone(), serde_json::json!(relative))
            })
            .collect();
        let mut entry = serde_json::json!({
            "id": conversation.id,
            "name": conversation.name,
            "type": if conversation.is_group { "group" } else { "private" },
            "message_count": messages.len(),
            "documents": documents,
        });
        if let Some(media) = media {
            entry.as_object_mut().expect("object").insert(
                "media".into(),
                serde_json::json!({
                    "saved": media.saved,
                    "failed": media.failed,
                    "failures": media.failures,
                }),
            );
        }
        report_conversations.push(entry);
    }
    if report_conversations.is_empty() {
        bail!("no readable conversation returned");
    }
    let report_path = export::write_report(
        &output_dir,
        account_id,
        exported_at_ms,
        &options.formats,
        &report_conversations,
    )?;
    let result = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "account_id": account_id,
        "output_directory": output_dir,
        "report": report_path,
        "conversation_count": report_conversations.len(),
        "message_count": total_messages,
        "formats": options.formats.iter().map(|format| format.name()).collect::<Vec<_>>(),
        "keep_output": options.keep_output || options.output_dir.is_some(),
    });
    println!("WeChat chat records exported to {}", output_dir.display());
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
