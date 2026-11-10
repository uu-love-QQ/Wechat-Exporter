use anyhow::{bail, Result};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
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
  1. 自动检索：缺省数据库路径时自动扫描系统标准微信存储目录（Windows/Linux）
  2. 完整解码：支持普通文本、群聊发件人、图片、语音、视频、表情、名片、位置、
              引用回复、文件附件、微信转账、微信小程序、接龙、系统撤回、拍一拍
  3. 媒体提取：按需解密 .dat 图片（传统异或/V1固定AES/V2动态AES）及附件
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

【使用方式】
  wechat-exporter <COMMAND> [OPTIONS]

  init    初始化配置并保存主密钥/图片密钥
  detect  检索本机微信数据库位置与账号
  list    列出会话
  search  全文检索
  export  导出会话，可附带媒体
  report  生成私聊/群聊/双人/年度分析报告
  moments 分析朋友圈发帖、收赞、点赞与好友排行
  backup  配置 WebDAV 自动备份
  upgrade 检查并安装新版本

  查看某个子命令的完整参数：
    wechat-exporter export --help

  密钥可通过命令行参数、密钥文件或 stdin 传入；缺省数据库路径时自动检索。
  分析报告示例：wechat-exporter report private --output-dir /path/to/report
"#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 初始化配置：提取主密钥、图片 AES/XOR 密钥并保存到本地配置文件
    Init(InitArgs),
    /// 自动检索并列出本机微信 4.x 数据库位置与账号
    Detect(DetectArgs),
    /// 列出聊天会话 (ID、昵称、备注、最后活跃时间、消息数)
    List(ListArgs),
    /// 跨会话全文关键词检索 (支持 zstd 压缩内容)
    Search(SearchArgs),
    /// 导出指定会话为 JSON/Markdown/HTML 等格式，可同时提取媒体
    Export(ExportArgs),
    /// 生成私聊/群聊/双人/年度分析报告
    Report(ReportArgs),
    /// 分析朋友圈：发帖类型、收到的赞、给出的赞与好友排行
    Moments(MomentsArgs),
    /// 检查并安装新版本
    Upgrade(UpgradeArgs),
    /// 配置或测试备份提供商
    Backup(BackupArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// 微信 4.x 的 db_storage 目录或单个会话数据库；缺省时自动检索
    #[arg(
        long = "source-database",
        short = 's',
        value_name = "PATH",
        help_heading = "数据库与密钥"
    )]
    pub source_database: Option<PathBuf>,

    /// 64 位 Hex 数据库解密密钥 (与 stdin 流输入等价)
    #[arg(
        long = "key",
        short = 'K',
        value_name = "HEX",
        help_heading = "数据库与密钥"
    )]
    pub key: Option<String>,

    /// 包含 64 位 Hex 数据库密钥的文件路径
    #[arg(long = "key-file", value_name = "PATH", help_heading = "数据库与密钥")]
    pub key_file: Option<PathBuf>,

    /// 手动指定当前微信登录用户账号 wxid
    #[arg(long = "self-username", value_name = "WXID", help_heading = "数据库与密钥")]
    pub self_username: Option<String>,

    /// 将数据库作为普通未加密的 SQLite 数据库打开
    #[arg(
        long = "plaintext",
        action = ArgAction::SetTrue,
        help_heading = "数据库与密钥"
    )]
    pub plaintext: bool,

    /// 开启详细调试输出 (媒体寻址、解密导出与语音探测细节)
    #[arg(long = "debug", action = ArgAction::SetTrue, help_heading = "其他")]
    pub debug: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 解密微信 V2 .dat 加密图片所需的 16 字节 (32 位 Hex) AES 密钥
    #[arg(long = "image-key", value_name = "HEX", help_heading = "数据库与密钥")]
    pub image_key: Option<String>,
}

#[derive(Args, Debug)]
pub struct DetectArgs {
    /// 只扫描指定目录；缺省时使用系统标准微信存储路径
    #[arg(
        long = "source-database",
        short = 's',
        value_name = "PATH",
        help_heading = "数据库"
    )]
    pub source_database: Option<PathBuf>,

    /// 开启详细调试输出
    #[arg(long = "debug", action = ArgAction::SetTrue, help_heading = "其他")]
    pub debug: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 按会话名称、备注、昵称或 ID 过滤会话
    #[arg(short = 'q', long = "query", value_name = "TEXT", help_heading = "查询")]
    pub query: Option<String>,

    /// 单页会话数量 (范围 1..1000，缺省 50)
    #[arg(
        long = "page-size",
        short = 'p',
        value_name = "N",
        help_heading = "分页"
    )]
    pub page_size: Option<u32>,

    /// 分页游标 (Base64 编码，上一页返回的 next_cursor)
    #[arg(long = "cursor", value_name = "BASE64", help_heading = "分页")]
    pub cursor: Option<String>,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// 要检索的关键词 (最多 512 字节，可匹配 zstd 压缩文本)
    #[arg(value_name = "TEXT")]
    pub query: String,

    #[command(flatten)]
    pub common: CommonArgs,

    /// 搜索结果按时间正序排列 (缺省为倒序)
    #[arg(long = "ascending", action = ArgAction::SetTrue, help_heading = "查询")]
    pub ascending: bool,

    /// 单页结果数量 (范围 1..1000，缺省 50)
    #[arg(
        long = "page-size",
        short = 'p',
        value_name = "N",
        help_heading = "分页"
    )]
    pub page_size: Option<u32>,

    /// 分页游标 (Base64 编码，上一页返回的 next_cursor)
    #[arg(long = "cursor", value_name = "BASE64", help_heading = "分页")]
    pub cursor: Option<String>,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 解密微信 V2 .dat 加密图片所需的 16 字节 (32 位 Hex) AES 密钥
    #[arg(long = "image-key", value_name = "HEX", help_heading = "导出")]
    pub image_key: Option<String>,

    /// 指定导出的会话 ID (如 wxid_xxx 或 xxx@chatroom，可重复指定)
    #[arg(
        long = "conversation-id",
        short = 'c',
        value_name = "ID",
        action = ArgAction::Append,
        help_heading = "导出"
    )]
    pub conversation_ids: Vec<String>,

    /// 导出格式 (可重复传入，或传入 'all'，支持: json, markdown, chatlab-json, chatlab-jsonl, html, txt, xlsx, arkme-json, sql, weclone)
    #[arg(
        long = "format",
        short = 'f',
        value_name = "NAME",
        action = ArgAction::Append,
        help_heading = "导出"
    )]
    pub formats: Vec<String>,

    /// 解析并提取媒体文件到输出目录下的 media/ 文件夹
    #[arg(
        long = "include-media",
        short = 'm',
        action = ArgAction::SetTrue,
        help_heading = "导出"
    )]
    pub include_media: bool,

    /// 起始时间范围 (Unix 毫秒时间戳)
    #[arg(long = "start-time-ms", value_name = "MS", help_heading = "导出")]
    pub start_time_ms: Option<i64>,

    /// 结束时间范围 (Unix 毫秒时间戳)
    #[arg(long = "end-time-ms", value_name = "MS", help_heading = "导出")]
    pub end_time_ms: Option<i64>,

    /// 导出的绝对路径目标文件夹 (必须尚不存在)
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        help_heading = "导出"
    )]
    pub output_dir: Option<PathBuf>,

    /// 保留临时输出目录
    #[arg(
        long = "keep-output",
        short = 'k',
        action = ArgAction::SetTrue,
        help_heading = "导出"
    )]
    pub keep_output: bool,

    /// 生成可视化聊天分析报告 conversation.report.html
    #[arg(long = "report", action = ArgAction::SetTrue, help_heading = "导出")]
    pub report: bool,

    /// 导出完成后自动上传到已配置的 WebDAV 备份
    #[arg(long = "backup", action = ArgAction::SetTrue, help_heading = "导出")]
    pub backup: bool,

    /// 不打印进度条
    #[arg(long = "silent", action = ArgAction::SetTrue, help_heading = "其他")]
    pub silent: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    /// 私聊分析：好友消息排行、消息类型、发送比例、时段分布
    Private,
    /// 群聊分析：成员明细、发言排行、活跃时段、媒体内容
    Group,
    /// 双人报告：指定好友的双方聊天分析
    Dual,
    /// 年度报告：按年或跨年度长期统计
    Annual,
}

#[derive(Args, Debug)]
pub struct ReportArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 报告类型
    #[arg(value_name = "KIND", value_enum)]
    pub kind: ReportKind,

    /// 报告输出目录 (绝对路径)
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        required = true,
        help_heading = "报告"
    )]
    pub output_dir: PathBuf,

    /// group/dual 报告指定会话 ID
    #[arg(long = "conversation-id", short = 'c', value_name = "ID", help_heading = "报告")]
    pub conversation_id: Option<String>,

    /// group/dual/annual 报告年份；缺省或 0 表示全部历史
    #[arg(long = "year", value_name = "YYYY", help_heading = "报告")]
    pub year: Option<i32>,

    /// 从已导出的 conversation.json、export-report.json 或导出目录加载，跳过数据库
    #[arg(long = "from-json", value_name = "PATH", help_heading = "报告")]
    pub from_json: Option<PathBuf>,

    /// 不打印进度条
    #[arg(long = "silent", action = ArgAction::SetTrue, help_heading = "其他")]
    pub silent: bool,
}

#[derive(Args, Debug)]
pub struct MomentsArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 只统计指定年份；缺省或 0 表示全部历史
    #[arg(long = "year", value_name = "YYYY", help_heading = "朋友圈")]
    pub year: Option<i32>,

    /// 将朋友圈统计写入该目录下的 moments.json；缺省直接打印 JSON
    #[arg(long = "output-dir", short = 'o', value_name = "DIR", help_heading = "朋友圈")]
    pub output_dir: Option<PathBuf>,

    /// 不打印进度或提示
    #[arg(long = "silent", action = ArgAction::SetTrue, help_heading = "其他")]
    pub silent: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BackupCommand {
    /// 配置 WebDAV 备份
    Webdav(WebdavArgs),
    /// 显示当前备份配置（隐藏密码）
    Show,
    /// 测试 WebDAV 连接
    Test,
    /// 删除备份配置
    Clear,
}

#[derive(Args, Debug, Clone)]
pub struct WebdavArgs {
    /// WebDAV 根地址
    #[arg(long = "url", value_name = "URL")]
    pub url: String,
    /// WebDAV 用户名
    #[arg(long = "username", value_name = "USER")]
    pub username: String,
    /// WebDAV 密码
    #[arg(long = "password", value_name = "PASS")]
    pub password: String,
    /// 远端目录，缺省 wechat-exports
    #[arg(long = "remote-path", value_name = "PATH", default_value = "wechat-exports")]
    pub remote_path: String,
}

#[derive(Args, Debug)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommand,
}

#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// 目标版本号，缺省 latest
    #[arg(long = "version", value_name = "VERSION", default_value = "latest")]
    pub version: String,

    /// 下载前缀，例如 https://ghfast.top/ 或 https://gh-proxy.com/
    #[arg(long = "prefix", value_name = "URL")]
    pub prefix: Option<String>,
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
    Report(ReportKind),
    Moments,
    Upgrade(String, Option<String>),
    Backup(BackupCommand),
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
    pub list_query: Option<String>,
    pub ascending: bool,
    pub include_media: bool,
    pub keep_output: bool,
    pub output_dir: Option<PathBuf>,
    pub self_username: Option<String>,
    pub image_key: Option<[u8; 16]>,
    pub key: Option<[u8; 32]>,
    pub plaintext: bool,
    pub debug: bool,
    pub report: bool,
    pub report_year: i32,
    pub from_json: Option<PathBuf>,
    pub silent: bool,
    pub backup: bool,
}

struct CommonValidated {
    source_database: PathBuf,
    key: Option<[u8; 32]>,
    self_username: Option<String>,
    plaintext: bool,
    debug: bool,
}

impl CommonArgs {
    fn validate(self) -> Result<CommonValidated> {
        let config_source = if self.source_database.is_none() {
            crate::config::ExporterConfig::load(&[]).and_then(|config| {
                let path = PathBuf::from(config.source_database);
                if path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
        } else {
            None
        };
        if let Some(path) = &config_source {
            eprintln!(
                "[+] 从本地配置读取数据库路径: {} (跳过自动检索)",
                path.display()
            );
        }
        let source_database = resolve_source_database(self.source_database.or(config_source))?;
        let key = crate::util::read_key(self.key.as_deref(), self.key_file.as_deref())?;
        Ok(CommonValidated {
            source_database,
            key,
            self_username: self.self_username,
            plaintext: self.plaintext,
            debug: self.debug,
        })
    }
}

fn validated(common: CommonValidated, mode: Mode) -> ValidatedCli {
    ValidatedCli {
        source_database: common.source_database,
        mode,
        formats: Vec::new(),
        conversation_ids: Vec::new(),
        start_time_ms: None,
        end_time_ms: None,
        page_size: 50,
        cursor: None,
        list_query: None,
        ascending: false,
        include_media: false,
        keep_output: false,
        output_dir: None,
        self_username: common.self_username,
        image_key: None,
        key: common.key,
        plaintext: common.plaintext,
        debug: common.debug,
        report: false,
        report_year: 0,
        from_json: None,
        silent: false,
        backup: false,
    }
}

fn resolve_source_database(source: Option<PathBuf>) -> Result<PathBuf> {
    match source {
        Some(path) => {
            if !path.exists() {
                bail!(
                    "source database or db_storage directory does not exist: {}",
                    path.display()
                );
            }
            Ok(path)
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
                for (i, candidate) in candidates.iter().enumerate().skip(1) {
                    eprintln!(
                        "      [{}] 账号: {} | 路径: {}",
                        i + 1,
                        candidate.account_id,
                        candidate.db_storage.display()
                    );
                }
            }
            Ok(selected.db_storage.clone())
        }
    }
}

fn parse_image_key(raw: Option<&str>) -> Result<Option<[u8; 16]>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let cleaned = raw.trim();
    if cleaned.len() != 32 || !cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("--image-key must be exactly 32 hexadecimal characters");
    }
    let bytes = hex::decode(cleaned).expect("validated hex");
    let mut key = [0u8; 16];
    key.copy_from_slice(&bytes);
    Ok(Some(key))
}

fn parse_page_size(value: Option<u32>) -> Result<u32> {
    let page_size = value.unwrap_or(50);
    if !(1..=1000).contains(&page_size) {
        bail!("page size must be 1..1000");
    }
    Ok(page_size)
}

fn parse_formats(values: &[String]) -> Result<Vec<Format>> {
    if values.is_empty() {
        return Ok(Format::all());
    }
    let mut out = Vec::new();
    for value in values {
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
    Ok(out)
}

impl Cli {
    pub fn validate(self) -> Result<ValidatedCli> {
        let Some(command) = self.command else {
            bail!("a subcommand is required; run `wechat-exporter --help`");
        };
        command.validate()
    }
}

impl Command {
    pub fn validate(self) -> Result<ValidatedCli> {
        match self {
            Command::Detect(args) => Ok(ValidatedCli {
                source_database: args.source_database.unwrap_or_else(|| PathBuf::from(".")),
                mode: Mode::DetectDatabases,
                formats: Vec::new(),
                conversation_ids: Vec::new(),
                start_time_ms: None,
                end_time_ms: None,
                page_size: 50,
                cursor: None,
                list_query: None,
                ascending: false,
                include_media: false,
                keep_output: false,
                output_dir: None,
                self_username: None,
                image_key: None,
                key: None,
                plaintext: true,
                debug: args.debug,
                report: false,
                report_year: 0,
                from_json: None,
                silent: false,
                backup: false,
            }),
            Command::Init(args) => {
                let common = args.common.validate()?;
                let image_key = parse_image_key(args.image_key.as_deref())?;
                let mut options = validated(common, Mode::Init);
                options.image_key = image_key;
                Ok(options)
            }
            Command::List(args) => {
                if let Some(query) = &args.query {
                    if query.trim().is_empty() || query.contains('\0') || query.len() > 512 {
                        bail!("list query must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
                    }
                }
                let common = args.common.validate()?;
                let page_size = parse_page_size(args.page_size)?;
                let mut options = validated(common, Mode::List);
                options.list_query = args.query.map(|query| query.trim().to_string());
                options.page_size = page_size;
                options.cursor = args.cursor;
                Ok(options)
            }
            Command::Search(args) => {
                if args.query.is_empty()
                    || args.query.contains('\0')
                    || args.query.len() > 512
                {
                    bail!("search text must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
                }
                let common = args.common.validate()?;
                let page_size = parse_page_size(args.page_size)?;
                let mut options = validated(common, Mode::Search(args.query));
                options.ascending = args.ascending;
                options.page_size = page_size;
                options.cursor = args.cursor;
                Ok(options)
            }
            Command::Export(args) => {
                for id in &args.conversation_ids {
                    if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
                        bail!("conversation ID must be nonempty, at most 1024 bytes, and contain no control characters");
                    }
                }
                if args.start_time_ms.is_some_and(|value| value <= 0) {
                    bail!("export time must be a positive int64 decimal value");
                }
                if args.end_time_ms.is_some_and(|value| value <= 0) {
                    bail!("export time must be a positive int64 decimal value");
                }
                if let (Some(start), Some(end)) = (args.start_time_ms, args.end_time_ms) {
                    if start > end {
                        bail!("export time range is inverted");
                    }
                }
                let formats = parse_formats(&args.formats)?;
                let image_key = parse_image_key(args.image_key.as_deref())?;
                if (!args.formats.is_empty()
                    || !args.conversation_ids.is_empty()
                    || args.start_time_ms.is_some()
                    || args.end_time_ms.is_some())
                    && args.output_dir.is_none()
                    && !args.keep_output
                {
                    bail!("--format requires --output-dir or --keep-output");
                }
                if !args.conversation_ids.is_empty()
                    && args.output_dir.is_none()
                    && !args.keep_output
                {
                    bail!("--conversation-id requires --output-dir or --keep-output");
                }
                if let Some(dir) = &args.output_dir {
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
                let common = args.common.validate()?;
                let mut options = validated(common, Mode::Export);
                options.formats = formats;
                options.conversation_ids = args.conversation_ids;
                options.start_time_ms = args.start_time_ms;
                options.end_time_ms = args.end_time_ms;
                options.include_media = args.include_media;
                options.keep_output = args.keep_output;
                options.output_dir = args.output_dir;
                options.image_key = image_key;
                options.report = args.report;
                options.silent = args.silent;
                options.backup = args.backup;
                Ok(options)
            }
            Command::Moments(args) => {
                if let Some(dir) = &args.output_dir {
                    if !dir.is_absolute() {
                        bail!("moments output directory must be an absolute path");
                    }
                    if let Some(parent) = dir.parent() {
                        if !parent.exists() {
                            bail!("moments output directory parent does not exist");
                        }
                    }
                    if dir.is_file() {
                        bail!("moments output directory is an existing file, not a directory");
                    }
                }
                let year = args.year.unwrap_or(0);
                if year != 0 && !(1970..=2100).contains(&year) {
                    bail!("moments year must be 1970..2100, or 0 for all history");
                }
                let common = args.common.validate()?;
                let mut options = validated(common, Mode::Moments);
                options.output_dir = args.output_dir;
                options.report_year = year;
                options.silent = args.silent;
                Ok(options)
            }
            Command::Backup(args) => {
                let common = CommonValidated {
                    source_database: PathBuf::from("."),
                    key: None,
                    self_username: None,
                    plaintext: true,
                    debug: false,
                };
                let mut options = validated(common, Mode::Backup(args.command));
                options.silent = false;
                Ok(options)
            }
            Command::Upgrade(args) => {
                let version = args.version.trim();
                if version.is_empty() || version.len() > 64 || version.contains(char::is_whitespace) {
                    bail!("upgrade version must be a nonempty tag without whitespace");
                }
                if let Some(prefix) = &args.prefix {
                    if !(prefix.starts_with("http://") || prefix.starts_with("https://")) {
                        bail!("--prefix must start with http:// or https://");
                    }
                }
                let common = CommonValidated {
                    source_database: PathBuf::from("."),
                    key: None,
                    self_username: None,
                    plaintext: true,
                    debug: false,
                };
                let mut options = validated(common, Mode::Upgrade(version.to_string(), args.prefix.clone()));
                options.silent = false;
                Ok(options)
            }
            Command::Report(args) => {
                if !args.output_dir.is_absolute() {
                    bail!("report output directory must be an absolute path");
                }
                if let Some(parent) = args.output_dir.parent() {
                    if !parent.exists() {
                        bail!("report output directory parent does not exist");
                    }
                }
                if args.output_dir.is_file() {
                    bail!("report output directory is an existing file, not a directory");
                }
                if let Some(id) = &args.conversation_id {
                    if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
                        bail!("conversation ID must be nonempty, at most 1024 bytes, and contain no control characters");
                    }
                }
                if let Some(path) = &args.from_json {
                    if !path.exists() {
                        bail!("--from-json path does not exist: {}", path.display());
                    }
                }
                match args.kind {
                    ReportKind::Private | ReportKind::Annual => {
                        if args.conversation_id.is_some() {
                            bail!("{} report does not accept --conversation-id", match args.kind {
                                ReportKind::Private => "private",
                                _ => "annual",
                            });
                        }
                    }
                    ReportKind::Group | ReportKind::Dual => {
                        if args.conversation_id.is_none() && args.from_json.is_none() {
                            bail!("{} report requires --conversation-id", match args.kind {
                                ReportKind::Group => "group",
                                _ => "dual",
                            });
                        }
                    }
                }
                if args.kind == ReportKind::Private && args.year.is_some() {
                    bail!("private report does not accept --year");
                }
                let year = args.year.unwrap_or(0);
                if year != 0 && !(1970..=2100).contains(&year) {
                    bail!("report year must be 1970..2100, or 0 for all history");
                }
                let mut options = if args.from_json.is_some() {
                    let common = CommonValidated {
                        source_database: PathBuf::from("."),
                        key: None,
                        self_username: args.common.self_username,
                        plaintext: true,
                        debug: args.common.debug,
                    };
                    validated(common, Mode::Report(args.kind))
                } else {
                    let common = args.common.validate()?;
                    validated(common, Mode::Report(args.kind))
                };
                options.output_dir = Some(args.output_dir);
                options.conversation_ids = args.conversation_id.into_iter().collect();
                options.report_year = year;
                options.from_json = args.from_json;
                options.silent = args.silent;
                Ok(options)
            }
        }
    }
}
