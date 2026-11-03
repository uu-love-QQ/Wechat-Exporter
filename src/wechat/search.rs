use std::collections::{HashMap, HashSet};

use super::build::export_message;
use super::model::{Contact, ExportMessage};
use super::shards::Store;
use crate::util::md5_hex;

pub struct SearchHit {
    pub conversation_id: String,
    pub conversation_name: String,
    pub message: ExportMessage,
}

pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
}

pub fn search(
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversation_names: &HashMap<String, String>,
    self_username: Option<&str>,
    query: &str,
    conversation_ids: &[String],
    ascending: bool,
    page_size: u32,
    offset: u64,
) -> SearchPage {
    let needle = query.to_lowercase();
    let allowed: Option<HashSet<&str>> = if conversation_ids.is_empty() {
        None
    } else {
        Some(conversation_ids.iter().map(String::as_str).collect())
    };
    let mut hits = Vec::new();
    let mut seen = HashSet::new();

    for shard in &store.shards {
        for table in &shard.tables {
            let Some(conversation_id) = shard.user_by_table.get(table) else {
                continue;
            };
            if let Some(allowed) = &allowed {
                if !allowed.contains(conversation_id.as_str()) {
                    continue;
                }
            }
            let Ok(rows) = shard.message_rows(table) else {
                continue;
            };
            for raw in rows {
                if !raw.raw_content.to_lowercase().contains(&needle) {
                    continue;
                }
                let dedup_key = if raw.server_id > 0 {
                    format!("s:{}", raw.server_id)
                } else {
                    format!(
                        "l:{}:{}:{}:{}",
                        raw.local_id,
                        raw.create_time,
                        raw.sender_id,
                        md5_hex(&raw.raw_content)
                    )
                };
                if !seen.insert(dedup_key) {
                    continue;
                }
                let message = export_message(
                    &raw,
                    self_username,
                    contacts,
                    conversation_id,
                    hits.len() + 1,
                );
                hits.push(SearchHit {
                    conversation_id: conversation_id.clone(),
                    conversation_name: conversation_names
                        .get(conversation_id)
                        .cloned()
                        .unwrap_or_else(|| conversation_id.clone()),
                    message,
                });
                if hits.len() >= 200_000 {
                    break;
                }
            }
        }
    }

    hits.sort_by(|a, b| {
        let ord = a
            .message
            .create_time
            .cmp(&b.message.create_time)
            .then_with(|| a.conversation_id.cmp(&b.conversation_id))
            .then_with(|| a.message.local_id.cmp(&b.message.local_id));
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.message.index = index + 1;
    }

    let total_matches = hits.len();
    let start = offset as usize;
    let end = start.saturating_add(page_size as usize).min(hits.len());
    let page: Vec<SearchHit> = if start < hits.len() {
        hits.drain(start..end).collect()
    } else {
        Vec::new()
    };
    let next_cursor = if end < total_matches {
        Some(crate::util::encode_cursor("search", 0, "", end as u64))
    } else {
        None
    };
    SearchPage {
        hits: page,
        next_cursor,
        total_matches,
    }
}
