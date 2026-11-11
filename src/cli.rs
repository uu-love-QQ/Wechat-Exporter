use anyhow::{bail, Context, Result};
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
  4. 11 种导出格式：
     - json          (完整结构化 JSON，含会话元数据)
     - markdown      (排版美观的 Markdown 聊天记录)
     - chatlab-json  (ChatLab 标准交换格式 v0.0.2 JSON)
     - chatlab-jsonl (ChatLab 大数据量流式行式 JSONL)
     - html          (内置离线即时搜索界面的自包含单文件 HTML)
     - txt           (标准纯文本流式时间线)
     - xlsx          (标准 Excel 表格文件)
     - arkme-json    (包含独立发件人索引与群成员列表的紧凑 JSON)
     - sql           (包含创建表与全部 INSERT 语句的标准 SQL 文件)
     - sqlite        (标准 SQLite 数据库，适合备份与恢复)
     - weclone       (标准 WeClone 带 BOM CSV 格式)

【使用方式】
  wechat-exporter <COMMAND> [OPTIONS]

  init    初始化配置并保存主密钥/图片密钥
  detect  检索本机微信数据库位置与账号
  contact 列出、搜索联系人与群聊
  search  全文检索
  conversation  列出会话；--export 导出会话，可附带媒体
  report  生成私聊/群聊/双人/年度分析报告
  moments 分析朋友圈发帖、收赞、点赞与好友排行
  favorite 获取微信收藏列表
  decrypt  解密数据库到指定目录
  recovery 恢复 SQLite 备份并写回微信数据库
  backup  配置 WebDAV 自动备份
  upgrade 检查并安装新版本

  查看某个子命令的完整参数：
    wechat-exporter conversation --help

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
    /// 列出、搜索联系人及群聊
    Contact(ContactArgs),
    /// 跨会话全文关键词检索 (支持 zstd 压缩内容)
    Search(SearchArgs),
    /// 列出会话；加 --export 导出为 JSON/Markdown/HTML 等格式
    Conversation(ConversationArgs),
    /// 生成私聊/群聊/双人/年度分析报告
    Report(ReportArgs),
    /// 分析朋友圈：发帖类型、收到的赞、给出的赞与好友排行
    Moments(MomentsArgs),
    /// 获取微信收藏列表
    Favorite(FavoriteArgs),
    /// 将加密数据库解密为普通 SQLite 到指定目录
    Decrypt(DecryptArgs),
    /// 恢复 SQLite 备份并写回微信原始数据库
    Recovery(RecoveryArgs),
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
    #[arg(
        long = "self-username",
        value_name = "WXID",
        help_heading = "数据库与密钥"
    )]
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

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactKind {
    /// 全部联系人与群聊
    All,
    /// 好友（含企业好友）
    Friend,
    /// 群聊（含企业群聊）
    Group,
    /// 公众号（订阅号）
    Official,
    /// 服务号
    Service,
    /// 企业好友
    Enterprise,
    /// 企业群聊
    EnterpriseGroup,
    /// 曾经的好友
    Former,
    /// 黑名单
    Blocked,
}

#[derive(Args, Debug)]
pub struct ContactArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 按 ID、昵称、备注或别名过滤
    #[arg(
        short = 'q',
        long = "query",
        value_name = "TEXT",
        help_heading = "查询"
    )]
    pub query: Option<String>,

    /// 联系人类型
    #[arg(long = "kind", value_enum, default_value_t = ContactKind::All, help_heading = "查询")]
    pub kind: ContactKind,

    /// 单页数量 (1..1000，缺省 50)
    #[arg(
        long = "page-size",
        short = 'p',
        value_name = "N",
        help_heading = "分页"
    )]
    pub page_size: Option<u32>,

    /// 分页游标 (上一页返回的 next_cursor)
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

    /// 导出格式 (可重复传入，或传入 'all'，支持: json, markdown, chatlab-json, chatlab-jsonl, html, txt, xlsx, arkme-json, sql, sqlite, weclone)
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

    /// 导出完成后自动上传到已配置的 WebDAV
    #[arg(long = "upload", action = ArgAction::SetTrue, help_heading = "导出")]
    pub upload: bool,

    /// 生成 backup.zip：包含 backup.db、HTML、图片/语音/表情等媒体；仅选定会话/时间范围
    #[arg(long = "backup", action = ArgAction::SetTrue, help_heading = "备份")]
    pub backup: bool,

    /// 不打印进度条
    #[arg(long = "silent", action = ArgAction::SetTrue, help_heading = "其他")]
    pub silent: bool,
}

#[derive(Args, Debug)]
pub struct ConversationArgs {
    #[command(flatten)]
    pub export: ExportArgs,

    /// 执行导出；缺省时列出会话
    #[arg(long = "export", action = ArgAction::SetTrue, help_heading = "导出")]
    pub export_mode: bool,

    /// 按会话名称、备注、昵称或 ID 过滤会话
    #[arg(
        short = 'q',
        long = "query",
        value_name = "TEXT",
        help_heading = "查询"
    )]
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
    #[arg(
        long = "conversation-id",
        short = 'c',
        value_name = "ID",
        help_heading = "报告"
    )]
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

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FavoriteType {
    /// 全部收藏
    All,
    /// 文本
    Text,
    /// 图片
    Image,
    /// 语音
    Voice,
    /// 视频
    Video,
    /// 文章/链接
    Article,
    /// 文件
    File,
    /// 聊天记录
    ChatRecord,
    /// 位置
    Location,
    /// 笔记
    Note,
    /// 小程序
    MiniProgram,
    /// 名片
    Card,
    /// 其他
    Other,
}

#[derive(Args, Debug)]
pub struct DecryptArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 解密后的普通 SQLite 输出目录
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        required = true,
        help_heading = "解密"
    )]
    pub output_dir: PathBuf,

    /// 覆盖已存在的目标数据库
    #[arg(long = "force", action = ArgAction::SetTrue, help_heading = "解密")]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct FavoriteArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 收藏类型
    #[arg(long = "type", value_enum, default_value_t = FavoriteType::All, help_heading = "收藏")]
    pub kind: FavoriteType,

    /// 搜索收藏内容、来源或聊天名
    #[arg(
        short = 'q',
        long = "query",
        value_name = "TEXT",
        help_heading = "收藏"
    )]
    pub query: Option<String>,

    /// 单页数量 (1..1000，缺省 50)
    #[arg(long = "limit", short = 'n', value_name = "N", help_heading = "收藏")]
    pub limit: Option<u32>,

    /// 分页偏移量
    #[arg(long = "offset", value_name = "N", help_heading = "收藏")]
    pub offset: Option<u32>,

    /// 输出完整收藏内容（默认只输出预览）
    #[arg(long = "full", action = ArgAction::SetTrue, help_heading = "收藏")]
    pub full: bool,

    /// 导出收藏清单与每条原始内容到该目录
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        help_heading = "收藏"
    )]
    pub output_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct MomentsArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 只统计指定年份；缺省或 0 表示全部历史
    #[arg(long = "year", value_name = "YYYY", help_heading = "朋友圈")]
    pub year: Option<i32>,

    /// 将朋友圈统计写入该目录下的 moments.json；缺省直接打印 JSON
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        help_heading = "朋友圈"
    )]
    pub output_dir: Option<PathBuf>,

    /// 输出朋友圈时间线（JSON timeline 数组）
    #[arg(long = "posts", action = ArgAction::SetTrue, help_heading = "朋友圈")]
    pub posts: bool,

    /// 时间线条数，范围 1..1000，缺省 50
    #[arg(long = "limit", value_name = "N", help_heading = "朋友圈")]
    pub limit: Option<u32>,

    /// 时间线偏移量
    #[arg(long = "offset", value_name = "N", help_heading = "朋友圈")]
    pub offset: Option<u32>,

    /// 按作者 WXID 或昵称过滤时间线
    #[arg(long = "user", value_name = "TEXT", help_heading = "朋友圈")]
    pub user: Option<String>,

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
    #[arg(
        long = "remote-path",
        value_name = "PATH",
        default_value = "wechat-exports"
    )]
    pub remote_path: String,
}

#[derive(Args, Debug)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommand,
}

#[derive(Args, Debug)]
pub struct RecoveryArgs {
    /// backup.zip，或 decrypt 生成的普通 SQLite 备份文件/目录
    #[arg(
        long = "input",
        visible_alias = "backup",
        short = 'i',
        value_name = "PATH",
        required = true,
        help_heading = "恢复"
    )]
    pub input: PathBuf,

    /// 写回微信原始 db_storage；与 --output-dir 二选一
    #[arg(
        long = "target-database",
        visible_alias = "source-database",
        short = 't',
        value_name = "DIR",
        help_heading = "写回微信数据库"
    )]
    pub target_database: Option<PathBuf>,

    /// 覆盖前写入备份的目标数据库确认开关
    #[arg(long = "force", action = ArgAction::SetTrue, help_heading = "写回微信数据库")]
    pub force: bool,

    /// 写回前备份目标数据库的目录；缺省自动创建在 db_storage 同级的隐藏目录
    #[arg(long = "backup-dir", value_name = "DIR", help_heading = "写回微信数据库")]
    pub backup_dir: Option<PathBuf>,

    /// 64 位 Hex 数据库密钥
    #[arg(long = "key", short = 'K', value_name = "HEX", help_heading = "写回微信数据库")]
    pub key: Option<String>,

    /// 包含 64 位 Hex 数据库密钥的文件
    #[arg(long = "key-file", value_name = "PATH", help_heading = "写回微信数据库")]
    pub key_file: Option<PathBuf>,

    /// 离线恢复输出目录 (必须尚不存在)；用于 conversation.sqlite 备份
    #[arg(
        long = "output-dir",
        short = 'o',
        value_name = "DIR",
        help_heading = "离线恢复"
    )]
    pub output_dir: Option<PathBuf>,

    /// 离线恢复输出格式 (可重复传入，或传入 'all'，缺省 json)
    #[arg(long = "format", short = 'f', value_name = "NAME", action = ArgAction::Append, help_heading = "离线恢复")]
    pub formats: Vec<String>,

    /// 离线恢复时复制备份中的 media/ 附件
    #[arg(long = "include-media", short = 'm', action = ArgAction::SetTrue, help_heading = "离线恢复")]
    pub include_media: bool,
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
    Sqlite,
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
            Format::Sqlite,
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
            "sqlite" => Format::Sqlite,
            "weclone" => Format::Weclone,
            _ => bail!(
                "export format must be json, markdown, chatlab-json, chatlab-jsonl, html, txt, xlsx, arkme-json, sql, sqlite, or weclone"
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
            Format::Sqlite => "sqlite",
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
            Format::Sqlite => "conversation.sqlite",
            Format::Weclone => "conversation.weclone.csv",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Mode {
    Export,
    List,
    Contacts(ContactKind),
    Search(String),
    DetectDatabases,
    Init,
    Report(ReportKind),
    Moments,
    Favorites(FavoriteType),
    Decrypt,
    Recovery,
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
    pub moments_posts: bool,
    pub moments_limit: u32,
    pub moments_offset: u32,
    pub moments_user: Option<String>,
    pub favorite_type: FavoriteType,
    pub favorite_full: bool,
    pub favorite_offset: u32,
    pub force: bool,
    pub recovery_input: Option<PathBuf>,
    pub target_database: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub from_json: Option<PathBuf>,
    pub silent: bool,
    pub upload: bool,
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
        moments_posts: false,
        moments_limit: 50,
        moments_offset: 0,
        moments_user: None,
        favorite_type: FavoriteType::All,
        favorite_full: false,
        favorite_offset: 0,
        force: false,
        recovery_input: None,
        target_database: None,
        backup_dir: None,
        from_json: None,
        silent: false,
        upload: false,
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
                bail!(
                    "{}",
                    crate::wechat::discover::format_scanned_roots_message()
                );
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

fn validate_export_args(args: ExportArgs) -> Result<ValidatedCli> {
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
    if args.backup && args.output_dir.is_none() {
        bail!("--backup requires --output-dir");
    }
    if (!args.formats.is_empty()
        || !args.conversation_ids.is_empty()
        || args.start_time_ms.is_some()
        || args.end_time_ms.is_some())
        && args.output_dir.is_none()
        && !args.keep_output
    {
        bail!("--format requires --output-dir or --keep-output");
    }
    if !args.conversation_ids.is_empty() && args.output_dir.is_none() && !args.keep_output {
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
    options.upload = args.upload;
    options.backup = args.backup;
    Ok(options)
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
                moments_posts: false,
                moments_limit: 50,
                moments_offset: 0,
                moments_user: None,
                favorite_type: FavoriteType::All,
                favorite_full: false,
                favorite_offset: 0,
                force: false,
                recovery_input: None,
                target_database: None,
                backup_dir: None,
                from_json: None,
                silent: false,
                upload: false,
                backup: false,
            }),
            Command::Init(args) => {
                let common = args.common.validate()?;
                let image_key = parse_image_key(args.image_key.as_deref())?;
                let mut options = validated(common, Mode::Init);
                options.image_key = image_key;
                Ok(options)
            }
            Command::Contact(args) => {
                if let Some(query) = &args.query {
                    if query.trim().is_empty() || query.contains('\0') || query.len() > 512 {
                        bail!("contact query must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
                    }
                }
                let common = args.common.validate()?;
                let page_size = parse_page_size(args.page_size)?;
                let mut options = validated(common, Mode::Contacts(args.kind));
                options.list_query = args.query.map(|query| query.trim().to_string());
                options.page_size = page_size;
                options.cursor = args.cursor;
                Ok(options)
            }
            Command::Search(args) => {
                if args.query.is_empty() || args.query.contains('\0') || args.query.len() > 512 {
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
            Command::Conversation(args) => {
                if args.export_mode {
                    if args.query.is_some() || args.page_size.is_some() || args.cursor.is_some() {
                        bail!("list options cannot be used with --export");
                    }
                    validate_export_args(args.export)
                } else {
                    let has_export_option = args.export.output_dir.is_some()
                        || !args.export.formats.is_empty()
                        || !args.export.conversation_ids.is_empty()
                        || args.export.start_time_ms.is_some()
                        || args.export.end_time_ms.is_some()
                        || args.export.image_key.is_some()
                        || args.export.include_media
                        || args.export.keep_output
                        || args.export.report
                        || args.export.upload
                        || args.export.backup;
                    if has_export_option {
                        bail!("export options require --export");
                    }
                    if let Some(query) = &args.query {
                        if query.trim().is_empty() || query.contains('\0') || query.len() > 512 {
                            bail!("conversation query must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
                        }
                    }
                    let common = args.export.common.validate()?;
                    let page_size = parse_page_size(args.page_size)?;
                    let mut options = validated(common, Mode::List);
                    options.list_query = args.query.map(|query| query.trim().to_string());
                    options.page_size = page_size;
                    options.cursor = args.cursor;
                    Ok(options)
                }
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
                let limit = args.limit.unwrap_or(50);
                if !(1..=1000).contains(&limit) {
                    bail!("moments limit must be 1..1000");
                }
                if let Some(user) = &args.user {
                    if user.trim().is_empty() || user.chars().any(char::is_control) {
                        bail!("moments user filter must be nonempty and contain no control characters");
                    }
                }
                let common = args.common.validate()?;
                let mut options = validated(common, Mode::Moments);
                options.output_dir = args.output_dir;
                options.report_year = year;
                options.silent = args.silent;
                options.moments_posts = args.posts;
                options.moments_limit = limit;
                options.moments_offset = args.offset.unwrap_or(0);
                options.moments_user = args.user.map(|value| value.trim().to_string());
                Ok(options)
            }
            Command::Decrypt(args) => {
                if !args.output_dir.is_absolute() {
                    bail!("decrypt output directory must be an absolute path");
                }
                if let Some(parent) = args.output_dir.parent() {
                    if !parent.exists() {
                        bail!("decrypt output directory parent does not exist");
                    }
                }
                if args.output_dir.is_file() {
                    bail!("decrypt output directory is an existing file, not a directory");
                }
                let common = args.common.validate()?;
                let mut options = validated(common, Mode::Decrypt);
                options.output_dir = Some(args.output_dir);
                options.force = args.force;
                Ok(options)
            }
            Command::Favorite(args) => {
                if let Some(dir) = &args.output_dir {
                    if !dir.is_absolute() {
                        bail!("favorite output directory must be an absolute path");
                    }
                    if let Some(parent) = dir.parent() {
                        if !parent.exists() {
                            bail!("favorite output directory parent does not exist");
                        }
                    }
                    if dir.is_file() {
                        bail!("favorite output directory is an existing file, not a directory");
                    }
                }
                if let Some(query) = &args.query {
                    if query.trim().is_empty() || query.contains('\0') || query.len() > 512 {
                        bail!("favorite query must be nonempty, contain no NUL, and be at most 512 UTF-8 bytes");
                    }
                }
                let limit = args.limit.unwrap_or(50);
                if !(1..=1000).contains(&limit) {
                    bail!("favorite limit must be 1..1000");
                }
                let common = args.common.validate()?;
                let mut options = validated(common, Mode::Favorites(args.kind));
                options.list_query = args.query.map(|query| query.trim().to_string());
                options.page_size = limit;
                options.favorite_offset = args.offset.unwrap_or(0);
                options.favorite_full = args.full;
                options.favorite_type = args.kind;
                options.output_dir = args.output_dir;
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
            Command::Recovery(args) => {
                if !args.input.exists() {
                    bail!("recovery input does not exist: {}", args.input.display());
                }
                if args.target_database.is_some() {
                    if args.output_dir.is_some() {
                        bail!("--target-database and --output-dir cannot be used together");
                    }
                    if !args.formats.is_empty() {
                        bail!("--format is only valid for offline recovery");
                    }
                    if args.include_media {
                        bail!("--include-media is only valid for offline recovery");
                    }
                    let target = args.target_database.as_ref().unwrap();
                    if !target.exists() {
                        bail!("target database does not exist: {}", target.display());
                    }
                    if !args.force {
                        bail!("writing into the WeChat database requires --force");
                    }
                    if let Some(parent) = target.parent() {
                        if !parent.exists() {
                            bail!("target database parent does not exist");
                        }
                    }
                    if let Some(dir) = &args.backup_dir {
                        if !dir.is_absolute() {
                            bail!("--backup-dir must be an absolute path");
                        }
                        if let Some(parent) = dir.parent() {
                            if !parent.exists() {
                                bail!("--backup-dir parent does not exist");
                            }
                        }
                        if dir.exists() {
                            bail!("--backup-dir already exists: {}", dir.display());
                        }
                    }
                    let key = crate::util::read_key(args.key.as_deref(), args.key_file.as_deref())?
                        .context("--key or --key-file is required when writing into the WeChat database")?;
                    return Ok(ValidatedCli {
                        source_database: PathBuf::from("."),
                        mode: Mode::Recovery,
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
                        key: Some(key),
                        plaintext: false,
                        debug: false,
                        report: false,
                        report_year: 0,
                        moments_posts: false,
                        moments_limit: 50,
                        moments_offset: 0,
                        moments_user: None,
                        favorite_type: FavoriteType::All,
                        favorite_full: false,
                        favorite_offset: 0,
                        force: true,
                        recovery_input: Some(args.input),
                        target_database: args.target_database,
                        backup_dir: args.backup_dir,
                        from_json: None,
                        silent: false,
                        upload: false,
                        backup: false,
                    });
                }
                let Some(output_dir) = args.output_dir else {
                    bail!("recovery requires --target-database <db_storage> (write back to WeChat) or --output-dir <DIR> (offline export)");
                };
                if !output_dir.is_absolute() {
                    bail!("recovery output directory must be an absolute path");
                }
                if let Some(parent) = output_dir.parent() {
                    if !parent.exists() {
                        bail!("recovery output directory parent does not exist");
                    }
                }
                if output_dir.is_file() {
                    bail!("recovery output directory is an existing file, not a directory");
                }
                let formats = if args.formats.is_empty() {
                    vec![Format::Json]
                } else {
                    parse_formats(&args.formats)?
                };
                Ok(ValidatedCli {
                    source_database: PathBuf::from("."),
                    mode: Mode::Recovery,
                    formats,
                    conversation_ids: Vec::new(),
                    start_time_ms: None,
                    end_time_ms: None,
                    page_size: 50,
                    cursor: None,
                    list_query: None,
                    ascending: false,
                    include_media: args.include_media,
                    keep_output: false,
                    output_dir: Some(output_dir),
                    self_username: None,
                    image_key: None,
                    key: None,
                    plaintext: false,
                    debug: false,
                    report: false,
                    report_year: 0,
                    moments_posts: false,
                    moments_limit: 50,
                    moments_offset: 0,
                    moments_user: None,
                    favorite_type: FavoriteType::All,
                    favorite_full: false,
                    favorite_offset: 0,
                    force: false,
                    recovery_input: Some(args.input),
                    target_database: None,
                    backup_dir: None,
                    from_json: None,
                    silent: false,
                    upload: false,
                    backup: false,
                })
            }
            Command::Upgrade(args) => {
                let version = args.version.trim();
                if version.is_empty() || version.len() > 64 || version.contains(char::is_whitespace)
                {
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
                let mut options = validated(
                    common,
                    Mode::Upgrade(version.to_string(), args.prefix.clone()),
                );
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
                            bail!(
                                "{} report does not accept --conversation-id",
                                match args.kind {
                                    ReportKind::Private => "private",
                                    _ => "annual",
                                }
                            );
                        }
                    }
                    ReportKind::Group | ReportKind::Dual => {
                        if args.conversation_id.is_none() && args.from_json.is_none() {
                            bail!(
                                "{} report requires --conversation-id",
                                match args.kind {
                                    ReportKind::Group => "group",
                                    _ => "dual",
                                }
                            );
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
