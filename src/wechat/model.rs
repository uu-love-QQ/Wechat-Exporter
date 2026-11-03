use serde::Serialize;

#[derive(Clone, Debug, Default)]
pub struct Contact {
    pub username: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub is_group: bool,
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

#[derive(Clone, Debug)]
pub struct MediaRef {
    pub md5: String,
    pub file_name: String,
    pub kind: MediaKind,
}

/// Readable fields recovered from the WeChat `source` column. The raw value
/// is an internal `<msgsource>` document and is not useful as a JSON field.
#[derive(Clone, Debug, Default)]
pub struct MessageSource {
    pub signature: String,
    pub silence: bool,
    pub member_count: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
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
}

#[derive(Clone, Debug)]
pub struct ParsedContent {
    pub text: String,
    pub type_name: String,
    pub app_subtype: Option<i64>,
    pub media: Option<MediaRef>,
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
