use regex::Regex;
use std::sync::{LazyLock, OnceLock};

use super::model::{MediaKind, MediaRef, ParsedContent};

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

fn media_ref(base_type: i64, subtype: Option<i64>, xml: &str) -> Option<MediaRef> {
    let kind = match base_type {
        3 => MediaKind::Image,
        34 => MediaKind::Voice,
        43 => MediaKind::Video,
        47 => MediaKind::Emoji,
        49 if subtype == Some(6) || xml.contains("<appattach") || xml.contains("<filename>") => {
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
        .or_else(|| xml_value(xml, "title"))
        .unwrap_or_default();
    if md5.is_empty() && file_name.is_empty() {
        return None;
    }
    Some(MediaRef {
        md5,
        file_name,
        kind,
    })
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

        let quote = "<msg><appmsg><title>reply</title><type>57</type><refermsg><displayname>Alice</displayname><content>original</content></refermsg></appmsg></msg>";
        let parsed = parse_content(quote, 49, 57, 244813135921);
        assert!(parsed.text.contains("original"));
        assert!(parsed.text.contains("reply"));
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
}
