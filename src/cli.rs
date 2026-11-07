use anyhow::{bail, Result};
use clap::{ArgAction, Parser};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "wechat-exporter",
    version = env!("CARGO_PKG_VERSION"),
    about = "微信 4.x 本地 WCDB 数据库解密与多格式聊天记录导出工具 (Windows / Linux)",
    long_about = r#"
================================================================================
wechat-exporter: 微信 4.x WCDB (SQLCipher 4) 数据库解密、检索与多格式导出工具
================================================================================

本工具支持 Windows 与 Linux 双端原生运行。基于 SQLCipher 4 密码学算法
（AES-256-CBC + HMAC-SHA512 + zstd）直接解密微信 4.x 本地数据库并完成聊天记录提取。
【核心功能】
  1. 自动检索：缺省 --source-database 时自动扫描系统标准微信存储目录（Windows/Linux）
  2. 完整解码：支持普通文本、群聊发件人、图片、语音、视频、表情、名片、位置、
              引用回复、文件附件、微信转账、微信小程序、接龙、系统撤回、拍一拍
  3. 媒体提取：--include-media 自动解密 .dat 图片（传统异或/V1固定AES/V2动态AES）及附件
  4. 10 种导出格式：
     - json          (完整结构化 JSON，含会话元数据)
     - markdown      (排版美观的 Markdown 聊天记录)
     - chatlab-json  (ChatLab 标准交换格式 v0.0.2 JSON)
     - chatlab-jsonl (ChatLab 大数据量流式行式 JSONL)
     - html          (内置离线即时搜索界面的自包含单文件 HTML)
     - txt           (标准纯文本流式时间线)
     - xlsx          (标准 Excel 表格文件)
     - arkme-json    (包含独立发件人索引与群成员列表的紧凑 JSON)
     - sql           (包含创建表与全部 INSERT 语句的标准 SQL 文件)
     - weclone       (标准 WeClone 带 BOM CSV 格式)

【使用示例】
  • 自动检索本机微信数据库：
    wechat-exporter --detect-databases

  • 快速列出会话（自动寻找数据库，通过 stdin 传入 64 位 Hex 密钥）：
    printf '%s' "0123...def" | wechat-exporter --list-conversations --page-size 20
    # 或直接使用 --key 参数：
    wechat-exporter --key <64-HEX> --list-conversations

  • 全文搜索所有消息（包含 zstd 压缩文本）：
    wechat-exporter --key <64-HEX> --search "关键词" --ascending

  • 导出单个群聊为全部 10 种格式并提取媒体：
    wechat-exporter --key <64-HEX> --conversation-id "123456789@chatroom" \
      --format all --include-media --output-dir "/path/to/backup"

  • 手动指定非默认路径数据库：
    wechat-exporter --source-database "/path/to/xwechat_files/wxid_xxx/db_storage" \
      --key <64-HEX> --list-conversations
"#
)]
pub struct Cli {
    /// 执行子命令 (如 'init' 初始化配置并持久化密钥)
    #[arg(value_name = "COMMAND", index = 1)]
    pub command: Option<String>,

    /// 初始化配置：提取主密钥、图片 AES 和 XOR 密钥并保存到本地配置文件，后续直接免扫运行
    #[arg(long = "init", short = 'i', action = ArgAction::SetTrue)]
    pub init: bool,
    #[arg(long = "source-database", short = 's', value_name = "PATH")]
    pub source_database: Option<PathBuf>,

    /// 自动检索并列出本机上所有的微信 4.x 数据库位置与账号
    #[arg(long = "detect-databases", short = 'D', action = ArgAction::SetTrue)]
    pub detect_databases: bool,

    /// 兼容选项：保留用于兼容第三方调用参数，纯 Rust 实现已内置解密能力，无需额外动态库
    #[arg(long = "native-library", value_name = "PATH")]
    pub native_library: Option<PathBuf>,

    /// 兼容选项：保留用于兼容第三方调用参数，纯 Rust 实现已内置解密能力，无需额外动态库
    #[arg(long = "wcdb-library", value_name = "PATH")]
    pub wcdb_library: Option<PathBuf>,
    /// 从正在运行的微信进程中抓取密钥 (本构建不含驱动级注入，请用外部工具提取密钥后传入)
    #[arg(long = "xkey-process-id", value_name = "PID")]
    pub xkey_process_id: Option<u32>,

    /// 列出所有聊天会话 (ID、昵称、备注、最后活跃时间、消息数)
    #[arg(long = "list-conversations", short = 'l', action = ArgAction::SetTrue)]
    pub list_conversations: bool,

    /// 跨会话全文关键词检索 (支持检索 zstd 压缩内容)
    #[arg(long = "search", short = 'q', value_name = "TEXT")]
    pub search: Option<String>,

    /// 搜索结果按时间正序排列 (缺省为倒序)
    #[arg(long = "ascending", action = ArgAction::SetTrue)]
    pub ascending: bool,

    /// 列表或搜索时的单页数量 (范围 1..1000，缺省 50)
    #[arg(long = "page-size", short = 'p', value_name = "N")]
    pub page_size: Option<u32>,

    /// 分页游标 (Base64 编码，上一页返回的 next_cursor)
    #[arg(long = "cursor", value_name = "BASE64")]
    pub cursor: Option<String>,

    /// 解析并提取媒体文件到输出目录下的 media/ 文件夹
    #[arg(long = "include-media", short = 'm', action = ArgAction::SetTrue)]
    pub include_media: bool,

    /// 导出格式 (可重复传入，或传入 'all'，支持: json, markdown, chatlab-json, chatlab-jsonl, html, txt, xlsx, arkme-json, sql, weclone)
    #[arg(long = "format", short = 'f', value_name = "NAME", action = ArgAction::Append)]
    pub formats: Vec<String>,

    /// 指定导出的会话 ID (如 wxid_xxx 或 xxx@chatroom，可重复指定)
    #[arg(long = "conversation-id", short = 'c', value_name = "ID", action = ArgAction::Append)]
    pub conversation_ids: Vec<String>,

    /// 起始时间范围 (Unix 毫秒时间戳)
    #[arg(long = "start-time-ms", value_name = "MS")]
    pub start_time_ms: Option<i64>,

    /// 结束时间范围 (Unix 毫秒时间戳)
    #[arg(long = "end-time-ms", value_name = "MS")]
    pub end_time_ms: Option<i64>,

    /// 保持临时导出的输出目录不被清除
    #[arg(long = "keep-output", short = 'k', action = ArgAction::SetTrue)]
    pub keep_output: bool,

    /// 导出的绝对路径目标文件夹 (必须尚不存在)
    #[arg(long = "output-dir", short = 'o', value_name = "DIR")]
    pub output_dir: Option<PathBuf>,

    /// 手动指定当前微信登录用户账号 wxid
    #[arg(long = "self-username", value_name = "WXID")]
    pub self_username: Option<String>,

    /// 解密微信 V2 .dat 加密图片所需的 16 字节 (32 位 Hex) AES 密钥
    #[arg(long = "image-key", value_name = "HEX")]
    pub image_key: Option<String>,

    /// 包含 64 位 Hex 数据库密钥的文件路径
    #[arg(long = "key-file", value_name = "PATH")]
    pub key_file: Option<PathBuf>,

    /// 64 位 Hex 数据库解密密钥 (与 stdin 流输入等价)
    #[arg(long = "key", short = 'K', value_name = "HEX")]
    pub key: Option<String>,

    /// 将数据库作为普通未加密的 SQLite 数据库打开
    #[arg(long = "plaintext", action = ArgAction::SetTrue)]
    pub plaintext: bool,

    /// 开启详细调试输出 (打印图片/表情的原图与缩略图信息、寻址过程及解密导出细节)
    #[arg(long = "debug", action = ArgAction::SetTrue)]
    pub debug: bool,

    /// 手动指定微信登录 Cookie (`wxuin=..; wxsid=..; skey=..; pass_ticket=..`)
    #[arg(long = "cookie", value_name = "COOKIE")]
    pub cookie: Option<String>,

}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    Markdown,
    ChatlabJson,
    ChatlabJsonl,
    Html,
    Txt,
    Xlsx,
    ArkmeJson,
    Sql,
    Weclone,
}

impl Format {
    pub fn all() -> Vec<Format> {
        vec![
            Format::Json,
            Format::Markdown,
            Format::ChatlabJson,
            Format::ChatlabJsonl,
            Format::Html,
            Format::Txt,
            Format::Xlsx,
            Format::ArkmeJson,
            Format::Sql,
            Format::Weclone,
        ]
    }

    pub fn parse(value: &str) -> Result<Format> {
        Ok(match value {
            "json" => Format::Json,
            "markdown" => Format::Markdown,
            "chatlab-json" => Format::ChatlabJson,
            "chatlab-jsonl" => Format::ChatlabJsonl,
            "html" => Format::Html,
            "txt" => Format::Txt,
            "xlsx" => Format::Xlsx,
            "arkme-json" => Format::ArkmeJson,
            "sql" => Format::Sql,
            "weclone" => Format::Weclone,
            _ => bail!(
                "export format must be json, markdown, chatlab-json, chatlab-jsonl, html, txt, xlsx, arkme-json, sql, or weclone"
            ),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Markdown => "markdown",
            Format::ChatlabJson => "chatlab-json",
            Format::ChatlabJsonl => "chatlab-jsonl",
            Format::Html => "html",
            Format::Txt => "txt",
            Format::Xlsx => "xlsx",
            Format::ArkmeJson => "arkme-json",
            Format::Sql => "sql",
            Format::Weclone => "weclone",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Format::Json => "conversation.json",
            Format::Markdown => "conversation.md",
            Format::ChatlabJson => "conversation.chatlab.json",
            Format::ChatlabJsonl => "conversation.chatlab.jsonl",
            Format::Html => "conversation.html",
            Format::Txt => "conversation.txt",
            Format::Xlsx => "conversation.xlsx",
            Format::ArkmeJson => "conversation.arkme.json",
            Format::Sql => "conversation.sql",
            Format::Weclone => "conversation.weclone.csv",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Mode {
    Export,
    List,
    Search(String),
    DetectDatabases,
    Init,
}

#[derive(Clone, Debug)]
pub struct ValidatedCli {
    pub source_database: PathBuf,
    pub mode: Mode,
    pub formats: Vec<Format>,
    pub conversation_ids: Vec<String>,
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub page_size: u32,
    pub cursor: Option<String>,
    pub ascending: bool,
    pub include_media: bool,
    pub keep_output: bool,
    pub output_dir: Option<PathBuf>,
    pub self_username: Option<String>,
    pub image_key: Option<[u8; 16]>,
    pub key: Option<[u8; 32]>,
    pub plaintext: bool,
    pub debug: bool,
    pub cookie: Option<String>,
}

impl Cli {
    pub fn validate(self) -> Result<ValidatedCli> {
        if self.detect_databases {
            let dummy_source = self.source_database.unwrap_or_else(|| PathBuf::from("."));
            return Ok(ValidatedCli {
                source_database: dummy_source,
                mode: Mode::DetectDatabases,
                formats: Vec::new(),
                conversation_ids: Vec::new(),
                start_time_ms: None,
                end_time_ms: None,
                page_size: 50,
                cursor: None,
                ascending: false,
                include_media: false,
                keep_output: false,
                output_dir: None,
                self_username: None,
                image_key: None,
                key: None,
                plaintext: true,
                debug: self.debug,
                cookie: self.cookie.clone(),
            });
        }

        let is_init = self.command.as_deref() == Some("init") || self.init;
        // Automatic discovery if --source-database is not provided
        let source_database = match self.source_database {
            Some(path) => {
                if !path.exists() {
                    bail!("source database or db_storage directory does not exist: {}", path.display());
                }
                path
            }
            None => {
                let candidates = crate::wechat::discover::auto_detect_candidates();
                if candidates.is_empty() {
                    bail!("{}", crate::wechat::discover::format_scanned_roots_message());
                }
                let selected = &candidates[0];
                eprintln!(
                    "[i] 自动检索到微信数据库: {}\n    (账号: {}, 共检测到 {} 个候选账号)",
                    selected.db_storage.display(),
                    selected.account_id,
                    candidates.len()
                );
                if candidates.len() > 1 {
                    eprintln!("    其他候选账号 (如需切换请使用 --source-database <路径>):");
                    for (i, c) in candidates.iter().enumerate().skip(1) {
                        eprintln!("      [{}] 账号: {} | 路径: {}", i + 1, c.account_id, c.db_storage.display());
                    }
                }
                selected.db_storage.clone()
            }
        };

        if self.xkey_process_id == Some(0) {
            bail!("--xkey-process-id must be greater than zero");
        }
        if self.xkey_process_id.is_some() {
            bail!("XKey process acquisition is not available; pipe the 64-hex database key on stdin");
        }
        if self.list_conversations && self.search.is_some() {
            bail!("--search conflicts with --list-conversations");
        }
        if let Some(text) = &self.search {
            if text.is_empty() || text.contains('\0') || text.len() > 512 {
                bail!("search text must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
            }
        }
        for id in &self.conversation_ids {
            if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
                bail!("conversation ID must be nonempty, at most 1024 bytes, and contain no control characters");
            }
        }
        let page_size = self.page_size.unwrap_or(50);
        if !(1..=1000).contains(&page_size) {
            bail!("page size must be 1..1000");
        }
        if (self.page_size.is_some() || self.cursor.is_some())
            && !(self.list_conversations || self.search.is_some())
        {
            bail!("--page-size and --cursor require --list-conversations or --search");
        }
        if self.ascending && self.search.is_none() {
            bail!("--ascending requires --search");
        }
        if self.start_time_ms.is_some_and(|v| v <= 0) {
            bail!("export time must be a positive int64 decimal value");
        }
        if self.end_time_ms.is_some_and(|v| v <= 0) {
            bail!("export time must be a positive int64 decimal value");
        }
        if let (Some(start), Some(end)) = (self.start_time_ms, self.end_time_ms) {
            if start > end {
                bail!("export time range is inverted");
            }
        }

        let mode = if is_init {
            Mode::Init
        } else if self.list_conversations {
            Mode::List
        } else if let Some(text) = self.search.clone() {
            Mode::Search(text)
        } else {
            Mode::Export
        };

        if matches!(mode, Mode::List) {
            if !self.formats.is_empty()
                || !self.conversation_ids.is_empty()
                || self.include_media
                || self.start_time_ms.is_some()
                || self.end_time_ms.is_some()
                || self.output_dir.is_some()
                || self.keep_output
            {
                bail!("--list-conversations conflicts with export options");
            }
        }

        if matches!(mode, Mode::Search(_)) {
            if !self.formats.is_empty()
                || self.include_media
                || self.start_time_ms.is_some()
                || self.end_time_ms.is_some()
                || self.output_dir.is_some()
                || self.keep_output
            {
                bail!("--search conflicts with export options");
            }
        }

        let formats = if self.formats.is_empty() {
            Format::all()
        } else {
            let mut out = Vec::new();
            for value in &self.formats {
                if value == "all" {
                    for format in Format::all() {
                        if !out.contains(&format) {
                            out.push(format);
                        }
                    }
                } else {
                    let format = Format::parse(value)?;
                    if !out.contains(&format) {
                        out.push(format);
                    }
                }
            }
            out
        };

        if matches!(mode, Mode::Export) {
            if (!self.formats.is_empty()
                || !self.conversation_ids.is_empty()
                || self.start_time_ms.is_some()
                || self.end_time_ms.is_some())
                && self.output_dir.is_none()
                && !self.keep_output
            {
                bail!("--format requires --output-dir or --keep-output");
            }
            if !self.conversation_ids.is_empty() && self.output_dir.is_none() && !self.keep_output {
                bail!("--conversation-id requires --output-dir or --keep-output");
            }
            if let Some(dir) = &self.output_dir {
                if !dir.is_absolute() {
                    bail!("output directory must be an absolute path");
                }
                if let Some(parent) = dir.parent() {
                    if !parent.exists() {
                        bail!("output directory parent does not exist");
                    }
                }
                if dir.is_file() {
                    bail!("output directory is an existing file, not a directory");
                }
            }
        }

        let image_key = match self.image_key.as_deref() {
            Some(raw) => {
                let cleaned = raw.trim();
                if cleaned.len() != 32 || !cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("--image-key must be exactly 32 hexadecimal characters");
                }
                let bytes = hex::decode(cleaned).expect("validated hex");
                let mut key = [0u8; 16];
                key.copy_from_slice(&bytes);
                Some(key)
            }
            None => None,
        };

        let key = crate::util::read_key(self.key.as_deref(), self.key_file.as_deref())?;

        Ok(ValidatedCli {
            source_database,
            mode,
            formats,
            conversation_ids: self.conversation_ids,
            start_time_ms: self.start_time_ms,
            end_time_ms: self.end_time_ms,
            page_size,
            cursor: self.cursor,
            ascending: self.ascending,
            include_media: self.include_media,
            keep_output: self.keep_output,
            output_dir: self.output_dir,
            self_username: self.self_username,
            image_key,
            key,
            plaintext: self.plaintext,
            debug: self.debug,
            cookie: self.cookie,
        })
    }
}
