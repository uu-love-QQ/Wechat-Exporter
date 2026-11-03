use std::collections::HashMap;

use super::discover::Layout;
use super::model::{Contact, Conversation, ConversationJson, SessionRow};
use super::shards::Store;
use crate::util::{is_group_id, sanitize_file_name};

pub fn build_conversations(
    _layout: &Layout,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    sessions: &[SessionRow],
) -> Vec<Conversation> {
    let mut map: HashMap<String, Conversation> = HashMap::new();
    for session in sessions {
        let contact = contacts.get(&session.username);
        let is_group = is_group_id(&session.username)
            || contact.map(|contact| contact.is_group).unwrap_or(false);
        let name = contact
            .map(|contact| contact.display_name())
            .unwrap_or_else(|| session.username.clone());
        map.insert(
            session.username.clone(),
            Conversation {
                id: session.username.clone(),
                name,
                nickname: contact.map(|c| c.nickname.clone()).unwrap_or_default(),
                remark: contact.map(|c| c.remark.clone()).unwrap_or_default(),
                alias: contact.map(|c| c.alias.clone()).unwrap_or_default(),
                is_group,
                unread_count: session.unread_count,
                sort_timestamp: session.sort_timestamp,
                last_timestamp: session.last_timestamp,
                message_count: store.message_count(&session.username),
            },
        );
    }

    for id in store.conversation_ids() {
        if map.contains_key(&id) {
            continue;
        }
        let contact = contacts.get(&id);
        let is_group = is_group_id(&id) || contact.map(|contact| contact.is_group).unwrap_or(false);
        let name = contact
            .map(|contact| contact.display_name())
            .unwrap_or_else(|| id.clone());
        let last = store.latest_time(&id);
        map.insert(
            id.clone(),
            Conversation {
                id: id.clone(),
                name,
                nickname: contact.map(|c| c.nickname.clone()).unwrap_or_default(),
                remark: contact.map(|c| c.remark.clone()).unwrap_or_default(),
                alias: contact.map(|c| c.alias.clone()).unwrap_or_default(),
                is_group,
                unread_count: 0,
                sort_timestamp: last,
                last_timestamp: last,
                message_count: store.message_count(&id),
            },
        );
    }

    let mut out: Vec<Conversation> = map.into_values().collect();
    for conversation in &mut out {
        if conversation.last_timestamp == 0 {
            conversation.last_timestamp = store.latest_time(&conversation.id);
        }
        if conversation.sort_timestamp == 0 {
            conversation.sort_timestamp = conversation.last_timestamp;
        }
    }
    out.sort_by(|a, b| {
        b.sort_timestamp
            .cmp(&a.sort_timestamp)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

impl Conversation {
    pub fn to_json(&self) -> ConversationJson {
        ConversationJson {
            id: self.id.clone(),
            name: self.name.clone(),
            nickname: self.nickname.clone(),
            remark: self.remark.clone(),
            alias: self.alias.clone(),
            kind: if self.is_group { "group" } else { "private" }.to_string(),
            unread_count: self.unread_count,
            is_pinned: false,
            sort_timestamp: self.sort_timestamp,
            last_timestamp: self.last_timestamp,
            message_count: self.message_count,
        }
    }

    pub fn folder_name(&self) -> String {
        let base = sanitize_file_name(&self.name);
        let suffix = crate::util::md5_hex(&self.id);
        format!("{base}__{}", &suffix[..8])
    }
}

pub fn select_conversations(
    all: &[Conversation],
    requested: &[String],
) -> anyhow::Result<Vec<Conversation>> {
    if requested.is_empty() {
        return Ok(all.to_vec());
    }
    let mut out = Vec::new();
    for id in requested {
        if let Some(found) = all.iter().find(|conversation| &conversation.id == id) {
            out.push(found.clone());
        } else if let Some(found) = all.iter().find(|conversation| {
            conversation.name == *id || conversation.alias == *id || conversation.nickname == *id
        }) {
            out.push(found.clone());
        } else {
            anyhow::bail!("explicit conversation did not return readable messages; no fallback was attempted");
        }
    }
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}
