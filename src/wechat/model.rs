use serde::Serialize;

#[derive(Clone, Debug, Default)]
pub struct Contact {
    pub username: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub is_group: bool,
    pub avatar_url: String,
    /// friend, group, official, former_friend, blocked, other
    pub kind: String,
    /// subscription, service, enterprise, unknown (official accounts only)
    pub official_account_kind: Option<String>,
    pub official_account_type: Option<i64>,
    pub is_enterprise: bool,
    pub is_blocked: bool,
    pub is_former_friend: bool,
}

impl Contact {
    pub fn display_name(&self) -> String {
        for value in [&self.remark, &self.nickname, &self.alias] {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
        self.username.clone()
    }
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub username: String,
    pub unread_count: i64,
    pub sort_timestamp: i64,
    pub last_timestamp: i64,
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: String,
    pub name: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub is_group: bool,
    pub unread_count: i64,
    pub sort_timestamp: i64,
    pub last_timestamp: i64,
    pub message_count: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MediaRef {
    pub md5: String,
    pub candidate_md5s: Vec<String>,
    pub file_name: String,
    pub kind: MediaKind,
    pub cdn_url: Option<String>,
    pub aes_key: Option<[u8; 16]>,
    /// Pixel size declared by the message XML. Used to tell a real original
    /// from a thumbnail after the candidate file has been decrypted.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Plaintext byte length declared by the message XML (`len` / `androidlen`).
    pub declared_size: Option<u64>,
    /// Voice duration in milliseconds (`<voicemsg voicelength="...">`).
    pub duration_ms: Option<u64>,
}

/// Readable notes recovered from WeChat's compressed `<msgsource>` column.
#[derive(Clone, Debug, Default)]
pub struct MessageSource {
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum MediaKind {
    #[default]
    Image,
    Video,
    Voice,
    Emoji,
    File,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
            MediaKind::Voice => "voice",
            MediaKind::Emoji => "emoji",
            MediaKind::File => "file",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Image => "图片",
            MediaKind::Video => "视频",
            MediaKind::Voice => "语音",
            MediaKind::Emoji => "表情",
            MediaKind::File => "文件",
        }
    }
}

/// A quoted message embedded in a reply (`<refermsg>`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct QuoteRef {
    pub server_id: Option<i64>,
    pub sender: String,
    pub content: String,
    pub create_time: Option<i64>,
    pub quoted_type: Option<i64>,
    /// The reply body typed by the sender.
    pub reply: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ChatRecordItem {
    pub data_type: Option<i64>,
    pub text: String,
    pub sender: String,
    pub time: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ChatRecord {
    pub title: String,
    pub desc: String,
    pub items: Vec<ChatRecordItem>,
}

#[derive(Clone, Debug)]
pub struct ParsedContent {
    pub text: String,
    pub type_name: String,
    pub app_subtype: Option<i64>,
    pub media: Option<MediaRef>,
    pub quote: Option<QuoteRef>,
    pub record: Option<ChatRecord>,
}

#[derive(Clone, Debug)]
pub struct RawMessage {
    pub local_id: i64,
    pub server_id: i64,
    pub local_type: i64,
    pub base_type: i64,
    pub sub_type: i64,
    pub sort_seq: i64,
    pub create_time: i64,
    pub sender_id: String,
    pub source: String,
    pub raw_content: String,
    pub packed_info: Vec<u8>,
}

/// A SQLite value preserved exactly enough to round-trip WCDB rows.
#[derive(Clone, Debug)]
pub enum RawSqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// One original WCDB message row plus the sender identity needed to remap
/// `Name2Id` when restoring into another database.
#[derive(Clone, Debug)]
pub struct RawWcdbRow {
    pub local_id: i64,
    pub server_id: i64,
    pub local_type: i64,
    pub sort_seq: i64,
    pub real_sender_id: i64,
    pub create_time: i64,
    pub status: i64,
    pub upload_status: i64,
    pub download_status: i64,
    pub server_seq: i64,
    pub origin_source: i64,
    pub source: RawSqlValue,
    pub message_content: RawSqlValue,
    pub compress_content: RawSqlValue,
    pub packed_info_data: RawSqlValue,
    pub wcdb_ct_message_content: i64,
    pub wcdb_ct_source: i64,
    pub sender_id: String,
}

/// Raw WCDB fields for a single selected conversation.
#[derive(Clone, Debug)]
pub struct RawBackup {
    pub table: String,
    pub shard: std::path::PathBuf,
    pub table_sql: Option<String>,
    pub rows: Vec<RawWcdbRow>,
    pub name2id: Vec<(i64, String)>,
    pub session: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct ExportMessage {
    pub index: usize,
    pub local_id: i64,
    pub server_id: i64,
    pub create_time: i64,
    pub sender_id: String,
    pub sender_name: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub is_send: bool,
    pub local_type: i64,
    pub base_type: i64,
    pub type_name: String,
    pub content: String,
    pub source: MessageSource,
    pub app_subtype: Option<i64>,
    pub media: Option<MediaRef>,
    pub media_path: Option<String>,
    pub quote: Option<QuoteRef>,
    pub record: Option<ChatRecord>,
}

impl ExportMessage {
    pub fn platform_message_id(&self) -> Option<String> {
        if self.server_id > 0 {
            Some(self.server_id.to_string())
        } else if self.local_id > 0 {
            Some(self.local_id.to_string())
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationJson {
    pub id: String,
    pub name: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub unread_count: i64,
    pub is_pinned: bool,
    pub sort_timestamp: i64,
    pub last_timestamp: i64,
    pub message_count: i64,
}
