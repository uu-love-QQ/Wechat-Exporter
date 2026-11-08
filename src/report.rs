use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local, TimeZone, Timelike};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::{ReportKind, ValidatedCli};
use crate::export::{self, ExportContext};
use crate::util::{format_timestamp, GENERATOR};
use crate::wechat::model::{Contact, Conversation, ExportMessage, MediaKind, MediaRef, MessageSource};
use crate::wechat::shards::Store;

const PRIVATE_ID_EXCLUDES: &[&str] = &[
    "filehelper",
    "qqmail",
    "fmessage",
    "medianote",
    "floatbottle",
    "newsapp",
    "brandsessionholder",
    "brandservicesessionholder",
    "notifymessage",
    "opencustomerservicemsg",
    "notification_messages",
    "userexperience_alarm",
];

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn is_private_conversation(conversation: &Conversation) -> bool {
    let id = conversation.id.as_str();
    if conversation.is_group || id.ends_with("@chatroom") || id.contains('@') {
        return false;
    }
    if id.starts_with("gh_") {
        return false;
    }
    !PRIVATE_ID_EXCLUDES.iter().any(|excluded| id == *excluded)
}

fn is_analyzable_conversation(conversation: &Conversation) -> bool {
    !conversation.id.starts_with("gh_")
        && conversation.id != "filehelper"
        && !conversation.id.starts_with("qqmail")
        && !conversation.id.starts_with("fmessage")
}

fn year_bounds(year: i32) -> (Option<i64>, Option<i64>) {
    if year == 0 {
        return (None, None);
    }
    let start = Local
        .with_ymd_and_hms(year, 1, 1, 0, 0, 0)
        .single()
        .map(|date| date.timestamp_millis());
    let end = Local
        .with_ymd_and_hms(year + 1, 1, 1, 0, 0, 0)
        .single()
        .map(|date| date.timestamp_millis() - 1);
    (start, end)
}

fn load_messages(
    store: &Store,
    conversation: &Conversation,
    contacts: &HashMap<String, Contact>,
    self_username: Option<&str>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<ExportMessage>> {
    store
        .load_messages(&conversation.id, self_username, contacts, start_ms, end_ms)
        .with_context(|| format!("failed to load messages for {}", conversation.id))
}

#[derive(Default)]
struct FriendAgg {
    id: String,
    name: String,
    total: usize,
    sent: usize,
    received: usize,
    last: i64,
}

#[derive(Default)]
struct MemberAgg {
    id: String,
    name: String,
    total: usize,
    media: usize,
    chars: usize,
    last: i64,
}

#[derive(Default)]
struct Aggregate {
    total: usize,
    sent: usize,
    received: usize,
    chars: usize,
    hours_me: [usize; 24],
    hours_other: [usize; 24],
    hours_all: [usize; 24],
    days: BTreeMap<String, (usize, usize)>,
    months: BTreeMap<String, (usize, usize)>,
    types: BTreeMap<String, usize>,
    media: BTreeMap<&'static str, usize>,
    missing_media: usize,
    friends: BTreeMap<String, FriendAgg>,
    groups: BTreeMap<String, FriendAgg>,
    members: BTreeMap<String, MemberAgg>,
    first: i64,
    last: i64,
}

impl Aggregate {
    fn new() -> Self {
        Self {
            first: i64::MAX,
            last: i64::MIN,
            ..Self::default()
        }
    }

    fn observe(&mut self, message: &ExportMessage) {
        self.total += 1;
        if message.is_send {
            self.sent += 1;
        } else {
            self.received += 1;
        }
        self.chars += message.content.chars().count();
        self.first = self.first.min(message.create_time);
        self.last = self.last.max(message.create_time);
        let type_name = if message.type_name.trim().is_empty() {
            "其他消息".to_string()
        } else {
            message.type_name.clone()
        };
        *self.types.entry(type_name).or_default() += 1;
        if let Some(media) = &message.media {
            *self.media.entry(media.kind.label()).or_default() += 1;
            if message.media_path.is_none() {
                self.missing_media += 1;
            }
        }
        if let Some(date) = Local.timestamp_opt(message.create_time, 0).single() {
            let day = date.format("%Y-%m-%d").to_string();
            let month = date.format("%Y-%m").to_string();
            let hour = date.hour() as usize;
            let bucket = self.days.entry(day).or_insert((0, 0));
            let month_bucket = self.months.entry(month).or_insert((0, 0));
            if message.is_send {
                bucket.0 += 1;
                month_bucket.0 += 1;
                self.hours_me[hour] += 1;
            } else {
                bucket.1 += 1;
                month_bucket.1 += 1;
                self.hours_other[hour] += 1;
            }
            self.hours_all[hour] += 1;
        }
    }

    fn peak(&self) -> (String, usize) {
        self.days
            .iter()
            .map(|(day, (me, other))| (day.clone(), me + other))
            .max_by_key(|(_, count)| *count)
            .unwrap_or_else(|| ("-".to_string(), 0))
    }

    fn active_days(&self) -> usize {
        self.days.len()
    }

    fn span_days(&self) -> usize {
        if self.total == 0 {
            return 0;
        }
        if let (Some(start), Some(end)) = (
            Local.timestamp_opt(self.first, 0).single(),
            Local.timestamp_opt(self.last, 0).single(),
        ) {
            (end.date_naive() - start.date_naive()).num_days().max(0) as usize + 1
        } else {
            self.active_days()
        }
    }
}

fn private_report(
    options: &ValidatedCli,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
) -> Result<(String, Aggregate)> {
    let mut aggregate = Aggregate::new();
    for conversation in conversations.iter().filter(|item| is_private_conversation(item)) {
        let messages = load_messages(store, conversation, contacts, self_username, None, None)?;
        if messages.is_empty() {
            continue;
        }
        let sent = messages.iter().filter(|message| message.is_send).count();
        let last = messages.iter().map(|message| message.create_time).max().unwrap_or(0);
        for message in &messages {
            aggregate.observe(message);
        }
        let entry = aggregate
            .friends
            .entry(conversation.id.clone())
            .or_insert_with(|| FriendAgg {
                id: conversation.id.clone(),
                name: conversation.name.clone(),
                ..FriendAgg::default()
            });
        entry.name = conversation.name.clone();
        entry.total += messages.len();
        entry.sent += sent;
        entry.received += messages.len() - sent;
        entry.last = entry.last.max(last);
    }
    if aggregate.total == 0 {
        bail!("no readable private conversation messages");
    }
    let _ = options;
    Ok((render_private(&aggregate), aggregate))
}

fn group_report(
    conversation: &Conversation,
    messages: &[ExportMessage],
    account_id: Option<&str>,
    member_ids: &[String],
    contacts: &HashMap<String, Contact>,
) -> Result<(String, Aggregate)> {
    let mut aggregate = Aggregate::new();
    for member_id in member_ids {
        let name = contacts
            .get(member_id)
            .map(|contact| contact.display_name())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| member_id.clone());
        aggregate.members.entry(member_id.clone()).or_insert_with(|| MemberAgg {
            id: member_id.clone(),
            name,
            ..MemberAgg::default()
        });
    }
    for message in messages {
        aggregate.observe(message);
        let key = if message.sender_id.trim().is_empty() {
            message.sender_name.clone()
        } else {
            message.sender_id.clone()
        };
        let name = if message.sender_name.trim().is_empty() {
            if message.is_send {
                "我".to_string()
            } else {
                key.clone()
            }
        } else {
            message.sender_name.clone()
        };
        let entry = aggregate.members.entry(key.clone()).or_insert_with(|| MemberAgg {
            id: key.clone(),
            name: name.clone(),
            ..MemberAgg::default()
        });
        entry.name = name;
        entry.total += 1;
        entry.chars += message.content.chars().count();
        if message.media.is_some() {
            entry.media += 1;
        }
        entry.last = entry.last.max(message.create_time);
    }
    Ok((render_group(conversation, &aggregate, account_id), aggregate))
}

fn annual_report(
    year: i32,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
) -> Result<(String, Aggregate)> {
    let (start, end) = year_bounds(year);
    let mut aggregate = Aggregate::new();
    for conversation in conversations.iter().filter(|item| is_analyzable_conversation(item)) {
        let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
        if messages.is_empty() {
            continue;
        }
        let sent = messages.iter().filter(|message| message.is_send).count();
        let last = messages.iter().map(|message| message.create_time).max().unwrap_or(0);
        for message in &messages {
            aggregate.observe(message);
        }
        let map = if conversation.is_group {
            &mut aggregate.groups
        } else {
            &mut aggregate.friends
        };
        let entry = map.entry(conversation.id.clone()).or_insert_with(|| FriendAgg {
            id: conversation.id.clone(),
            name: conversation.name.clone(),
            ..FriendAgg::default()
        });
        entry.name = conversation.name.clone();
        entry.total += messages.len();
        entry.sent += sent;
        entry.received += messages.len() - sent;
        entry.last = entry.last.max(last);
    }
    if aggregate.total == 0 {
        bail!("no messages available for the selected year");
    }
    Ok((render_annual(year, &aggregate), aggregate))
}

fn render_page(
    title: &str,
    eyebrow: &str,
    hero_title: &str,
    hero_sub: &str,
    hero_range: &str,
    hero_meta: &str,
    metrics: &[(String, String, String)],
    body: &str,
) -> String {
    use std::fmt::Write as _;
    let mut html = String::with_capacity(24_000);
    html.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>");
    html.push_str(&export::escape_html(title));
    html.push_str("</title><style>");
    html.push_str(export::REPORT_CSS);
    html.push_str("</style></head><body><main>");
    write!(
        &mut html,
        "<header class=\"hero\"><div class=\"eyebrow\">{}</div><h1>{}</h1><div class=\"hero-sub\">{}</div><div class=\"hero-range\">{}</div><div class=\"hero-meta\">{}</div></header>",
        export::escape_html(eyebrow),
        export::escape_html(hero_title),
        export::escape_html(hero_sub),
        export::escape_html(hero_range),
        export::escape_html(hero_meta)
    ).ok();
    html.push_str("<section class=\"kpis\" aria-label=\"核心指标\">");
    for (label, value, note) in metrics {
        write!(
            &mut html,
            "<div class=\"kpi\"><span>{}</span><strong>{}</strong><small>{}</small></div>",
            export::escape_html(label),
            export::escape_html(value),
            export::escape_html(note)
        ).ok();
    }
    html.push_str("</section>");
    html.push_str(body);
    write!(
        &mut html,
        "<footer>由 {} 生成 · 静态离线报告</footer></main></body></html>",
        export::escape_html(GENERATOR)
    ).ok();
    html
}

fn section(title: &str, badge: &str, body: &str) -> String {
    use std::fmt::Write as _;
    let mut html = String::new();
    write!(
        &mut html,
        "<section class=\"card\"><div class=\"section-head\"><h2>{}</h2><span class=\"badge\">{}</span></div>{}",
        export::escape_html(title),
        export::escape_html(badge),
        body
    ).ok();
    html.push_str("</section>");
    html
}

fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    use std::fmt::Write as _;
    let mut html = String::from("<div class=\"table-wrap\"><table><thead><tr>");
    for header in headers {
        write!(&mut html, "<th>{}</th>", export::escape_html(header)).ok();
    }
    html.push_str("</tr></thead><tbody>");
    for row in rows {
        html.push_str("<tr>");
        for value in row {
            write!(&mut html, "<td>{}</td>", export::escape_html(value)).ok();
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table></div>");
    html
}

fn kpis_for(aggregate: &Aggregate, kind: &str) -> Vec<(String, String, String)> {
    let peak = aggregate.peak();
    let avg = if aggregate.total == 0 {
        0.0
    } else {
        aggregate.chars as f64 / aggregate.total as f64
    };
    let mut metrics = Vec::new();
    match kind {
        "private" => {
            metrics.push(("好友数".into(), export::format_count(aggregate.friends.len()), "有消息记录".into()));
        }
        "group" => {
            metrics.push(("群成员数".into(), export::format_count(aggregate.members.len()), "含未发言成员".into()));
        }
        "annual" => {
            metrics.push(("好友数".into(), export::format_count(aggregate.friends.len()), "有消息记录".into()));
            metrics.push(("群聊数".into(), export::format_count(aggregate.groups.len()), "有消息记录".into()));
        }
        _ => {}
    }
    metrics.push(("消息总数".into(), export::format_count(aggregate.total), "含系统与媒体消息".into()));
    metrics.push(("总字数".into(), export::format_count(aggregate.chars), format!("平均 {avg:.1} 字/条")));
    metrics.push(("活跃天数".into(), export::format_count(aggregate.active_days()), format!("跨度 {} 天", aggregate.span_days())));
    metrics.push(("单日最高".into(), export::format_count(peak.1), peak.0));
    metrics
}

fn type_rows(types: &BTreeMap<String, usize>, total: usize) -> String {
    use std::fmt::Write as _;
    let mut rows: Vec<(&String, &usize)> = types.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows.truncate(12);
    let max = rows.first().map(|(_, count)| **count).unwrap_or(1).max(1);
    let mut html = String::from("<div class=\"rows\">");
    for (name, count) in &rows {
        write!(
            &mut html,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill teal\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · {}</span></div>",
            export::escape_html(name),
            **count as f64 * 100.0 / max as f64,
            export::format_count(**count),
            export::percent(**count, total)
        ).ok();
    }
    html.push_str("</div>");
    html
}

fn hour_rows(hours_me: &[usize; 24], hours_other: &[usize; 24], title: &str, split: bool) -> String {
    use std::fmt::Write as _;
    let max = if split {
        hours_me.iter().chain(hours_other.iter()).copied().max().unwrap_or(1).max(1)
    } else {
        hours_me.iter().copied().max().unwrap_or(1).max(1)
    };
    let mut html = String::from("<div class=\"grouped\" role=\"img\" aria-label=\"24 小时消息分布\">");
    for hour in 0..24 {
        if split {
            let title = format!("{hour:02}:00 · 我 {} 条 · 其他人 {} 条", hours_me[hour], hours_other[hour]);
            html.push_str(&export::bar_group(&format!("{hour:02}"), hours_me[hour], hours_other[hour], max, &title));
        } else {
            let title = format!("{hour:02}:00 · {} 条", hours_me[hour]);
            html.push_str(&export::bar_group(&format!("{hour:02}"), hours_me[hour], 0, max, &title));
        }
    }
    html.push_str("</div><div class=\"chart-caption\"><span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>23:00</span></div>");
    if split {
        write!(
            &mut html,
            "<div class=\"legend\"><span><i class=\"dot\"></i>我</span><span><i class=\"dot other\"></i>其他人</span><span>{title} · 峰值 {max} 条</span></div>"
        ).ok();
    } else {
        write!(
            &mut html,
            "<div class=\"legend\"><span><i class=\"dot\"></i>消息数</span><span>{title} · 峰值 {max} 条</span></div>"
        ).ok();
    }
    html
}

fn month_bars(months: &BTreeMap<String, (usize, usize)>, limit: usize) -> String {
    let mut rows: Vec<(&String, &(usize, usize))> = months.iter().collect();
    if rows.len() > limit {
        rows = rows[rows.len() - limit..].to_vec();
    }
    let max = rows
        .iter()
        .map(|(_, (me, other))| me + other)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut html = String::from("<div class=\"grouped\" role=\"img\" aria-label=\"月度消息趋势\">");
    for (month, (me, other)) in &rows {
        let label = month.get(2..).unwrap_or(month);
        let title = format!("{} · 我 {} 条 · 其他人 {} 条", month, me, other);
        html.push_str(&export::bar_group(label, *me, *other, max, &title));
    }
    html.push_str("</div><div class=\"chart-caption\"><span>");
    if let Some((first, _)) = rows.first() {
        html.push_str(&export::escape_html(first));
    }
    html.push_str("</span><span>");
    if let Some((last, _)) = rows.last() {
        html.push_str(&export::escape_html(last));
    }
    html.push_str("</span></div><div class=\"legend\"><span><i class=\"dot\"></i>我</span><span><i class=\"dot other\"></i>其他人</span></div>");
    html
}

fn top_day_rows(days: &BTreeMap<String, (usize, usize)>) -> String {
    use std::fmt::Write as _;
    let mut rows: Vec<(&String, usize)> = days
        .iter()
        .map(|(day, (me, other))| (day, me + other))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    rows.truncate(10);
    let max = rows.first().map(|(_, count)| *count).unwrap_or(1).max(1);
    let mut html = String::from("<div class=\"rows\">");
    for (day, count) in rows {
        write!(
            &mut html,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} 条</span></div>",
            export::escape_html(day),
            count as f64 * 100.0 / max as f64,
            export::format_count(count)
        ).ok();
    }
    html.push_str("</div>");
    html
}

fn render_private(aggregate: &Aggregate) -> String {
    use std::fmt::Write as _;
    let peak = aggregate.peak();
    let mut body = String::new();
    write!(
        &mut body,
        "<section class=\"card\"><div class=\"section-head\"><h2>我的发送比例</h2><span class=\"badge\">共 {} 条</span></div><p class=\"pk-summary\">我发了 <b class=\"me\">{}</b> 条，好友发了 <b class=\"other\">{}</b> 条。</p><div class=\"pk-bar\" role=\"img\" aria-label=\"我 {}%，好友 {}%\"><div class=\"pk-sent\" style=\"width:{:.1}%\">我 {:.0}%</div><div class=\"pk-recv\" style=\"width:{:.1}%\">好友 {:.0}%</div></div></section>",
        export::format_count(aggregate.total),
        export::format_count(aggregate.sent),
        export::format_count(aggregate.received),
        export::percent(aggregate.sent, aggregate.total),
        export::percent(aggregate.received, aggregate.total),
        aggregate.sent as f64 * 100.0 / aggregate.total.max(1) as f64,
        aggregate.sent as f64 * 100.0 / aggregate.total.max(1) as f64,
        aggregate.received as f64 * 100.0 / aggregate.total.max(1) as f64,
        aggregate.received as f64 * 100.0 / aggregate.total.max(1) as f64,
    ).ok();

    let mut friend_rows: Vec<(&FriendAgg, usize)> = aggregate.friends.values().map(|friend| (friend, friend.total)).collect();
    friend_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    let max_friend = friend_rows.first().map(|(_, count)| *count).unwrap_or(1).max(1);
    let mut ranking = String::from("<div class=\"rows\">");
    for (friend, count) in friend_rows.iter().take(20) {
        write!(
            &mut ranking,
            "<div class=\"row\"><span class=\"name\" title=\"{}\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · 我{} / 对方{}</span></div>",
            export::escape_html(&friend.id),
            export::escape_html(&friend.name),
            *count as f64 * 100.0 / max_friend as f64,
            export::format_count(*count),
            export::format_count(friend.sent),
            export::format_count(friend.received)
        ).ok();
    }
    ranking.push_str("</div>");
    body.push_str(&section("好友消息排行", "TOP 20", &ranking));

    let friend_table: Vec<Vec<String>> = friend_rows
        .iter()
        .take(50)
        .map(|(friend, _)| {
            vec![
                friend.name.clone(),
                friend.id.clone(),
                export::format_count(friend.total),
                export::format_count(friend.sent),
                export::format_count(friend.received),
                if friend.last > 0 { format_timestamp(friend.last) } else { "-".to_string() },
            ]
        })
        .collect();
    body.push_str(&section(
        "好友明细",
        &format!("{} 位", aggregate.friends.len()),
        &table(&["好友", "账号", "消息", "我发", "对方发", "最后活跃"], &friend_table),
    ));

    let type_body = format!(
        "{}<div class=\"legend\"><span><i class=\"dot\"></i>文本/媒体/系统消息</span></div>",
        type_rows(&aggregate.types, aggregate.total)
    );
    body.push_str(&section("消息类型占比", "全部私聊", &type_body));

    let hours = hour_rows(&aggregate.hours_me, &aggregate.hours_other, "一天中的分布", true);
    body.push_str(&section("消息时段分布", "24 小时", &hours));

    let months = month_bars(&aggregate.months, 36);
    body.push_str(&section("月度趋势", "最近 36 个月", &months));

    let mut media = String::from("<div class=\"rows\">");
    for (label, count) in &aggregate.media {
        write!(
            &mut media,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill teal\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} 个</span></div>",
            export::escape_html(label),
            *count as f64 * 100.0 / aggregate.total.max(1) as f64,
            count
        ).ok();
    }
    if aggregate.missing_media > 0 {
        write!(&mut media, "<p class=\"chart-note\">其中 {} 个媒体未在本地索引到。</p>", aggregate.missing_media).ok();
    }
    media.push_str("</div>");
    body.push_str(&section("媒体内容", "图片 / 语音 / 视频 / 表情 / 文件", &media));

    let metrics = kpis_for(aggregate, "private");
    render_page(
        "私聊分析报告",
        "PRIVATE CHAT ANALYTICS · 私聊分析",
        "私聊分析 · 好友消息全景",
        &format!("共 {} 位好友 · {} 条消息", aggregate.friends.len(), export::format_count(aggregate.total)),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        &format!("跨度 {} 天 · 活跃 {} 天 · 单日峰值 {}", aggregate.span_days(), aggregate.active_days(), export::format_count(peak.1)),
        &metrics,
        &body,
    )
}

fn render_group(conversation: &Conversation, aggregate: &Aggregate, account_id: Option<&str>) -> String {
    use std::fmt::Write as _;
    let peak = aggregate.peak();
    let mut body = String::new();
    write!(
        &mut body,
        "<section class=\"card\"><div class=\"section-head\"><h2>群聊概览</h2><span class=\"badge\">{}</span></div><p class=\"pk-summary\">群 <b>{}</b>，账号 <b>{}</b>，共 <b>{}</b> 位成员。</p><div class=\"pk-counts\"><span>消息总数 <b>{}</b></span><span>总字数 <b>{}</b></span><span>媒体消息 <b>{}</b></span></div></section>",
        export::escape_html(&conversation.name),
        export::escape_html(&conversation.id),
        export::escape_html(account_id.unwrap_or("-")),
        aggregate.members.len(),
        export::format_count(aggregate.total),
        export::format_count(aggregate.chars),
        aggregate.media.values().sum::<usize>()
    ).ok();

    let mut members: Vec<(&MemberAgg, usize)> = aggregate.members.values().map(|member| (member, member.total)).collect();
    members.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    let max_member = members.first().map(|(_, count)| *count).unwrap_or(1).max(1);
    let mut ranking = String::from("<div class=\"rows\">");
    for (member, count) in members.iter().take(20) {
        write!(
            &mut ranking,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} · {}</span></div>",
            export::escape_html(&member.name),
            *count as f64 * 100.0 / max_member as f64,
            export::format_count(*count),
            export::percent(*count, aggregate.total)
        ).ok();
    }
    ranking.push_str("</div>");
    body.push_str(&section("发言排行", "TOP 20", &ranking));

    let member_table: Vec<Vec<String>> = members
        .iter()
        .map(|(member, _)| {
            vec![
                member.name.clone(),
                member.id.clone(),
                export::format_count(member.total),
                export::format_count(member.media),
                export::format_count(member.chars),
                format!("{}", export::percent(member.total, aggregate.total)),
                if member.last > 0 { format_timestamp(member.last) } else { "-".to_string() },
            ]
        })
        .collect();
    body.push_str(&section(
        "群成员详细信息",
        &format!("共 {} 人", aggregate.members.len()),
        &table(&["成员", "账号", "发言", "媒体", "字数", "占比", "最后活跃"], &member_table),
    ));

    let hours = hour_rows(&aggregate.hours_all, &[0; 24], "群聊活跃时段", false);
    body.push_str(&section("活跃时段", "24 小时", &hours));

    body.push_str(&section(
        "消息类型",
        "全部消息",
        &type_rows(&aggregate.types, aggregate.total),
    ));

    let mut media = String::from("<div class=\"rows\">");
    for (label, count) in &aggregate.media {
        write!(
            &mut media,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill teal\" style=\"width:{:.1}%\"></div></div><span class=\"value\">{} 个</span></div>",
            export::escape_html(label),
            *count as f64 * 100.0 / aggregate.total.max(1) as f64,
            count
        ).ok();
    }
    if aggregate.missing_media > 0 {
        write!(&mut media, "<p class=\"chart-note\">其中 {} 个媒体未在本地索引到。</p>", aggregate.missing_media).ok();
    }
    media.push_str("</div>");
    body.push_str(&section("媒体内容", "图片 / 语音 / 视频 / 表情 / 文件", &media));

    let metrics = kpis_for(aggregate, "group");
    render_page(
        &format!("群聊分析报告 - {}", conversation.name),
        "GROUP CHAT ANALYTICS · 群聊分析",
        &format!("群聊分析 · {}", conversation.name),
        &format!("共 {} 条消息 · {} 位成员", export::format_count(aggregate.total), aggregate.members.len()),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        &format!("跨度 {} 天 · 活跃 {} 天 · 单日峰值 {}", aggregate.span_days(), aggregate.active_days(), export::format_count(peak.1)),
        &metrics,
        &body,
    )
}

fn render_annual(year: i32, aggregate: &Aggregate) -> String {
    let peak = aggregate.peak();
    let label = if year == 0 { "长期历史报告".to_string() } else { format!("{year} 年度报告") };
    let mut body = String::new();

    let months = month_bars(&aggregate.months, 36);
    body.push_str(&section("月度消息", if year == 0 { "最近 36 个月" } else { "全年 12 个月" }, &months));

    let mut friends: Vec<(&FriendAgg, usize)> = aggregate.friends.values().map(|friend| (friend, friend.total)).collect();
    friends.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    let friend_table: Vec<Vec<String>> = friends
        .iter()
        .take(15)
        .map(|(friend, _)| {
            vec![
                friend.name.clone(),
                export::format_count(friend.total),
                export::format_count(friend.sent),
                export::format_count(friend.received),
                format_timestamp(friend.last),
            ]
        })
        .collect();
    body.push_str(&section("好友消息排行", "TOP 15", &table(&["好友", "消息", "我发", "对方发", "最后活跃"], &friend_table)));

    let mut groups: Vec<(&FriendAgg, usize)> = aggregate.groups.values().map(|group| (group, group.total)).collect();
    groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    let group_table: Vec<Vec<String>> = groups
        .iter()
        .take(15)
        .map(|(group, _)| vec![group.name.clone(), export::format_count(group.total), format_timestamp(group.last)])
        .collect();
    body.push_str(&section("群聊消息排行", "TOP 15", &table(&["群聊", "消息", "最后活跃"], &group_table)));

    body.push_str(&section("消息类型", "全部消息", &type_rows(&aggregate.types, aggregate.total)));

    let hours = hour_rows(&aggregate.hours_all, &[0; 24], "活跃时段", false);
    body.push_str(&section("活跃时段", "24 小时", &hours));

    body.push_str(&section("最活跃的日子", "TOP 10", &top_day_rows(&aggregate.days)));

    let metrics = kpis_for(aggregate, "annual");
    render_page(
        &format!("年度分析报告 - {}", if year == 0 { "全部历史".to_string() } else { year.to_string() }),
        "ANNUAL CHAT ANALYTICS · 年度报告",
        &label,
        &format!("共 {} 位好友 · {} 个群聊 · {} 条消息", aggregate.friends.len(), aggregate.groups.len(), export::format_count(aggregate.total)),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        &format!("跨度 {} 天 · 活跃 {} 天 · 单日峰值 {}", aggregate.span_days(), aggregate.active_days(), export::format_count(peak.1)),
        &metrics,
        &body,
    )
}

pub fn run(
    kind: ReportKind,
    options: &ValidatedCli,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    contact_db: Option<&Path>,
    self_username: Option<&str>,
    account_id: Option<&str>,
) -> Result<()> {
    let output_dir = options
        .output_dir
        .as_ref()
        .context("report output directory is missing")?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let year = options.report_year;
    let (html, file_name, report_type, conversation_id) = match kind {
        ReportKind::Private => {
            let (html, _) = private_report(options, store, contacts, conversations, self_username)?;
            (html, "private-report.html", "private", None)
        }
        ReportKind::Group => {
            let id = options.conversation_ids.first().context("group report requires --conversation-id")?;
            let conversation = conversations
                .iter()
                .find(|item| item.id == *id)
                .with_context(|| format!("conversation not found: {id}"))?;
            if !conversation.is_group {
                bail!("group report requires a group conversation");
            }
            let (start, end) = year_bounds(year);
            let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
            if messages.is_empty() {
                bail!("no messages available for group {}", id);
            }
            let member_ids = crate::wechat::contact::load_group_members(
                contact_db,
                &conversation.id,
                options.key.as_ref(),
                options.plaintext,
            );
            let (html, _) = group_report(conversation, &messages, account_id, &member_ids, contacts)?;
            (html, "group-report.html", "group", Some(conversation.id.clone()))
        }
        ReportKind::Annual => {
            let (html, _) = annual_report(year, store, contacts, conversations, self_username)?;
            (html, "annual-report.html", "annual", None)
        }
        ReportKind::Dual => {
            let id = options.conversation_ids.first().context("dual report requires --conversation-id")?;
            let conversation = conversations
                .iter()
                .find(|item| item.id == *id)
                .with_context(|| format!("conversation not found: {id}"))?;
            if conversation.is_group {
                bail!("dual report requires a private conversation");
            }
            let (start, end) = year_bounds(year);
            let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
            if messages.is_empty() {
                bail!("no messages available for conversation {}", id);
            }
            let self_name = self_username
                .and_then(|uid| contacts.get(uid))
                .map(|contact| contact.display_name())
                .filter(|value| !value.is_empty())
                .unwrap_or_default();
            let context = ExportContext {
                conversation,
                messages: &messages,
                self_username,
                self_name,
                exported_at_ms: now_ms(),
                include_media: false,
                avatar_path: None,
            };
            let path = output_dir.join("dual-report.html");
            export::write_analytics_report(&context, &path)?;
            let result = serde_json::json!({
                "type": "dual",
                "conversation_id": conversation.id,
                "conversation_name": conversation.name,
                "message_count": messages.len(),
                "report": path,
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }
    };
    let path = output_dir.join(file_name);
    fs::write(&path, html).with_context(|| format!("failed to write {}", path.display()))?;
    let result = serde_json::json!({
        "type": report_type,
        "year": year,
        "conversation_id": conversation_id,
        "report": path,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[derive(Clone)]
struct LoadedConversation {
    conversation: Conversation,
    messages: Vec<ExportMessage>,
    exported_at_ms: i64,
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse JSON {}", path.display()))
}

fn find_conversation_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_conversation_files(&path, depth + 1, out);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("conversation.json") {
            out.push(path);
        }
    }
}

fn load_json_conversation(path: &Path) -> Result<LoadedConversation> {
    let value = read_json(path)?;
    parse_json_conversation(&value, path)
}

fn parse_json_conversation(value: &serde_json::Value, path: &Path) -> Result<LoadedConversation> {
    let session = value.get("session");
    let id = session
        .and_then(|session| session.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    let name = session
        .and_then(|session| session.get("name"))
        .and_then(|name| name.as_str())
        .or_else(|| session.and_then(|session| session.get("remark")).and_then(|name| name.as_str()))
        .unwrap_or("conversation")
        .to_string();
    let is_group = session
        .and_then(|session| session.get("kind"))
        .and_then(|kind| kind.as_str())
        .map(|kind| kind == "group")
        .unwrap_or_else(|| id.ends_with("@chatroom"));
    let messages_json = value
        .get("messages")
        .and_then(|messages| messages.as_array())
        .cloned()
        .unwrap_or_default();
    let mut messages = Vec::with_capacity(messages_json.len());
    for (index, message) in messages_json.iter().enumerate() {
        let sender = message.get("sender");
        let sender_id = sender
            .and_then(|sender| sender.get("id"))
            .and_then(|id| id.as_str())
            .unwrap_or("")
            .to_string();
        let sender_name = sender
            .and_then(|sender| sender.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or("")
            .to_string();
        let media = message.get("media").and_then(|media| {
            let kind = match media.get("kind").and_then(|kind| kind.as_str()).unwrap_or("image") {
                "video" => MediaKind::Video,
                "voice" => MediaKind::Voice,
                "emoji" => MediaKind::Emoji,
                "file" => MediaKind::File,
                _ => MediaKind::Image,
            };
            let path = media.get("path").and_then(|path| path.as_str()).unwrap_or("");
            Some(MediaRef {
                md5: media.get("md5").and_then(|md5| md5.as_str()).unwrap_or("").to_string(),
                file_name: media.get("name").and_then(|name| name.as_str()).unwrap_or("").to_string(),
                kind,
                ..MediaRef::default()
            }).map(|media_ref| (media_ref, path.to_string()))
        });
        let (media_ref, media_path) = match media {
            Some((media_ref, path)) => (Some(media_ref), if path.is_empty() { None } else { Some(path) }),
            None => (None, None),
        };
        let create_time = message.get("time").and_then(|time| time.as_i64()).unwrap_or(0);
        messages.push(ExportMessage {
            index: index + 1,
            local_id: message.get("id").and_then(|id| id.as_i64()).unwrap_or(index as i64 + 1),
            server_id: message
                .get("platformId")
                .and_then(|id| id.as_str())
                .and_then(|id| id.parse::<i64>().ok())
                .unwrap_or(0),
            create_time,
            sender_id,
            sender_name,
            nickname: sender.and_then(|sender| sender.get("nickname")).and_then(|value| value.as_str()).unwrap_or("").to_string(),
            remark: sender.and_then(|sender| sender.get("remark")).and_then(|value| value.as_str()).unwrap_or("").to_string(),
            alias: sender.and_then(|sender| sender.get("alias")).and_then(|value| value.as_str()).unwrap_or("").to_string(),
            is_send: message.get("fromMe").and_then(|value| value.as_bool()).unwrap_or(false),
            local_type: message.get("typeCode").and_then(|value| value.as_i64()).unwrap_or(0),
            base_type: message.get("typeCode").and_then(|value| value.as_i64()).unwrap_or(0),
            type_name: message.get("type").and_then(|value| value.as_str()).unwrap_or("消息").to_string(),
            content: message.get("content").and_then(|value| value.as_str()).unwrap_or("").to_string(),
            source: MessageSource::default(),
            app_subtype: message.get("app").and_then(|app| app.get("type")).and_then(|value| value.as_i64()),
            media: media_ref,
            media_path,
            quote: None,
            record: None,
        });
    }
    if messages.is_empty() {
        bail!("no messages in {}", path.display());
    }
    messages.sort_by_key(|message| message.create_time);
    let first = messages.iter().map(|message| message.create_time).min().unwrap_or(0);
    let last = messages.iter().map(|message| message.create_time).max().unwrap_or(0);
    let exported_at_ms = value
        .get("weflow")
        .and_then(|weflow| weflow.get("exportedAt"))
        .and_then(|value| value.as_i64())
        .map(|seconds| seconds * 1000)
        .unwrap_or_else(now_ms);
    Ok(LoadedConversation {
        conversation: Conversation {
            id,
            name,
            nickname: session.and_then(|s| s.get("nickname")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            remark: session.and_then(|s| s.get("remark")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            alias: session.and_then(|s| s.get("alias")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            is_group,
            unread_count: 0,
            sort_timestamp: last,
            last_timestamp: last,
            message_count: messages.len() as i64,
        },
        messages,
        exported_at_ms: exported_at_ms.max(first * 1000),
    })
}

fn load_json_path(path: &Path) -> Result<Vec<LoadedConversation>> {
    let mut files = Vec::new();
    if path.is_dir() {
        let report = path.join("export-report.json");
        if report.is_file() {
            return load_export_report(&report);
        }
        find_conversation_files(path, 0, &mut files);
    } else {
        let value = read_json(path)?;
        if value.get("conversations").is_some() {
            return load_export_report(path);
        }
        files.push(path.to_path_buf());
    }
    let mut loaded = Vec::new();
    for file in files {
        if let Ok(item) = load_json_conversation(&file) {
            loaded.push(item);
        }
    }
    if loaded.is_empty() {
        bail!("no conversation JSON found at {}", path.display());
    }
    Ok(loaded)
}

fn load_export_report(path: &Path) -> Result<Vec<LoadedConversation>> {
    let value = read_json(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut loaded = Vec::new();
    let Some(conversations) = value.get("conversations").and_then(|value| value.as_array()) else {
        bail!("export-report.json has no conversations array");
    };
    for conversation in conversations {
        let Some(json_path) = conversation
            .get("documents")
            .and_then(|documents| documents.get("json"))
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let path = base.join(json_path);
        if let Ok(item) = load_json_conversation(&path) {
            loaded.push(item);
        }
    }
    if loaded.is_empty() {
        bail!("export-report.json does not reference readable conversation.json files");
    }
    Ok(loaded)
}

fn filter_year(mut loaded: Vec<LoadedConversation>, year: i32) -> Vec<LoadedConversation> {
    if year == 0 {
        return loaded;
    }
    for item in &mut loaded {
        item.messages.retain(|message| {
            Local
                .timestamp_opt(message.create_time, 0)
                .single()
                .map(|date| date.year() == year)
                .unwrap_or(false)
        });
    }
    loaded.retain(|item| !item.messages.is_empty());
    loaded
}

fn aggregate_loaded(loaded: &[LoadedConversation], include_groups: bool, private_only: bool) -> Aggregate {
    let mut aggregate = Aggregate::new();
    for item in loaded {
        if private_only && !is_private_conversation(&item.conversation) {
            continue;
        }
        if !include_groups && item.conversation.is_group {
            continue;
        }
        let sent = item.messages.iter().filter(|message| message.is_send).count();
        for message in &item.messages {
            aggregate.observe(message);
        }
        let map = if item.conversation.is_group {
            &mut aggregate.groups
        } else {
            &mut aggregate.friends
        };
        let entry = map.entry(item.conversation.id.clone()).or_insert_with(|| FriendAgg {
            id: item.conversation.id.clone(),
            name: item.conversation.name.clone(),
            ..FriendAgg::default()
        });
        entry.name = item.conversation.name.clone();
        entry.total += item.messages.len();
        entry.sent += sent;
        entry.received += item.messages.len() - sent;
        entry.last = entry.last.max(item.messages.iter().map(|message| message.create_time).max().unwrap_or(0));
    }
    aggregate
}

fn select_loaded<'a>(
    loaded: &'a [LoadedConversation],
    selector: Option<&String>,
    want_group: bool,
) -> Result<&'a LoadedConversation> {
    if let Some(selector) = selector {
        return loaded
            .iter()
            .find(|item| item.conversation.id == *selector || item.conversation.name == *selector)
            .with_context(|| format!("conversation not found in JSON: {selector}"));
    }
    if loaded.len() == 1 {
        let item = &loaded[0];
        if want_group != item.conversation.is_group {
            bail!("selected JSON conversation does not match report type");
        }
        return Ok(item);
    }
    bail!("multiple conversations found; pass --conversation-id");
}

pub fn run_from_json(kind: ReportKind, options: &ValidatedCli, path: &Path) -> Result<()> {
    let output_dir = options.output_dir.as_ref().context("report output directory is missing")?;
    fs::create_dir_all(output_dir).with_context(|| format!("failed to create {}", output_dir.display()))?;
    let loaded = filter_year(load_json_path(path)?, options.report_year);
    if loaded.is_empty() {
        bail!("no messages available in the selected JSON for year {}", options.report_year);
    }
    match kind {
        ReportKind::Private => {
            let aggregate = aggregate_loaded(&loaded, false, true);
            if aggregate.total == 0 {
                bail!("no private conversation messages found in JSON");
            }
            let html = render_private(&aggregate);
            let path = output_dir.join("private-report.html");
            fs::write(&path, html).with_context(|| format!("failed to write {}", path.display()))?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"private","report":path}))?);
        }
        ReportKind::Group => {
            let item = select_loaded(&loaded, options.conversation_ids.first(), true)?;
            if !item.conversation.is_group {
                bail!("group report requires a group conversation");
            }
            let empty = HashMap::new();
            let (html, _) = group_report(&item.conversation, &item.messages, None, &[], &empty)?;
            let path = output_dir.join("group-report.html");
            fs::write(&path, html).with_context(|| format!("failed to write {}", path.display()))?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"group","report":path,"conversation_id":item.conversation.id}))?);
        }
        ReportKind::Annual => {
            let aggregate = aggregate_loaded(&loaded, true, false);
            if aggregate.total == 0 {
                bail!("no messages found in JSON");
            }
            let html = render_annual(options.report_year, &aggregate);
            let path = output_dir.join("annual-report.html");
            fs::write(&path, html).with_context(|| format!("failed to write {}", path.display()))?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"annual","year":options.report_year,"report":path}))?);
        }
        ReportKind::Dual => {
            let item = select_loaded(&loaded, options.conversation_ids.first(), false)?;
            if item.conversation.is_group {
                bail!("dual report requires a private conversation");
            }
            let self_name = item
                .messages
                .iter()
                .find(|message| message.is_send)
                .map(|message| message.sender_name.clone())
                .unwrap_or_default();
            let context = ExportContext {
                conversation: &item.conversation,
                messages: &item.messages,
                self_username: None,
                self_name,
                exported_at_ms: item.exported_at_ms,
                include_media: false,
                avatar_path: None,
            };
            let path = output_dir.join("dual-report.html");
            export::write_analytics_report(&context, &path)?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"dual","report":path,"conversation_id":item.conversation.id}))?);
        }
    }
    Ok(())
}
