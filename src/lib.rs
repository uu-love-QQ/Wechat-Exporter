pub mod cli;
pub mod db;
pub mod export;
pub mod report;
pub mod progress;
pub mod upgrade;
pub mod backup;
pub mod util;
pub mod wechat;
pub mod config;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cli::{Cli, Mode, ValidatedCli};
use progress::Progress;
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
1. 检索本机数据库：
   wechat-exporter detect

2. 列出会话列表（缺省 --source-database 时自动使用最新活跃账号）：
   wechat-exporter list --key <64位Hex密钥> --page-size 20

3. 全文关键词搜索（支持检索 zstd 压缩的聊天记录）：
   wechat-exporter search "合同" --key <64位Hex密钥> --ascending

4. 导出指定聊天为全部 10 种格式并提取媒体图片/视频：
   wechat-exporter export --key <64位Hex密钥> --conversation-id "123456789@chatroom" \
     --format all --include-media --output-dir "/path/to/backup"

5. 查看子命令帮助：
   wechat-exporter export --help
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
    println!("提示：执行 list 或 export 时，直接运行即可自动使用最新账号，");
    println!("      亦可通过 --source-database <数据库路径> 手动指定具体账号。");
    Ok(())
}

fn run_init(options: &ValidatedCli, layout: &wechat::discover::Layout) -> Result<()> {
    println!("================================================================================");
    println!("[i] 正在执行微信配置与密钥初始化...");
    println!("    数据库路径: {}", options.source_database.display());
    // 1. Resolve master database key
    let mut db_key = options.key;
    if db_key.is_none() {
        println!("[i] 正在从 Weixin.exe 进程内存中提取主通行码...");
        let p1 = match &layout.session_db {
            Some(p) => std::fs::read(p).ok(),
            None => None,
        };
        let p2 = if p1.is_none() {
            layout.message_dbs.first().and_then(|p| std::fs::read(p).ok())
        } else {
            None
        };
        let p1_ref = p1.as_deref().or(p2.as_deref());
        db_key = db::auto_scan_wechat_key_from_process(p1_ref);
    }

    let Some(raw_key) = db_key else {
        anyhow::bail!("无法从 Weixin.exe 提取主通行码，请确保微信正在运行且已登录，或通过 --key 手动指定");
    };
    let key_hex = hex::encode(raw_key);
    println!("[+] 主通行码就绪: {}", key_hex);

    // 2. Discover account id
    let account_id = layout
        .account_root
        .as_ref()
        .and_then(|r| r.file_name())
        .and_then(|s| s.to_str())
        .map(util::clean_account_id)
        .unwrap_or_else(|| "unknown".to_string());

    // 3. Scan image seed and derive V2 AES & XOR keys
    println!("[i] 正在提取图片种子及 V2 AES/XOR 密钥...");
    let media_index = wechat::media::MediaIndex::build(layout.account_root.as_deref());
    let (image_key_bytes, image_xor_key, image_seed) =
        wechat::media::resolve_image_keys(&media_index, &account_id, options.image_key);
    let image_aes_key = image_key_bytes.map(|k| String::from_utf8_lossy(&k).to_string());

    // 5. Save to config
    let config = config::ExporterConfig {
        account_id: account_id.clone(),
        source_database: options.source_database.to_string_lossy().to_string(),
        key: key_hex.clone(),
        image_seed,
        image_aes_key: image_aes_key.clone(),
        image_xor_key,
        updated_at: now_ms(),
    };

    let cwd_path = config.save(None)?;
    println!("[+] 配置文件已成功保存至当前目录: {}", cwd_path.display());
    if let Some(root) = layout.account_root.as_ref() {
        if let Ok(acc_path) = config.save(Some(root)) {
            println!("[+] 配置文件已备份至账号目录: {}", acc_path.display());
        }
    }

    println!("================================================================================");
    println!("[✓] 初始化完成！已将密钥持久化至本地：");
    println!("    • 微信账号: {}", account_id);
    println!("    • 数据库通行码: {}", key_hex);
    if let Some(aes) = image_aes_key {
        println!("    • 图片 AES 密钥: {}", aes);
    }
    if let Some(xor) = image_xor_key {
        println!("    • 图片 XOR 密钥: 0x{:02x} ({})", xor, xor);
    }
    println!("================================================================================");
    println!("后续执行 export 或 list 子命令时，");
    println!("程序将自动读取 .wechat-exporter.json 配置文件，无需进行内存扫描，即使微信退出也能完整离线导出！");
    Ok(())
}

pub fn run() -> Result<()> {
    if std::env::args().len() <= 1 {
        print_welcome_help();
        return Ok(());
    }

    let cli = Cli::parse();
    let mut options = cli.validate()?;

    if let Mode::Upgrade(version, prefix) = &options.mode {
        return upgrade::run(version, prefix.as_deref());
    }

    if let Mode::Backup(action) = &options.mode {
        return backup::run(action);
    }

    if let Mode::Report(kind) = &options.mode {
        if let Some(path) = &options.from_json {
            return report::run_from_json(*kind, &options, path);
        }
    }

    if matches!(options.mode, Mode::DetectDatabases) {
        return run_detect_databases();
    }

    let layout = discover(&options.source_database)?;

    if matches!(options.mode, Mode::Init) {
        return run_init(&options, &layout);
    }

    // Auto-load config if key is not passed on command line
    let search_roots = [
        options.source_database.as_path(),
        layout.account_root.as_deref().unwrap_or(options.source_database.as_path()),
    ];
    if let Some(cfg) = config::ExporterConfig::load(&search_roots) {
        if options.key.is_none() {
            if let Some(k) = cfg.db_key_bytes() {
                options.key = Some(k);
                println!(
                    "[+] 从本地配置 ({}) 加载数据库通行码: {}...",
                    config::CONFIG_FILE_NAME,
                    &cfg.key[..16]
                );
            }
        }
        if options.image_key.is_none() {
            if let Some(ik) = cfg.image_aes_key_bytes() {
                options.image_key = Some(ik);
                if let (Some(seed), Some(xor)) = (cfg.image_seed, cfg.image_xor_key) {
                    wechat::media::set_image_seed_cache(seed, ik, xor);
                    println!(
                        "[+] 从本地配置读取图片密钥: AES {}, XOR 0x{:02x} (跳过内存扫描)",
                        cfg.image_aes_key.as_deref().unwrap_or(""),
                        xor
                    );
                }
            }
        }
    }

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
        Mode::DetectDatabases | Mode::Init | Mode::Upgrade(_, _) | Mode::Backup(_) => unreachable!(),
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
        Mode::Report(kind) => report::run(
            *kind,
            &options,
            &store,
            &contacts,
            &conversations,
            &layout.message_dbs,
            layout.contact_db.as_deref(),
            layout.account_root.as_deref(),
            layout.db_storage.as_deref(),
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
    let query = options.list_query.as_deref().map(str::to_lowercase);
    let filtered: Vec<&Conversation> = conversations
        .iter()
        .filter(|conversation| {
            let Some(query) = query.as_deref() else {
                return true;
            };
            let haystack = [
                conversation.id.as_str(),
                conversation.name.as_str(),
                conversation.nickname.as_str(),
                conversation.remark.as_str(),
                conversation.alias.as_str(),
            ]
            .join(" ")
            .to_lowercase();
            haystack.contains(query)
        })
        .collect();
    let offset = match &options.cursor {
        Some(cursor) => util::decode_cursor("list", cursor)?.2,
        None => 0,
    } as usize;
    let page_size = options.page_size as usize;
    let end = offset.saturating_add(page_size).min(filtered.len());
    let page: Vec<_> = if offset < filtered.len() {
        filtered[offset..end]
            .iter()
            .map(|conversation| conversation.to_json())
            .collect()
    } else {
        Vec::new()
    };
    let next_cursor = if end < filtered.len() {
        Some(util::encode_cursor("list", 0, "", end as u64))
    } else {
        None
    };
    let value = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "account_id": account_id,
        "query": options.list_query,
        "total": filtered.len(),
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

fn download_avatar(url: &str, conversation_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use sha2::{Digest, Sha256};
    let response = ureq::get(url)
        .set("User-Agent", "MicroMessenger Client")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .ok()?;
    let mut data = Vec::new();
    use std::io::Read;
    response.into_reader().read_to_end(&mut data).ok()?;
    if data.is_empty() {
        return None;
    }
    let ext = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"GIF8") {
        "gif"
    } else if data.starts_with(b"RIFF") {
        "webp"
    } else {
        "jpg"
    };
    let digest = hex::encode(Sha256::digest(&data));
    let dir = conversation_dir.join("media").join("avatar");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{digest}.{ext}"));
    if !path.exists() {
        std::fs::write(&path, &data).ok()?;
    }
    Some(path)
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
    let voice_index = match (options.include_media, layout.db_storage.as_deref()) {
        (true, Some(db_storage)) => Some(wechat::voice::VoiceIndex::build(
            db_storage,
            options.key.as_ref(),
            options.plaintext,
            options.debug,
        )),
        _ => None,
    };
    let single_direct = selected.len() == 1 && !options.conversation_ids.is_empty();

    let mut progress = Progress::new(options.silent);
    let mut report_conversations = Vec::new();
    let mut total_messages = 0usize;
    let mut overall_start = i64::MAX;
    let mut overall_end = i64::MIN;
    for (conversation_index, conversation) in selected.iter().enumerate() {
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
        let attachment_total = messages.iter().filter(|message| message.media.is_some()).count();
        progress.set_phase(format!(
            "导出会话 {}/{} · {}",
            conversation_index + 1,
            selected.len(),
            conversation.name
        ));
        progress.set_totals(messages.len(), attachment_total);
        let conversation_dir = if single_direct {
            output_dir.clone()
        } else {
            output_dir.join(conversation.folder_name())
        };
        std::fs::create_dir_all(&conversation_dir)
            .with_context(|| format!("failed to create {}", conversation_dir.display()))?;
        let media = if let (Some(index), true) = (media_index.as_ref(), options.include_media) {
            let mut on_media = |done: usize, total: usize, attachment_done: usize, attachment_total: usize| {
                progress.messages(done, total);
                progress.attachments(attachment_done, attachment_total);
            };
            let outcome = wechat::media::export_media(
                &mut messages,
                index,
                &conversation_dir,
                options.image_key,
                account_id.as_deref().unwrap_or(""),
                options.debug,
                &conversation.id,
                voice_index.as_ref(),
                Some(&mut on_media),
            );
            progress.attachments(attachment_total, attachment_total);
            Some(outcome)
        } else {
            None
        };
        let wants_html = options.formats.iter().any(|format| *format == cli::Format::Html);
        let avatar_path = if options.include_media && wants_html {
            contacts
                .get(&conversation.id)
                .and_then(|contact| {
                    let url = contact.avatar_url.trim();
                    if url.is_empty() {
                        None
                    } else {
                        download_avatar(url, &conversation_dir).map(|path| {
                            path.strip_prefix(&conversation_dir)
                                .unwrap_or(path.as_path())
                                .to_string_lossy()
                                .replace('\\', "/")
                        })
                    }
                })
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
            avatar_path,
        };
        progress.set_phase(format!(
            "写入会话 {}/{} · {}",
            conversation_index + 1,
            selected.len(),
            conversation.name
        ));
        let written = export::write_conversation(
            &context,
            &conversation_dir,
            &options.formats,
            options.report,
        )?;
        progress.messages(messages.len(), messages.len());
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
        let start_time = messages.iter().map(|message| message.create_time).min().unwrap_or(0);
        let end_time = messages.iter().map(|message| message.create_time).max().unwrap_or(0);
        overall_start = overall_start.min(start_time);
        overall_end = overall_end.max(end_time);
        let mut entry = serde_json::json!({
            "id": conversation.id,
            "name": conversation.name,
            "type": if conversation.is_group { "group" } else { "private" },
            "message_count": messages.len(),
            "start_time": start_time,
            "end_time": end_time,
            "start_time_text": util::format_timestamp(start_time),
            "end_time_text": util::format_timestamp(end_time),
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
    progress.set_phase("生成导出报告");
    let report_path = export::write_report(
        &output_dir,
        account_id,
        exported_at_ms,
        &options.formats,
        &report_conversations,
        if overall_start == i64::MAX { 0 } else { overall_start },
        if overall_end == i64::MIN { 0 } else { overall_end },
    )?;
    let backup_summary = if options.backup {
        progress.set_phase("上传 WebDAV 备份");
        Some(
            backup::upload_directory(&output_dir, &mut progress)
                .with_context(|| format!("export completed at {} but WebDAV backup failed", output_dir.display()))?,
        )
    } else {
        None
    };
    progress.finish();
    if let Some(summary) = &backup_summary {
        println!("[+] WebDAV 备份完成: {} ({} 个文件)", summary.remote, summary.files);
    }
    let result = serde_json::json!({
        "schema": util::SCHEMA,
        "generator": util::GENERATOR,
        "account_id": account_id,
        "output_directory": output_dir,
        "report": report_path,
        "conversation_count": report_conversations.len(),
        "message_count": total_messages,
        "start_time": if overall_start == i64::MAX { 0 } else { overall_start },
        "end_time": if overall_end == i64::MIN { 0 } else { overall_end },
        "formats": options.formats.iter().map(|format| format.name()).collect::<Vec<_>>(),
        "keep_output": options.keep_output,
        "output_dir_explicit": options.output_dir.is_some(),
        "backup": backup_summary.as_ref().map(|summary| serde_json::json!({
            "remote": summary.remote,
            "files": summary.files,
        })),
    });
    println!("WeChat chat records exported to {}", output_dir.display());
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod avatar_tests {
    use super::download_avatar;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn downloads_and_content_addresses_avatar() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 512];
            let _ = stream.read(&mut buffer);
            let body: &[u8] = b"\xff\xd8\xff\xe0avatar";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        });
        let dir = tempfile::TempDir::new().unwrap();
        let path = download_avatar(&format!("http://{address}/a"), dir.path()).expect("avatar");
        server.join().unwrap();
        assert!(path.starts_with(dir.path().join("media").join("avatar")));
        assert!(path.to_string_lossy().ends_with(".jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"\xff\xd8\xff\xe0avatar");
    }
}
