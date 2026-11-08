use regex::Regex;
use std::sync::{LazyLock, OnceLock};

use super::model::{MediaKind, MediaRef, MessageSource, ParsedContent};

static SENDER_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^([A-Za-z0-9_@\-\.]{2,128}):\r?\n").expect("sender prefix regex")
});

pub fn strip_sender_prefix(content: &str) -> (String, Option<(String, String)>) {
    if let Some(captures) = SENDER_PREFIX.captures(content) {
        if let (Some(sender), Some(whole)) = (captures.get(1), captures.get(0)) {
            let body = content[whole.end()..].to_string();
            return (body, Some((sender.as_str().to_string(), content.to_string())));
        }
    }
    (content.to_string(), None)
}

pub fn normalize_xml(content: &str) -> String {
    let (body, _) = strip_sender_prefix(content);
    let body = body.trim_matches('\0').trim();
    let start = body.find('<');
    let end = body.rfind('>');
    match (start, end) {
        (Some(start), Some(end)) if end > start => body[start..=end].to_string(),
        _ => body.to_string(),
    }
}

pub fn xml_value(xml: &str, tag: &str) -> Option<String> {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let pattern = format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}>");
    let regex = {
        let mut map = cache.lock().ok()?;
        map.entry(tag.to_string())
            .or_insert_with(|| Regex::new(&pattern).expect("xml value regex"))
            .clone()
    };
    let captures = regex.captures(xml)?;
    let raw = captures.get(1)?.as_str();
    let raw = raw
        .replace("<![CDATA[", "")
        .replace("]]>", "");
    let value = unescape_xml(raw.trim());
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let key = format!("{tag}\u{1}{attr}");
    let pattern = format!(r#"(?is)<{tag}\b[^>]*\b{attr}\s*=\s*["']([^"']*)["']"#);
    let regex = {
        let mut map = cache.lock().ok()?;
        map.entry(key)
            .or_insert_with(|| Regex::new(&pattern).expect("xml attr regex"))
            .clone()
    };
    let captures = regex.captures(xml)?;
    Some(unescape_xml(captures.get(1)?.as_str().trim()))
}

pub fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

pub fn strip_tags(value: &str) -> String {
    static TAGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<[^>]*>").expect("tag regex"));
    let text = TAGS.replace_all(value, " ");
    let text = unescape_xml(&text);
    let text = text.replace('\u{00a0}', " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_system(content: &str) -> String {
    let xml = normalize_xml(content);
    if let Some(replace) = xml_value(&xml, "replacemsg") {
        return replace;
    }
    if xml.contains("<revokemsg") || xml.contains("撤回") {
        if let Some(text) = xml_value(&xml, "msg") {
            return text;
        }
        return "撤回了一条消息".to_string();
    }
    let text = strip_tags(&xml);
    if text.is_empty() {
        content.trim().to_string()
    } else {
        text
    }
}

fn app_type(xml: &str, fallback_sub: i64) -> Option<i64> {
    xml_value(xml, "type")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .or(if fallback_sub > 0 { Some(fallback_sub) } else { None })
}

fn type_name_for_app(subtype: Option<i64>, content: &str) -> Option<&'static str> {
    match subtype {
        Some(3) => Some("音乐消息"),
        Some(5) | Some(49) => Some("链接消息"),
        Some(6) => Some("文件消息"),
        Some(19) => Some("聊天记录"),
        Some(33) | Some(36) => Some("小程序消息"),
        Some(57) => Some("引用消息"),
        Some(87) => Some("群公告"),
        Some(2000) => Some("转账消息"),
        _ => {
            if content.contains("<appmsg") || content.contains("<msg>") {
                Some("链接消息")
            } else {
                None
            }
        }
    }
}

pub fn extract_cdn_url(xml: &str) -> Option<String> {
    // Verified against the live CDN and Alpha's own exports:
    //   <emoji cdnurl=...>      -> plaintext original (md5 == message md5, len == declared)
    //   <emoji encrypturl=...>  -> AES-encrypted variant
    //   <emoji externurl=...>   -> AES-encrypted wxgf preview
    //   <img cdnbigimgurl=...>  -> original, cdnmidimgurl -> mid, cdnthumburl -> thumb
    // So the plaintext original always comes first, per message kind.
    let ordered: &[(&str, &[&str])] = &[
        ("emoji", &["cdnurl", "encrypturl", "externurl"]),
        (
            "img",
            &["cdnbigimgurl", "cdnmidimgurl", "cdnthumburl", "cdnurl"],
        ),
        (
            "videomsg",
            &["cdnvideourl", "cdnthumburl", "cdnurl"],
        ),
        (
            "appmsg",
            &["cdnbigimgurl", "cdnmidimgurl", "cdnthumburl", "cdnurl"],
        ),
        (
            "appattach",
            &["cdnattachurl", "cdnthumburl", "cdnurl"],
        ),
    ];

    let normalise = |raw: &str| -> Option<String> {
        let value = raw.replace("&amp;", "&").trim().to_string();
        if value.starts_with("http://") || value.starts_with("https://") {
            Some(value)
        } else {
            None
        }
    };

    for (tag, attrs) in ordered {
        for attr in *attrs {
            if let Some(value) = xml_attr(xml, tag, *attr).and_then(|v| normalise(&v)) {
                return Some(value);
            }
        }
    }
    // Child-element form (<cdnurl>...</cdnurl>) used by some message versions.
    for attr in [
        "cdnurl",
        "cdnbigimgurl",
        "cdnmidimgurl",
        "encrypturl",
        "externurl",
        "cdnthumburl",
        "thumburl",
        "dataurl",
        "url",
    ] {
        if let Some(value) = xml_value(xml, attr).and_then(|v| normalise(&v)) {
            return Some(value);
        }
    }
    None
}
fn first_attr_number(xml: &str, attrs: &[&str]) -> Option<u32> {
    for attr in attrs {
        for tag in ["emoji", "img", "videomsg", "appmsg", "appattach"] {
            if let Some(val) = xml_attr(xml, tag, attr) {
                if let Ok(parsed) = val.trim().parse::<u32>() {
                    if parsed > 0 {
                        return Some(parsed);
                    }
                }
            }
        }
    }
    None
}

fn first_attr_u64(xml: &str, attrs: &[&str]) -> Option<u64> {
    for attr in attrs {
        for tag in ["emoji", "img", "videomsg", "appmsg", "appattach"] {
            if let Some(val) = xml_attr(xml, tag, attr) {
                if let Ok(parsed) = val.trim().parse::<u64>() {
                    if parsed > 0 {
                        return Some(parsed);
                    }
                }
            }
        }
    }
    None
}

pub fn extract_aes_key(xml: &str) -> Option<[u8; 16]> {
    for tag in ["emoji", "img", "appattach", "appmsg"] {
        if let Some(val) = xml_attr(xml, tag, "aeskey") {
            let trimmed = val.trim();
            if trimmed.len() == 32 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Ok(bytes) = hex::decode(trimmed) {
                    if bytes.len() == 16 {
                        let mut key = [0u8; 16];
                        key.copy_from_slice(&bytes);
                        return Some(key);
                    }
                }
            }
        }
    }
    static AESKEY_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"aeskey\s*=\s*"([0-9a-fA-F]{32})""#).unwrap()
    });
    if let Some(caps) = AESKEY_REGEX.captures(xml) {
        if let Some(m) = caps.get(1) {
            if let Ok(bytes) = hex::decode(m.as_str()) {
                if bytes.len() == 16 {
                    let mut key = [0u8; 16];
                    key.copy_from_slice(&bytes);
                    return Some(key);
                }
            }
        }
    }
    None
}

fn media_ref(base_type: i64, subtype: Option<i64>, xml: &str) -> Option<MediaRef> {
    let kind = match base_type {
        3 => MediaKind::Image,
        34 => MediaKind::Voice,
        43 => MediaKind::Video,
        47 => MediaKind::Emoji,
        49 if subtype == Some(6) || (subtype.is_none() && xml_value(xml, "filename").is_some()) => {
            MediaKind::File
        }
        _ => return None,
    };
    let md5 = xml_attr(xml, "img", "md5")
        .or_else(|| xml_attr(xml, "voicemsg", "md5"))
        .or_else(|| xml_attr(xml, "videomsg", "md5"))
        .or_else(|| xml_attr(xml, "videomsg", "rawmd5"))
        .or_else(|| xml_attr(xml, "emoji", "md5"))
        .or_else(|| xml_value(xml, "md5"))
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let file_name = xml_value(xml, "filename")
        .or_else(|| {
            if subtype == Some(6) {
                xml_value(xml, "title")
            } else {
                None
            }
        })
        .unwrap_or_default();
    // Voice payloads are stored in a media database, not by md5/filename, so they
    // must still produce a MediaRef even when both are empty.
    if md5.is_empty() && file_name.is_empty() && base_type != 34 {
        return None;
    }
    let cdn_url = extract_cdn_url(xml);
    let aes_key = extract_aes_key(xml);
    let width = first_attr_number(xml, &["width", "cdnthumbwidth"]);
    let height = first_attr_number(xml, &["height", "cdnthumbheight"]);
    let declared_size = first_attr_u64(
        xml,
        &["len", "length", "androidlen", "hdlength", "totallen"],
    );
    let mut candidate_md5s = Vec::new();
    for attr in ["externmd5", "encryptmd5", "androidmd5", "s60v3md5", "s60v5md5", "rawmd5", "midmd5", "hdmd5"] {
        for tag in ["emoji", "img", "appmsg", "appattach"] {
            if let Some(val) = xml_attr(xml, tag, attr) {
                let lower = val.trim().to_ascii_lowercase();
                if (lower.len() == 32 || lower.len() == 64) && lower.bytes().all(|b| b.is_ascii_hexdigit()) {
                    if !candidate_md5s.contains(&lower) && lower != md5 {
                        candidate_md5s.push(lower);
                    }
                }
            }
        }
    }
    static M_PARAM_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"[?&]m=([0-9a-fA-F]{32})"#).unwrap()
    });
    for caps in M_PARAM_REGEX.captures_iter(xml) {
        if let Some(m) = caps.get(1) {
            let lower = m.as_str().to_ascii_lowercase();
            if !candidate_md5s.contains(&lower) && lower != md5 {
                candidate_md5s.push(lower);
            }
        }
    }
    Some(MediaRef {
        md5,
        candidate_md5s,
        file_name,
        kind,
        cdn_url,
        aes_key,
        width,
        height,
        declared_size,
    })
}

/// Turn WeChat's `<msgsource>` document into short Chinese notes.
/// Cryptographic signatures and empty tags are dropped.
pub fn parse_message_source(raw: &str) -> MessageSource {
    let xml = normalize_xml(raw);
    if !xml.to_ascii_lowercase().contains("msgsource") && !xml.contains('<') {
        return MessageSource::default();
    }
    let mut notes = Vec::new();
    let mut push = |note: String| {
        if !note.is_empty() && !notes.iter().any(|existing| existing == &note) {
            notes.push(note);
        }
    };
    if let Some(value) = xml_value(&xml, "membercount") {
        if let Ok(count) = value.parse::<i64>() {
            if count > 0 {
                push(format!("发送时群里有 {count} 人"));
            }
        }
    }
    if xml_value(&xml, "silence").as_deref() == Some("1") {
        push("群已开启消息免打扰".to_string());
    }
    // alpha_native.dll matches this exact document to identify a message the
    // account owner sent from a phone. Other <fr> values are only device flags.
    let compact = xml.split_whitespace().collect::<String>();
    if compact.contains("<msgsource><alnode><fr>1</fr></alnode></msgsource>") {
        push("本人从手机发送".to_string());
    } else {
        match xml_value(&xml, "fr").as_deref() {
            Some("2") => push("从电脑发送".to_string()),
            Some("3") => push("从平板发送".to_string()),
            _ => {}
        }
    }
    if let Some(scene) = xml_value(&xml, "scene") {
        let label = match scene.as_str() {
            "1" => Some("通过搜索添加"),
            "3" | "14" => Some("通过群聊添加"),
            "17" => Some("通过名片分享"),
            "30" => Some("通过扫一扫添加"),
            _ => None,
        };
        if let Some(label) = label {
            push(label.to_string());
        }
    }
    if xml_value(&xml, "atuserlist").is_some_and(|value| !value.trim().is_empty()) {
        push("消息里 @ 了人".to_string());
    }
    if xml.to_ascii_lowercase().contains("revokemsg") {
        push("这是一条撤回记录".to_string());
    }
    if xml.contains("<bizflag>1") || xml.to_ascii_lowercase().contains("enterprise") {
        push("来自企业微信".to_string());
    }
    MessageSource { notes }
}

fn tag_block<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let start = xml.find(&open)?;
    let end = xml[start..].find(&close)? + start + close.len();
    Some(&xml[start..end])
}

/// Parse the structured quote block so the HTML exporter can link back to the
/// original message and detect when it is outside the exported set.
fn quote_ref(xml: &str) -> Option<crate::wechat::model::QuoteRef> {
    let block = tag_block(xml, "refermsg")?;
    let number = |tag: &str| {
        xml_value(block, tag)
            .and_then(|value| value.trim().parse::<i64>().ok())
    };
    let text = |tag: &str| xml_value(block, tag).unwrap_or_default().trim().to_string();
    let content = strip_tags(&text("content")).trim().to_string();
    let sender = text("displayname");
    let reply = xml_value(xml, "title").unwrap_or_default().trim().to_string();
    let quote = crate::wechat::model::QuoteRef {
        server_id: number("svrid"),
        sender,
        content,
        create_time: number("createtime"),
        quoted_type: number("type"),
        reply,
    };
    if quote.server_id.is_none()
        && quote.sender.is_empty()
        && quote.content.is_empty()
        && quote.reply.is_empty()
        && quote.create_time.is_none()
    {
        None
    } else {
        Some(quote)
    }
}

fn quote_text(xml: &str) -> Option<String> {
    let quoted_sender = xml_value(xml, "displayname").unwrap_or_default();
    let quoted = xml_value(xml, "content").unwrap_or_default();
    let quoted = strip_tags(&quoted);
    let reply = xml_value(xml, "title").unwrap_or_default();
    let mut out = String::new();
    if !quoted_sender.is_empty() || !quoted.is_empty() {
        out.push_str("[引用");
        if !quoted_sender.is_empty() {
            out.push(' ');
            out.push_str(&quoted_sender);
        }
        out.push_str("] ");
        out.push_str(&quoted);
    }
    if !reply.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&reply);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn parse_content(
    raw: &str,
    base_type: i64,
    sub_type: i64,
    full_type: i64,
) -> ParsedContent {
    let (body, _) = strip_sender_prefix(raw);
    let body = body.trim_matches('\0').trim().to_string();
    let xml = normalize_xml(&body);
    let app_subtype = app_type(&xml, sub_type);

    if full_type == 266_287_972_401 {
        let text = clean_system(&body);
        return ParsedContent {
            text: if text.is_empty() {
                "[拍一拍]".to_string()
            } else {
                format!("[拍一拍] {text}")
            },
            type_name: "其他消息".to_string(),
            app_subtype: Some(62),
            media: None,
            quote: None,
        };
    }

    let (text, media, is_system) = match base_type {
        1 => (body, None, false),
        3 => (
            "[图片]".to_string(),
            media_ref(base_type, app_subtype, &xml),
            false,
        ),
        34 => ("[语音消息]".to_string(), media_ref(base_type, app_subtype, &xml), false),
        37 => ("[好友请求]".to_string(), None, false),
        42 => {
            let nickname = xml_attr(&xml, "msg", "nickname")
                .or_else(|| xml_attr(&xml, "msg", "displayname"))
                .or_else(|| xml_value(&xml, "nickname"))
                .unwrap_or_default();
            (
                if nickname.is_empty() {
                    "[名片]".to_string()
                } else {
                    format!("[名片] {nickname}")
                },
                None,
                false,
            )
        }
        43 => {
            let length = xml_value(&xml, "playlength")
                .or_else(|| xml_value(&xml, "length"))
                .unwrap_or_default();
            let text = if length.is_empty() {
                "[视频]".to_string()
            } else {
                format!("[视频] {length}s")
            };
            (text, media_ref(base_type, app_subtype, &xml), false)
        }
        47 => (
            "[动画表情]".to_string(),
            media_ref(base_type, app_subtype, &xml),
            false,
        ),
        48 => {
            let poi = xml_attr(&xml, "location", "poiname")
                .or_else(|| xml_value(&xml, "poiname"))
                .unwrap_or_default();
            let label = xml_attr(&xml, "location", "label")
                .or_else(|| xml_value(&xml, "label"))
                .unwrap_or_default();
            let x = xml_attr(&xml, "location", "x")
                .or_else(|| xml_attr(&xml, "location", "latitude"))
                .unwrap_or_default();
            let y = xml_attr(&xml, "location", "y")
                .or_else(|| xml_attr(&xml, "location", "longitude"))
                .unwrap_or_default();
            let mut parts = Vec::new();
            if !poi.is_empty() {
                parts.push(poi);
            }
            if !label.is_empty() && !parts.contains(&label) {
                parts.push(label);
            }
            if !x.is_empty() && !y.is_empty() {
                parts.push(format!("({x},{y})"));
            }
            let text = if parts.is_empty() {
                "[位置]".to_string()
            } else {
                format!("[位置] {}", parts.join(" "))
            };
            (text, None, false)
        }
        49 => {
            let title = xml_value(&xml, "title").unwrap_or_default();
            let url = xml_value(&xml, "url").unwrap_or_default();
            let text = match app_subtype {
                Some(3) => {
                    let song = xml_value(&xml, "songname")
                        .filter(|value| !value.is_empty())
                        .unwrap_or(title);
                    format!("[音乐] {song}")
                }
                Some(6) => {
                    let file = xml_value(&xml, "filename").unwrap_or(title);
                    format!("[文件] {file}")
                }
                Some(19) => {
                    if title.is_empty() {
                        "[聊天记录]".to_string()
                    } else {
                        format!("[聊天记录] {title}")
                    }
                }
                Some(33) | Some(36) => {
                    if title.is_empty() {
                        "[小程序]".to_string()
                    } else {
                        format!("[小程序] {title}")
                    }
                }
                Some(57) => quote_text(&xml)
                    .or_else(|| if title.is_empty() { None } else { Some(format!("[引用] {title}")) })
                    .unwrap_or_else(|| "[引用消息]".to_string()),
                Some(87) => {
                    let announcement = xml_value(&xml, "textannouncement").unwrap_or(title);
                    if announcement.is_empty() {
                        "[群公告]".to_string()
                    } else {
                        format!("[群公告] {announcement}")
                    }
                }
                Some(2000) => {
                    let feedesc = xml_value(&xml, "feedesc").unwrap_or_default();
                    let memo = xml_value(&xml, "pay_memo").unwrap_or_default();
                    let mut value = "[转账]".to_string();
                    if !feedesc.is_empty() {
                        value.push(' ');
                        value.push_str(&feedesc);
                    }
                    if !memo.is_empty() {
                        value.push(' ');
                        value.push_str(&memo);
                    }
                    value
                }
                Some(51) | Some(63) | Some(115) => {
                    if title.is_empty() {
                        "[视频号]".to_string()
                    } else {
                        format!("[视频号] {title}")
                    }
                }
                _ => {
                    if title.is_empty() {
                        if url.is_empty() {
                            "[链接]".to_string()
                        } else {
                            format!("[链接] {url}")
                        }
                    } else if url.is_empty() {
                        format!("[链接] {title}")
                    } else {
                        format!("[链接] {title} {url}")
                    }
                }
            };
            (text, media_ref(base_type, app_subtype, &xml), false)
        }
        50 => {
            let duration = xml_value(&xml, "duration")
                .or_else(|| xml_value(&xml, "invitetime"))
                .unwrap_or_default();
            let text = if duration.is_empty() {
                "[通话]".to_string()
            } else {
                format!("[通话] {duration}s")
            };
            (text, None, false)
        }
        10000 | 10002 => (clean_system(&body), None, true),
        _ if raw.trim_start().starts_with("<sysmsg")
            || raw.contains("revokemsg")
            || raw.contains("sysmsg") =>
        {
            (clean_system(&body), None, true)
        }
        _ => {
            if xml.contains("<appmsg") || xml.contains("<msg>") {
                let title = xml_value(&xml, "title").unwrap_or_default();
                let text = if title.is_empty() {
                    strip_tags(&xml)
                } else {
                    title
                };
                (text, media_ref(base_type, app_subtype, &xml), false)
            } else if body.is_empty() {
                ("[其他消息]".to_string(), None, false)
            } else {
                (body, None, false)
            }
        }
    };

    let type_name = if is_system {
        "系统消息".to_string()
    } else if base_type == 49 {
        type_name_for_app(app_subtype, &xml)
            .unwrap_or("链接消息")
            .to_string()
    } else {
        match base_type {
            1 => "文本消息",
            3 => "图片消息",
            34 => "语音消息",
            37 => "好友请求",
            42 => "名片消息",
            43 => "视频消息",
            47 => "动画表情",
            48 => "位置消息",
            49 => "链接消息",
            50 => "通话消息",
            10000 | 10002 => "系统消息",
            _ => "其他消息",
        }
        .to_string()
    };

    ParsedContent {
        text,
        type_name,
        app_subtype,
        media,
        quote: quote_ref(&xml),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_group_sender_prefix() {
        let (body, sender) = strip_sender_prefix("wxid_alice:\nhello");
        assert_eq!(body, "hello");
        assert_eq!(sender.unwrap().0, "wxid_alice");
    }

    #[test]
    fn parses_link_and_quote() {
        let link = "<msg><appmsg><title>Example</title><type>5</type><url>https://e.test</url></appmsg></msg>";
        let parsed = parse_content(link, 49, 0, 49);
        assert_eq!(parsed.type_name, "链接消息");
        assert!(parsed.text.contains("Example"));
        assert!(parsed.text.contains("https://e.test"));
        assert!(parsed.media.is_none());

        let quote = "<msg><appmsg><title>reply</title><type>57</type><refermsg><displayname>Alice</displayname><content>original</content></refermsg><appattach><totallen>0</totallen></appattach></appmsg></msg>";
        let parsed = parse_content(quote, 49, 57, 244813135921);
        assert!(parsed.text.contains("original"));
        assert!(parsed.text.contains("reply"));
        assert!(parsed.media.is_none());
    }
    #[test]
    fn parses_system_message() {
        let parsed = parse_content(
            "<sysmsg type=\"revokemsg\"><revokemsg><replacemsg>Alice 撤回了一条消息</replacemsg></revokemsg></sysmsg>",
            10000,
            0,
            10000,
        );
        assert_eq!(parsed.type_name, "系统消息");
        assert_eq!(parsed.text, "Alice 撤回了一条消息");
    }

    #[test]
    fn decodes_zstd_payload() {
        let compressed = [
            0x28, 0xB5, 0x2F, 0xFD, 0x04, 0x58, 0x81, 0x00, 0x00, 0x63, 0x6F, 0x6D, 0x70, 0x72, 0x65,
            0x73, 0x73, 0x65, 0x64, 0x20, 0x68, 0x65, 0x6C, 0x6C, 0x6F, 0xEF, 0xC0, 0x7C, 0xA3,
        ];
        let decoded = super::super::shards::decompress_payload(compressed.to_vec(), 4);
        assert_eq!(String::from_utf8(decoded).unwrap(), "compressed hello");
    }

    #[test]
    fn prefers_plaintext_original_per_message_kind() {
        let emoji = r#"<msg><emoji md5 = "a977c9bbc0cc67c2fd4f601f5219f16e" cdnurl = "http://c/1.jpg" encrypturl = "http://c/2.jpg" externurl = "http://c/3.jpg"/></msg>"#;
        assert_eq!(extract_cdn_url(emoji).as_deref(), Some("http://c/1.jpg"));

        let image = r#"<msg><img cdnthumburl="http://c/t.jpg" cdnmidimgurl="http://c/m.jpg" cdnbigimgurl="http://c/b.jpg"/></msg>"#;
        assert_eq!(extract_cdn_url(image).as_deref(), Some("http://c/b.jpg"));

        let attachment = r#"<msg><appmsg><appattach><cdnattachurl>3057aa</cdnattachurl></appattach></appmsg></msg>"#;
        assert_eq!(extract_cdn_url(attachment), None);
    }

    #[test]
    fn parses_structured_quote_metadata() {
        let xml = "<msg><appmsg><title>reply</title><type>57</type><refermsg><type>1</type><svrid>9003</svrid><displayname>Alice</displayname><content>original text</content><createtime>1703001620</createtime></refermsg></appmsg></msg>";
        let parsed = parse_content(xml, 49, 57, 244813135921);
        let quote = parsed.quote.expect("quote metadata");
        assert_eq!(quote.server_id, Some(9003));
        assert_eq!(quote.sender, "Alice");
        assert_eq!(quote.content, "original text");
        assert_eq!(quote.create_time, Some(1703001620));
        assert_eq!(quote.quoted_type, Some(1));
        assert_eq!(quote.reply, "reply");
    }

    #[test]
    fn prefers_big_image_url_and_reads_declared_length() {
        let xml = r#"<msg><img aeskey="00112233445566778899aabbccddeeff" cdnthumburl="http://t/t.jpg" cdnthumbwidth="120" cdnthumbheight="90" cdnmidimgurl="http://t/m.jpg" length="198488" cdnbigimgurl="http://t/big.jpg" hdlength="300000" md5="3d7399f3e4e2ad4855318105efd62c62"/></msg>"#;
        assert_eq!(extract_cdn_url(xml).as_deref(), Some("http://t/big.jpg"));
        let parsed = media_ref(3, None, xml).expect("image media ref");
        assert_eq!(parsed.declared_size, Some(198488));
        assert_eq!(parsed.width, Some(120));
        assert_eq!(parsed.height, Some(90));
    }

    #[test]
    fn translates_msgsource_into_notes() {
        let owner = "<msgsource><alnode><fr>1</fr></alnode></msgsource>";
        assert_eq!(
            parse_message_source(owner).notes,
            vec!["本人从手机发送".to_string()]
        );
        let raw = "<msgsource><silence>1</silence><membercount>4</membercount><signature>v1_secret</signature><sec_msg_node><alnode><fr>2</fr></alnode></sec_msg_node></msgsource>";
        let parsed = parse_message_source(raw);
        assert_eq!(
            parsed.notes,
            vec![
                "发送时群里有 4 人".to_string(),
                "群已开启消息免打扰".to_string(),
                "从电脑发送".to_string(),
            ]
        );
        let binary = parse_message_source("\u{1}\u{2}not xml");
        assert!(binary.notes.is_empty());
    }
}
