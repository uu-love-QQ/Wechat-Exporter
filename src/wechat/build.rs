use std::collections::HashMap;

use super::model::{Contact, ExportMessage, MediaKind, RawMessage};
use super::parse::{parse_content, strip_sender_prefix};
use super::shards::all_media_from_packed_info;
use crate::util::is_group_id;

pub fn export_message(
    raw: &RawMessage,
    self_username: Option<&str>,
    contacts: &HashMap<String, Contact>,
    conversation_id: &str,
    index: usize,
) -> ExportMessage {
    let parsed = parse_content(
        &raw.raw_content,
        raw.base_type,
        raw.sub_type,
        raw.local_type,
    );
    let mut media = parsed.media;
    let from_packed = all_media_from_packed_info(&raw.packed_info);
    if media.is_none() {
        if let Some(md5) = from_packed.first().cloned() {
            let kind = match raw.base_type {
                3 => Some(MediaKind::Image),
                34 => Some(MediaKind::Voice),
                43 => Some(MediaKind::Video),
                47 => Some(MediaKind::Emoji),
                49 if parsed.app_subtype == Some(6)
                    || (parsed.app_subtype.is_none() && raw.raw_content.contains("<filename>")) =>
                {
                    Some(MediaKind::File)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                media = Some(super::model::MediaRef {
                    md5,
                    candidate_md5s: from_packed.clone(),
                    file_name: String::new(),
                    kind,
                    cdn_url: crate::wechat::parse::extract_cdn_url(&raw.raw_content),
                    aes_key: crate::wechat::parse::extract_aes_key(&raw.raw_content),
                });
            }
        }
    } else if let Some(m) = &mut media {
        for cand in from_packed {
            if !m.candidate_md5s.contains(&cand) && cand != m.md5 {
                m.candidate_md5s.push(cand);
            }
        }
        if m.cdn_url.is_none() {
            m.cdn_url = crate::wechat::parse::extract_cdn_url(&raw.raw_content);
        }
        if m.aes_key.is_none() {
            m.aes_key = crate::wechat::parse::extract_aes_key(&raw.raw_content);
        }
    }
    let mut sender_id = raw.sender_id.clone();
    let (_, prefixed) = strip_sender_prefix(&raw.raw_content);
    if let Some((sender, _)) = prefixed {
        // Group text rows carry the authoritative sender before the first
        // newline. Some historical shards use role numbers in real_sender_id,
        // so prefer this prefix when it is present.
        sender_id = sender;
    } else if sender_id.is_empty() {
        sender_id = if is_group_id(conversation_id) {
            "system".to_string()
        } else {
            conversation_id.to_string()
        };
    }
    let is_send = self_username
        .map(|self_id| sender_id == self_id)
        .unwrap_or(false);

    let (sender_name, nickname, remark, alias) = if is_send {
        match self_username.and_then(|id| contacts.get(id)) {
            Some(contact) => (
                "我".to_string(),
                contact.nickname.clone(),
                contact.remark.clone(),
                contact.alias.clone(),
            ),
            None => (
                "我".to_string(),
                self_username.unwrap_or("").to_string(),
                String::new(),
                String::new(),
            ),
        }
    } else {
        match contacts.get(&sender_id) {
            Some(contact) => (
                contact.display_name(),
                contact.nickname.clone(),
                contact.remark.clone(),
                contact.alias.clone(),
            ),
            None => {
                let name = if sender_id.is_empty() {
                    if is_group_id(conversation_id) {
                        "未知".to_string()
                    } else {
                        contacts
                            .get(conversation_id)
                            .map(|contact| contact.display_name())
                            .unwrap_or_else(|| conversation_id.to_string())
                    }
                } else {
                    sender_id.clone()
                };
                (name, sender_id.clone(), String::new(), String::new())
            }
        }
    };

    ExportMessage {
        index,
        local_id: raw.local_id,
        server_id: raw.server_id,
        create_time: raw.create_time,
        sender_id,
        sender_name,
        nickname,
        remark,
        alias,
        is_send,
        local_type: raw.local_type,
        base_type: raw.base_type,
        type_name: parsed.type_name,
        content: parsed.text,
        source: crate::wechat::parse::parse_message_source(&raw.source),
        app_subtype: parsed.app_subtype,
        media,
        media_path: None,
    }
}
