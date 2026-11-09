use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::{ReportKind, ValidatedCli};
use crate::db::Db;
use crate::export::{self, ExportContext};
use crate::progress::Progress;
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

fn analysis_text(message: &ExportMessage) -> &str {
    if let Some(quote) = &message.quote {
        if !quote.reply.trim().is_empty() {
            return quote.reply.as_str();
        }
        return "";
    }
    if message.media.is_some() || message.record.is_some() {
        return "";
    }
    if message.type_name.contains("系统") || message.type_name.contains("撤回") {
        return "";
    }
    let content = message.content.trim_start();
    if content.starts_with("[引用") || content.starts_with("[聊天记录]") {
        return "";
    }
    message.content.as_str()
}

fn is_word_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('[') {
        return false;
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    if trimmed.contains("http://")
        || trimmed.contains("https://")
        || trimmed.contains("wxid_")
        || trimmed.contains("@chatroom")
        || trimmed.contains("eyJ")
    {
        return false;
    }
    if trimmed.contains('$')
        || trimmed.contains('{')
        || trimmed.contains('}')
        || trimmed.contains('\\')
        || trimmed.contains(';')
        || trimmed.starts_with('#')
    {
        return false;
    }
    let total = trimmed.chars().count();
    let ascii = trimmed.chars().filter(|ch| ch.is_ascii()).count();
    if total > 40 && ascii * 100 / total > 55 {
        return false;
    }
    true
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
    chars_me: usize,
    chars_other: usize,
    hours_me: [usize; 24],
    hours_other: [usize; 24],
    hours_all: [usize; 24],
    weekdays_me: [usize; 7],
    weekdays_other: [usize; 7],
    days: BTreeMap<String, (usize, usize)>,
    months: BTreeMap<String, (usize, usize)>,
    types: BTreeMap<String, usize>,
    media: BTreeMap<&'static str, usize>,
    missing_media: usize,
    friends: BTreeMap<String, FriendAgg>,
    groups: BTreeMap<String, FriendAgg>,
    members: BTreeMap<String, MemberAgg>,
    words_me: BTreeMap<String, usize>,
    words_other: BTreeMap<String, usize>,
    word_budget: usize,
    first: i64,
    last: i64,
}

impl Aggregate {
    fn new() -> Self {
        Self {
            first: i64::MAX,
            last: i64::MIN,
            word_budget: 2_000_000,
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
        let text = analysis_text(message);
        let message_chars = text.chars().count();
        self.chars += message_chars;
        if message.is_send {
            self.chars_me += message_chars;
        } else {
            self.chars_other += message_chars;
        }
        if self.word_budget > 0 && is_word_text(text) {
            if message.is_send {
                export::collect_words(text, &mut self.words_me);
            } else {
                export::collect_words(text, &mut self.words_other);
            }
            self.word_budget = self.word_budget.saturating_sub(message_chars.min(3000));
        }
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
            let weekday = date.weekday().num_days_from_monday() as usize;
            let bucket = self.days.entry(day).or_insert((0, 0));
            let month_bucket = self.months.entry(month).or_insert((0, 0));
            if message.is_send {
                bucket.0 += 1;
                month_bucket.0 += 1;
                self.hours_me[hour] += 1;
                self.weekdays_me[weekday] += 1;
            } else {
                bucket.1 += 1;
                month_bucket.1 += 1;
                self.hours_other[hour] += 1;
                self.weekdays_other[weekday] += 1;
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

#[derive(Default)]
struct FriendAnnual {
    id: String,
    name: String,
    total: usize,
    sent: usize,
    received: usize,
    days: std::collections::BTreeSet<String>,
    daily_messages: BTreeMap<String, usize>,
    months: BTreeMap<String, usize>,
    night: usize,
    last: i64,
    replies: usize,
    reply_sum_ms: i64,
    initiated_mine: usize,
    initiated_peer: usize,
}

#[derive(Default, Clone)]
struct MomentsStats {
    posts: usize,
    likes_received: usize,
    likes_given: usize,
    post_types: BTreeMap<String, usize>,
    top_likers: BTreeMap<String, usize>,
    top_liked_authors: BTreeMap<String, usize>,
}

#[derive(Default)]
struct AnnualStats {
    aggregate: Aggregate,
    friends: BTreeMap<String, FriendAnnual>,
    heatmap: [[usize; 24]; 7],
    starts_mine: usize,
    starts_peer: usize,
    reply_mine_sum_ms: i64,
    reply_mine_count: usize,
    reply_peer_sum_ms: i64,
    reply_peer_count: usize,
    monthly_friend: BTreeMap<String, BTreeMap<String, usize>>,
    monthly_total: BTreeMap<String, usize>,
    day_total: BTreeMap<String, usize>,
    day_friend: BTreeMap<String, BTreeMap<String, usize>>,
    phrases: BTreeMap<String, usize>,
    moments: Option<MomentsStats>,
}

impl AnnualStats {
    fn new() -> Self {
        Self {
            aggregate: Aggregate::new(),
            ..Self::default()
        }
    }

    fn sync_aggregate_friends(&mut self) {
        for (id, friend) in &self.friends {
            self.aggregate.friends.insert(
                id.clone(),
                FriendAgg {
                    id: id.clone(),
                    name: friend.name.clone(),
                    total: friend.total,
                    sent: friend.sent,
                    received: friend.received,
                    last: friend.last,
                },
            );
        }
    }

    fn observe_conversation(&mut self, conversation: &Conversation, messages: &[ExportMessage]) {
        let mut ordered: Vec<&ExportMessage> = messages.iter().collect();
        ordered.sort_by_key(|message| message.create_time);
        let mut previous: Option<&ExportMessage> = None;
        let mut segment_last = 0i64;
        for message in ordered {
            self.aggregate.observe(message);
            let Some(date) = Local.timestamp_opt(message.create_time, 0).single() else {
                previous = Some(message);
                continue;
            };
            let day = date.format("%Y-%m-%d").to_string();
            let month = date.format("%Y-%m").to_string();
            let hour = date.hour() as usize;
            let weekday = date.weekday().num_days_from_monday() as usize;
            self.heatmap[weekday][hour] += 1;
            *self.monthly_total.entry(month.clone()).or_default() += 1;
            *self
                .monthly_friend
                .entry(month.clone())
                .or_default()
                .entry(conversation.id.clone())
                .or_default() += 1;
            *self.day_total.entry(day.clone()).or_default() += 1;
            *self
                .day_friend
                .entry(day.clone())
                .or_default()
                .entry(conversation.id.clone())
                .or_default() += 1;
            let entry = self.friends.entry(conversation.id.clone()).or_insert_with(|| FriendAnnual {
                id: conversation.id.clone(),
                name: conversation.name.clone(),
                ..FriendAnnual::default()
            });
            entry.name = conversation.name.clone();
            entry.total += 1;
            entry.last = entry.last.max(message.create_time);
            entry.days.insert(day.clone());
            *entry.daily_messages.entry(day).or_default() += 1;
            *entry.months.entry(month.clone()).or_default() += 1;
            if message.is_send {
                entry.sent += 1;
            } else {
                entry.received += 1;
            }
            if hour < 6 {
                entry.night += 1;
            }
            let segment_start = message.is_send;
            if segment_last == 0 || message.create_time - segment_last > 3600 {
                if segment_start {
                    self.starts_mine += 1;
                    entry.initiated_mine += 1;
                } else {
                    self.starts_peer += 1;
                    entry.initiated_peer += 1;
                }
            }
            if let Some(previous) = previous {
                let gap_ms = (message.create_time - previous.create_time).max(0) * 1000;
                if previous.is_send != message.is_send && gap_ms <= 86_400_000 {
                    if message.is_send {
                        self.reply_mine_sum_ms += gap_ms;
                        self.reply_mine_count += 1;
                    } else {
                        self.reply_peer_sum_ms += gap_ms;
                        self.reply_peer_count += 1;
                    }
                    entry.replies += 1;
                    entry.reply_sum_ms += gap_ms;
                }
            }
            if message.is_send && message.media.is_none() && message.record.is_none() {
                let text = analysis_text(message).trim();
                let len = text.chars().count();
                if (2..=20).contains(&len) && is_word_text(text) {
                    *self.phrases.entry(text.to_string()).or_default() += 1;
                }
            }
            segment_last = message.create_time;
            previous = Some(message);
        }
    }
}

fn longest_streak(days: &std::collections::BTreeSet<String>) -> Option<(usize, String, String)> {
    let mut best = 0usize;
    let mut best_start = String::new();
    let mut best_end = String::new();
    let mut run = 0usize;
    let mut previous: Option<NaiveDate> = None;
    let mut run_start = String::new();
    for day in days {
        let Ok(date) = NaiveDate::parse_from_str(day, "%Y-%m-%d") else {
            continue;
        };
        if previous.and_then(|value| value.succ_opt()) == Some(date) {
            run += 1;
        } else {
            run = 1;
            run_start = day.clone();
        }
        if run > best {
            best = run;
            best_start = run_start.clone();
            best_end = day.clone();
        }
        previous = Some(date);
    }
    (best >= 2).then_some((best, best_start, best_end))
}

fn top_friend_rows(friends: &BTreeMap<String, FriendAnnual>) -> Vec<serde_json::Value> {
    let mut rows: Vec<&FriendAnnual> = friends.values().collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then(a.name.cmp(&b.name)));
    rows.into_iter()
        .map(|friend| serde_json::json!({
            "conversationId": friend.id,
            "displayName": friend.name,
            "messages": friend.total,
            "sent": friend.sent,
            "received": friend.received,
        }))
        .collect()
}

fn payload_annual_stats(stats: &AnnualStats, year: i32) -> serde_json::Value {
    let aggregate = &stats.aggregate;
    let mut friends = stats.friends.values().collect::<Vec<_>>();
    friends.sort_by(|a, b| b.total.cmp(&a.total));
    let top_friends = top_friend_rows(&stats.friends);
    let mut periods = Vec::new();
    if let (Some(start), Some(end)) = (
        Local.timestamp_opt(aggregate.first, 0).single(),
        Local.timestamp_opt(aggregate.last, 0).single(),
    ) {
        let mut year = start.year();
        let mut month = start.month();
        let end_year = end.year();
        let end_month = end.month();
        while year < end_year || (year == end_year && month <= end_month) {
            periods.push(format!("{year}-{month:02}"));
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
    }
    let mut monthly_companions = Vec::new();
    for period in periods {
        let period_messages = stats.monthly_total.get(&period).copied().unwrap_or(0);
        let top = stats.monthly_friend.get(&period).and_then(|counts| {
            counts
                .iter()
                .max_by_key(|(_, count)| **count)
                .and_then(|(id, messages)| stats.friends.get(id).map(|friend| (friend, messages)))
        });
        monthly_companions.push(match top {
            Some((friend, messages)) => serde_json::json!({
                "period": period,
                "conversationId": friend.id,
                "displayName": friend.name,
                "messages": messages,
                "periodMessages": period_messages,
            }),
            None => serde_json::json!({
                "period": period,
                "conversationId": serde_json::Value::Null,
                "displayName": "",
                "messages": 0,
                "periodMessages": 0,
            }),
        });
    }
    let mut heatmap_peak = serde_json::Value::Null;
    let mut peak_value = 0;
    for weekday in 0..7 {
        for hour in 0..24 {
            if stats.heatmap[weekday][hour] > peak_value {
                peak_value = stats.heatmap[weekday][hour];
                heatmap_peak = serde_json::json!({"weekday": weekday, "hour": hour, "messages": peak_value});
            }
        }
    }
    let night_total: usize = stats.friends.values().map(|friend| friend.night).sum();
    let night = stats.friends.values().max_by_key(|friend| friend.night).filter(|friend| friend.night > 0);
    let night_companion = night.map(|friend| serde_json::json!({
        "conversationId": friend.id,
        "displayName": friend.name,
        "nightMessages": friend.night,
        "share": if night_total == 0 { 0.0 } else { friend.night as f64 / night_total as f64 },
    }));
    let rate_mine = if stats.starts_mine + stats.starts_peer == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!(stats.starts_mine as f64 / (stats.starts_mine + stats.starts_peer) as f64)
    };
    let top_initiated = stats.friends.values().max_by_key(|friend| friend.initiated_mine).map(|friend| serde_json::json!({
        "conversationId": friend.id,
        "displayName": friend.name,
        "startsMine": friend.initiated_mine,
        "startsPeer": friend.initiated_peer,
    }));
    let mine = (stats.reply_mine_count > 0).then(|| serde_json::json!({
        "averageMs": stats.reply_mine_sum_ms / stats.reply_mine_count as i64,
        "count": stats.reply_mine_count,
    }));
    let peer = (stats.reply_peer_count > 0).then(|| serde_json::json!({
        "averageMs": stats.reply_peer_sum_ms / stats.reply_peer_count as i64,
        "count": stats.reply_peer_count,
    }));
    let fastest = stats.friends.values().filter(|friend| friend.replies >= 10)
        .min_by_key(|friend| if friend.replies == 0 { i64::MAX } else { friend.reply_sum_ms / friend.replies as i64 })
        .map(|friend| serde_json::json!({
            "conversationId": friend.id,
            "displayName": friend.name,
            "averageMs": if friend.replies == 0 { 0 } else { friend.reply_sum_ms / friend.replies as i64 },
            "replies": friend.replies,
        }));
    let balanced = stats.friends.values()
        .filter(|friend| friend.sent >= 50 && friend.received >= 50)
        .map(|friend| {
            let high = friend.sent.max(friend.received) as f64;
            (friend, if high == 0.0 { 0.0 } else { friend.sent.min(friend.received) as f64 / high })
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(friend, balance)| serde_json::json!({
            "conversationId": friend.id,
            "displayName": friend.name,
            "sent": friend.sent,
            "received": friend.received,
            "balance": balance,
        }));
    let streak = friends.iter().filter_map(|friend| {
        longest_streak(&friend.days).map(|(days, start, end)| (friend, days, start, end))
    }).max_by_key(|(_, days, _, _)| *days).map(|(friend, days, start, end)| serde_json::json!({
        "conversationId": friend.id,
        "displayName": friend.name,
        "days": days,
        "startDate": start,
        "endDate": end,
    }));
    let all_days: std::collections::BTreeSet<String> = aggregate.days.keys().cloned().collect();
    let account_streak = longest_streak(&all_days).map(|(days, start, end)| serde_json::json!({
        "days": days,
        "startDate": start,
        "endDate": end,
    }));
    let busiest = stats.day_total.iter().max_by_key(|(_, count)| **count).map(|(date, count)| {
        let top_friend = stats.day_friend.get(date).and_then(|counts| counts.iter().max_by_key(|(_, count)| **count)).map(|(friend_id, messages)| {
            let friend = stats.friends.get(friend_id);
            serde_json::json!({
                "displayName": friend.map(|friend| friend.name.as_str()).unwrap_or(friend_id),
                "messages": messages,
            })
        });
        serde_json::json!({"date": date, "messages": count, "topFriend": top_friend})
    });
    let mut phrase_rows: Vec<(&String, &usize)> = stats.phrases.iter().filter(|(_, count)| **count >= 2).collect();
    phrase_rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    phrase_rows.truncate(10);
    let phrases = phrase_rows.into_iter().map(|(text, count)| serde_json::json!({"text": text, "count": count})).collect::<Vec<_>>();
    let fading = if let (Some(first), Some(last)) = (
        aggregate.days.keys().next().and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()),
        aggregate.days.keys().next_back().and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()),
    ) {
        let span = (last - first).num_days();
        if span >= 60 {
            let mid = first + chrono::Duration::days(span / 2);
            let threshold = if year == 0 { 100 } else { 80 };
            friends.iter().filter_map(|friend| {
                let mut early = 0usize;
                let mut late = 0usize;
                for (day, count) in &friend.daily_messages {
                    if let Ok(date) = NaiveDate::parse_from_str(day, "%Y-%m-%d") {
                        if date <= mid { early += *count; } else { late += *count; }
                    }
                }
                (early >= threshold && early > late.saturating_mul(5)).then_some((friend, early, late))
            }).max_by_key(|(_, early, _)| *early).map(|(friend, early, late)| serde_json::json!({
                "conversationId": friend.id,
                "displayName": friend.name,
                "earlyMessages": early,
                "lateMessages": late,
                "earlyStart": first.to_string(),
                "earlyEnd": mid.to_string(),
                "lateStart": mid.succ_opt().unwrap_or(mid).to_string(),
                "lateEnd": last.to_string(),
            }))
        } else { None }
    } else { None };
    let mut summary = Vec::new();
    summary.push(serde_json::json!({"label":"私聊消息","value":format!("{} 条", export::format_count(aggregate.total))}));
    summary.push(serde_json::json!({"label":"聊过的好友","value":format!("{} 位", stats.friends.len())}));
    if let Some(top) = top_friends.first() {
        summary.push(serde_json::json!({"label":"聊得最多","value":format!("{} 条", top["messages"]), "person": top["displayName"]}));
    }
    if peak_value > 0 {
        summary.push(serde_json::json!({"label":"最常聊天","value":format!("周{} {:02}:00", ["一","二","三","四","五","六","日"][heatmap_peak["weekday"].as_u64().unwrap_or(0) as usize], heatmap_peak["hour"].as_u64().unwrap_or(0))}));
    }
    if !rate_mine.is_null() {
        summary.push(serde_json::json!({"label":"你先开口","value":format!("{:.0}%", rate_mine.as_f64().unwrap_or(0.0) * 100.0)}));
    }
    if let Some(mine) = &mine {
        summary.push(serde_json::json!({"label":"你的平均回复","value":format_duration(mine["averageMs"].as_i64().unwrap_or(0))}));
    }
    if let Some(streak) = &streak {
        summary.push(serde_json::json!({"label":"最长连续","value":format!("{} 天", streak["days"]), "person": streak["displayName"]}));
    }
    if let Some(busiest) = &busiest {
        summary.push(serde_json::json!({"label":"私聊最多的一天","value":format!("{} · {} 条", busiest["date"].as_str().unwrap_or(""), busiest["messages"])}));
    }
    if let Some(phrase) = phrases.first() {
        summary.push(serde_json::json!({"label":"口头禅","value":format!("「{}」 {} 次", phrase["text"].as_str().unwrap_or(""), phrase["count"])}));
    }
    serde_json::json!({
        "topFriends": top_friends,
        "monthlyCompanions": monthly_companions,
        "heatmap": {"weekdayHourCounts": stats.heatmap, "peak": heatmap_peak},
        "nightCompanion": night_companion,
        "nightShareOfAll": if aggregate.total == 0 { serde_json::Value::Null } else { serde_json::json!(night_total as f64 / aggregate.total as f64) },
        "initiative": {"startsMine": stats.starts_mine, "startsPeer": stats.starts_peer, "rateMine": rate_mine, "topInitiated": top_initiated},
        "response": {"mine": mine, "peer": peer, "fastestFriend": fastest},
        "balancedFriend": balanced,
        "streak": streak,
        "accountStreak": account_streak,
        "busiestDay": busiest,
        "fading": fading,
        "phrases": phrases,
        "summary": summary,
        "moments": stats.moments.as_ref().map(|moments| {
            let mut post_types: Vec<_> = moments.post_types.iter().collect();
            post_types.sort_by(|a, b| b.1.cmp(a.1));
            let mut likers: Vec<_> = moments.top_likers.iter().collect();
            likers.sort_by(|a, b| b.1.cmp(a.1));
            let mut liked: Vec<_> = moments.top_liked_authors.iter().collect();
            liked.sort_by(|a, b| b.1.cmp(a.1));
            serde_json::json!({
                "status": "ok",
                "posts": moments.posts,
                "likesReceived": moments.likes_received,
                "likesGiven": moments.likes_given,
                "postTypes": post_types.into_iter().map(|(name, count)| serde_json::json!({"contentType": name, "count": count})).collect::<Vec<_>>(),
                "topLikers": likers.into_iter().map(|(username, count)| serde_json::json!({"username": username, "count": count})).collect::<Vec<_>>(),
                "topLikedAuthors": liked.into_iter().map(|(username, count)| serde_json::json!({"username": username, "count": count})).collect::<Vec<_>>(),
            })
        }),
    })
}

fn format_duration(ms: i64) -> String {
    if ms < 1000 { format!("{ms} 毫秒") }
    else if ms < 60_000 { format!("{:.1} 秒", ms as f64 / 1000.0) }
    else if ms < 3_600_000 { format!("{:.1} 分钟", ms as f64 / 60_000.0) }
    else { format!("{:.1} 小时", ms as f64 / 3_600_000.0) }
}

fn moments_db_path(account_root: Option<&Path>, db_storage: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = account_root {
        candidates.push(root.join("sns").join("sns.db"));
        candidates.push(root.join("db_storage").join("sns").join("sns.db"));
    }
    if let Some(db) = db_storage {
        candidates.push(db.join("sns").join("sns.db"));
        if let Some(parent) = db.parent() {
            candidates.push(parent.join("sns").join("sns.db"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

fn load_moments(
    account_root: Option<&Path>,
    db_storage: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    self_username: Option<&str>,
    year: i32,
) -> Option<MomentsStats> {
    let path = moments_db_path(account_root, db_storage)?;
    let db = Db::open(&path, key, plaintext).ok()?;
    let tables = db.tables().ok()?;
    let table = tables.iter().find(|table| {
        table.eq_ignore_ascii_case("SnsTimeLine") || table.to_ascii_lowercase().contains("snstimeline")
    })?.clone();
    let columns = db.columns(&table).ok()?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let user_col = ["user_name", "userName", "username"].iter().find(|name| has(name))?;
    let content_col = ["content", "Content"].iter().find(|name| has(name))?;
    let time_col = ["create_time", "createTime", "createtime"].iter().find(|name| has(name)).copied();
    let sql = format!(
        "SELECT {}, {}{} FROM {}",
        crate::db::quote_ident(user_col),
        crate::db::quote_ident(content_col),
        time_col.map(|column| format!(", {}", crate::db::quote_ident(column))).unwrap_or_default(),
        crate::db::quote_ident(&table)
    );
    let mut stmt = db.conn().prepare(&sql).ok()?;
    let mut rows = stmt.query([]).ok()?;
    let self_id = self_username.unwrap_or("").to_lowercase();
    let mut stats = MomentsStats::default();
    while let Ok(Some(row)) = rows.next() {
        let poster = match row.get::<_, Option<rusqlite::types::Value>>(0).ok().flatten() {
            Some(rusqlite::types::Value::Text(value)) => value,
            Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
            _ => String::new(),
        };
        let content = match row.get::<_, Option<rusqlite::types::Value>>(1).ok().flatten() {
            Some(rusqlite::types::Value::Text(value)) => value,
            Some(rusqlite::types::Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
            _ => String::new(),
        };
        let table_time = if time_col.is_some() {
            match row.get::<_, Option<rusqlite::types::Value>>(2).ok().flatten() {
                Some(rusqlite::types::Value::Integer(value)) => value,
                Some(rusqlite::types::Value::Text(value)) => value.trim().parse::<i64>().unwrap_or(0),
                _ => 0,
            }
        } else {
            0
        };
        let timestamp = crate::wechat::parse::xml_value(&content, "createTime")
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(table_time);
        let timestamp = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
        if timestamp <= 0 {
            continue;
        }
        let Some(date) = Local.timestamp_opt(timestamp, 0).single() else {
            continue;
        };
        if year != 0 && date.year() != year {
            continue;
        }
        let likes = regex::Regex::new(r"(?is)<like>.*?<username>(.*?)</username>")
            .ok()?
            .captures_iter(&content)
            .filter_map(|capture| capture.get(1).map(|value| crate::wechat::parse::unescape_xml(value.as_str()).trim().to_string()))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if poster.to_lowercase() == self_id {
            stats.posts += 1;
            stats.likes_received += likes.len();
            let content_type = crate::wechat::parse::xml_value(&content, "contentType")
                .or_else(|| crate::wechat::parse::xml_value(&content, "type"))
                .unwrap_or_else(|| "0".to_string());
            let label = match content_type.as_str() {
                "1" => "图片", "2" => "文字", "3" => "链接", "4" => "音乐",
                "5" => "分享", "15" => "视频", "28" => "视频号", "34" => "视频号直播",
                "42" => "音乐", "54" => "实况照片", _ => "其他",
            };
            *stats.post_types.entry(label.to_string()).or_default() += 1;
            for liker in likes {
                *stats.top_likers.entry(liker).or_default() += 1;
            }
        } else if likes.iter().any(|liker| liker.to_lowercase() == self_id) {
            stats.likes_given += 1;
            *stats.top_liked_authors.entry(poster).or_default() += 1;
        }
    }
    if stats.posts == 0 && stats.likes_given == 0 {
        return None;
    }
    Some(stats)
}

fn private_report(
    options: &ValidatedCli,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
    progress: &mut Progress,
) -> Result<Aggregate> {
    let mut aggregate = Aggregate::new();
    let private: Vec<&Conversation> = conversations.iter().filter(|item| is_private_conversation(item)).collect();
    let mut scanned = 0usize;
    for (index, conversation) in private.iter().enumerate() {
        progress.set_phase(format!(
            "扫描私聊 {}/{} · {} · 已读取 {} 条",
            index + 1,
            private.len(),
            conversation.name,
            scanned
        ));
        progress.steps(index + 1, private.len(), "会话");
        let messages = load_messages(store, conversation, contacts, self_username, None, None)?;
        scanned += messages.len();
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
    Ok(aggregate)
}

fn group_report(
    conversation: &Conversation,
    messages: &[ExportMessage],
    account_id: Option<&str>,
    member_ids: &[String],
    contacts: &HashMap<String, Contact>,
) -> Result<Aggregate> {
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
    let _ = (conversation, account_id);
    Ok(aggregate)
}

fn annual_report(
    year: i32,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    self_username: Option<&str>,
    account_root: Option<&Path>,
    db_storage: Option<&Path>,
    key: Option<&[u8; 32]>,
    plaintext: bool,
    progress: &mut Progress,
) -> Result<AnnualStats> {
    let (start, end) = year_bounds(year);
    let mut stats = AnnualStats::new();
    let analyzable: Vec<&Conversation> = conversations.iter().filter(|item| is_private_conversation(item)).collect();
    let mut scanned = 0usize;
    for (index, conversation) in analyzable.iter().enumerate() {
        progress.set_phase(format!(
            "扫描年度会话 {}/{} · {} · 已读取 {} 条",
            index + 1,
            analyzable.len(),
            conversation.name,
            scanned
        ));
        progress.steps(index + 1, analyzable.len(), "会话");
        let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
        scanned += messages.len();
        if messages.is_empty() {
            continue;
        }
        stats.observe_conversation(conversation, &messages);
    }
    stats.sync_aggregate_friends();
    progress.set_phase("分析朋友圈");
    stats.moments = load_moments(account_root, db_storage, key, plaintext, self_username, year);
    if let Some(moments) = &mut stats.moments {
        let display = |username: &str| {
            contacts
                .get(username)
                .map(|contact| contact.display_name())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| username.to_string())
        };
        moments.top_likers = moments
            .top_likers
            .iter()
            .map(|(username, count)| (display(username), *count))
            .collect();
        moments.top_liked_authors = moments
            .top_liked_authors
            .iter()
            .map(|(username, count)| (display(username), *count))
            .collect();
    }
    if stats.aggregate.total == 0 {
        bail!("no messages available for the selected year");
    }
    Ok(stats)
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
        }
        _ => {}
    }
    metrics.push(("消息总数".into(), export::format_count(aggregate.total), "含系统与媒体消息".into()));
    metrics.push(("总字数".into(), export::format_count(aggregate.chars), format!("平均 {avg:.1} 字/条")));
    metrics.push(("活跃天数".into(), export::format_count(aggregate.active_days()), format!("跨度 {} 天", aggregate.span_days())));
    metrics.push(("单日最高".into(), export::format_count(peak.1), peak.0));
    metrics
}

fn series_from_days(days: &BTreeMap<String, (usize, usize)>) -> Vec<serde_json::Value> {
    days.iter()
        .map(|(day, (me, other))| serde_json::json!({"label": day, "me": me, "other": other}))
        .collect()
}

fn series_from_months(months: &BTreeMap<String, (usize, usize)>) -> Vec<serde_json::Value> {
    months.iter()
        .map(|(month, (me, other))| serde_json::json!({"label": month, "me": me, "other": other}))
        .collect()
}

fn series_from_hours(me: &[usize; 24], other: &[usize; 24]) -> Vec<serde_json::Value> {
    (0..24)
        .map(|hour| serde_json::json!({"label": format!("{hour:02}"), "me": me[hour], "other": other[hour]}))
        .collect()
}

fn series_from_week(me: &[usize; 7], other: &[usize; 7]) -> Vec<serde_json::Value> {
    let labels = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    (0..7)
        .map(|day| serde_json::json!({"label": labels[day], "me": me[day], "other": other[day]}))
        .collect()
}

fn count_rows(map: &BTreeMap<String, usize>) -> Vec<serde_json::Value> {
    let mut rows: Vec<(&String, &usize)> = map.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows.into_iter()
        .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
        .collect()
}

fn friend_rows(friends: &BTreeMap<String, FriendAgg>, total: usize) -> Vec<serde_json::Value> {
    let mut rows: Vec<&FriendAgg> = friends.values().collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then(a.name.cmp(&b.name)));
    rows.into_iter()
        .map(|friend| {
            serde_json::json!({
                "name": friend.name,
                "id": friend.id,
                "count": friend.total,
                "sent": friend.sent,
                "received": friend.received,
                "last": if friend.last > 0 { format_timestamp(friend.last) } else { "-".to_string() },
                "share": if total == 0 { 0.0 } else { friend.total as f64 / total as f64 },
            })
        })
        .collect()
}

fn group_rows(groups: &BTreeMap<String, FriendAgg>) -> Vec<serde_json::Value> {
    let mut rows: Vec<&FriendAgg> = groups.values().collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then(a.name.cmp(&b.name)));
    rows.into_iter()
        .map(|group| {
            serde_json::json!({
                "name": group.name,
                "id": group.id,
                "count": group.total,
                "last": if group.last > 0 { format_timestamp(group.last) } else { "-".to_string() },
            })
        })
        .collect()
}

fn member_rows(members: &BTreeMap<String, MemberAgg>, total: usize) -> Vec<serde_json::Value> {
    let mut rows: Vec<&MemberAgg> = members.values().collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then(a.name.cmp(&b.name)));
    rows.into_iter()
        .map(|member| {
            serde_json::json!({
                "name": member.name,
                "id": member.id,
                "count": member.total,
                "media": member.media,
                "chars": member.chars,
                "share": if total == 0 { 0.0 } else { member.total as f64 / total as f64 },
                "last": if member.last > 0 { format_timestamp(member.last) } else { "-".to_string() },
            })
        })
        .collect()
}

fn word_rows(words: &BTreeMap<String, usize>) -> Vec<serde_json::Value> {
    let mut rows: Vec<(&String, &usize)> = words.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows.truncate(24);
    rows.into_iter()
        .map(|(word, count)| serde_json::json!({"word": word, "count": count}))
        .collect()
}

fn top_day_rows_json(days: &BTreeMap<String, (usize, usize)>) -> Vec<serde_json::Value> {
    let mut rows: Vec<(&String, &(usize, usize))> = days.iter().collect();
    rows.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)).then(a.0.cmp(b.0)));
    rows.truncate(10);
    rows.into_iter()
        .map(|(date, (me, other))| {
            serde_json::json!({"date": date, "count": me + other, "me": me, "other": other})
        })
        .collect()
}

fn payload_common(
    kind: &str,
    title: &str,
    heading: &str,
    subtitle: &str,
    range: &str,
    meta: Vec<String>,
    kpis: &[(String, String, String)],
    aggregate: &Aggregate,
    me_label: &str,
    other_label: &str,
) -> serde_json::Value {
    let ratio = if aggregate.total > 0 {
        serde_json::json!({
            "me": aggregate.sent,
            "other": aggregate.received,
            "meLabel": me_label,
            "otherLabel": other_label,
            "meAvg": if aggregate.sent == 0 { 0.0 } else { aggregate.chars_me as f64 / aggregate.sent as f64 },
            "otherAvg": if aggregate.received == 0 { 0.0 } else { aggregate.chars_other as f64 / aggregate.received as f64 },
        })
    } else {
        serde_json::Value::Null
    };
    let words = if aggregate.words_me.is_empty() && aggregate.words_other.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "meLabel": me_label,
            "otherLabel": other_label,
            "me": word_rows(&aggregate.words_me),
            "other": word_rows(&aggregate.words_other),
        })
    };
    serde_json::json!({
        "kind": kind,
        "title": title,
        "heading": heading,
        "subtitle": subtitle,
        "range": range,
        "meta": meta,
        "kpis": kpis.iter().map(|(label, value, note)| serde_json::json!({
            "label": label, "value": value, "note": note
        })).collect::<Vec<_>>(),
        "ratio": ratio,
        "trend": series_from_days(&aggregate.days),
        "months": series_from_months(&aggregate.months),
        "hours": series_from_hours(&aggregate.hours_me, &aggregate.hours_other),
        "weekdays": series_from_week(&aggregate.weekdays_me, &aggregate.weekdays_other),
        "types": count_rows(&aggregate.types),
        "media": count_rows(&aggregate.media.iter().map(|(name, count)| (name.to_string(), *count)).collect()),
        "missingMedia": aggregate.missing_media,
        "topDays": top_day_rows_json(&aggregate.days),
        "friends": friend_rows(&aggregate.friends, aggregate.total),
        "groups": group_rows(&aggregate.groups),
        "members": member_rows(&aggregate.members, aggregate.total),
        "words": words,
        "generator": GENERATOR,
    })
}

fn payload_private(aggregate: &Aggregate) -> serde_json::Value {
    let peak = aggregate.peak();
    payload_common(
        "private",
        "私聊分析报告",
        "私聊分析 · 好友消息全景",
        &format!("共 {} 位好友 · {} 条消息", aggregate.friends.len(), export::format_count(aggregate.total)),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        vec![
            format!("跨度 {} 天 · 活跃 {} 天", aggregate.span_days(), aggregate.active_days()),
            format!("单日峰值 {} 条", export::format_count(peak.1)),
            format!("导出时间 {}", format_timestamp(now_ms() / 1000)),
        ],
        &kpis_for(aggregate, "private"),
        aggregate,
        "我",
        "好友",
    )
}

fn payload_group(conversation: &Conversation, aggregate: &Aggregate) -> serde_json::Value {
    let peak = aggregate.peak();
    payload_common(
        "group",
        &format!("群聊分析报告 - {}", conversation.name),
        &format!("群聊分析 · {}", conversation.name),
        &format!("共 {} 条消息 · {} 位成员", export::format_count(aggregate.total), aggregate.members.len()),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        vec![
            format!("跨度 {} 天 · 活跃 {} 天", aggregate.span_days(), aggregate.active_days()),
            format!("单日峰值 {} 条", export::format_count(peak.1)),
            format!("群 ID {}", conversation.id),
        ],
        &kpis_for(aggregate, "group"),
        aggregate,
        "我",
        "其他人",
    )
}

fn payload_annual(year: i32, stats: &AnnualStats) -> serde_json::Value {
    let aggregate = &stats.aggregate;
    let peak = aggregate.peak();
    let title = if year == 0 { "长期历史报告" } else { &format!("{year} 年度报告") };
    let mut payload = payload_common(
        "annual",
        &format!("年度分析报告 - {}", if year == 0 { "全部历史".to_string() } else { year.to_string() }),
        title,
        &format!("共 {} 位好友 · {} 条私聊消息", aggregate.friends.len(), export::format_count(aggregate.total)),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        vec![
            format!("跨度 {} 天 · 活跃 {} 天", aggregate.span_days(), aggregate.active_days()),
            format!("单日峰值 {} 条", export::format_count(peak.1)),
        ],
        &kpis_for(aggregate, "annual"),
        aggregate,
        "我",
        "其他人",
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("annual".to_string(), payload_annual_stats(stats, year));
    }
    payload
}

fn payload_dual(context: &ExportContext<'_>) -> serde_json::Value {
    let mut aggregate = Aggregate::new();
    for message in context.messages {
        aggregate.observe(message);
    }
    let other_label = if context.group() { "其他人" } else { context.display_name() };
    let total = context.messages.len();
    payload_common(
        "dual",
        &format!("聊天分析报告 - {}", context.display_name()),
        "微信聊天记录 · 可视化报告",
        &format!("与 {} 的对话轨迹 · 共 {} 条记录", other_label, export::format_count(total)),
        &format!("{} → {}", format_timestamp(aggregate.first), format_timestamp(aggregate.last)),
        vec![
            format!("跨度 {} 天 · 活跃 {} 天", aggregate.span_days(), aggregate.active_days()),
            format!("导出于 {}", format_timestamp(context.exported_at_sec())),
        ],
        &kpis_for(&aggregate, "dual"),
        &aggregate,
        "我",
        other_label,
    )
}

fn write_payload(payload: &serde_json::Value, path: &Path) -> Result<()> {
    const TEMPLATE: &str = include_str!("export/report_template.html");
    let data = serde_json::to_string(payload).context("serialize report payload")?;
    let data = data.replace('<', "\\u003c");
    let html = TEMPLATE.replace("__REPORT_DATA__", &data);
    fs::write(path, html).with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn write_dual_report(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    write_payload(&payload_dual(context), path)
}

pub(crate) fn write_conversation_report(context: &ExportContext<'_>, path: &Path) -> Result<()> {
    if context.group() {
        let empty = HashMap::new();
        let aggregate = group_report(context.conversation, context.messages, None, &[], &empty)?;
        write_payload(&payload_group(context.conversation, &aggregate), path)
    } else {
        write_dual_report(context, path)
    }
}

pub fn run(
    kind: ReportKind,
    options: &ValidatedCli,
    store: &Store,
    contacts: &HashMap<String, Contact>,
    conversations: &[Conversation],
    contact_db: Option<&Path>,
    account_root: Option<&Path>,
    db_storage: Option<&Path>,
    self_username: Option<&str>,
    account_id: Option<&str>,
) -> Result<()> {
    let output_dir = options
        .output_dir
        .as_ref()
        .context("report output directory is missing")?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut progress = Progress::new(options.silent);
    let year = options.report_year;
    let (payload, file_name, report_type, conversation_id) = match kind {
        ReportKind::Private => {
            let aggregate = private_report(options, store, contacts, conversations, self_username, &mut progress)?;
            (payload_private(&aggregate), "private-report.html", "private", None)
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
            progress.set_phase(format!("读取群聊消息 · {}", conversation.name));
            progress.steps(0, 1, "任务");
            let (start, end) = year_bounds(year);
            let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
            if messages.is_empty() {
                bail!("no messages available for group {}", id);
            }
            progress.set_phase(format!("分析群聊 · {} 条消息", messages.len()));
            let member_ids = crate::wechat::contact::load_group_members(
                contact_db,
                &conversation.id,
                options.key.as_ref(),
                options.plaintext,
            );
            let aggregate = group_report(conversation, &messages, account_id, &member_ids, contacts)?;
            (payload_group(conversation, &aggregate), "group-report.html", "group", Some(conversation.id.clone()))
        }
        ReportKind::Annual => {
            let stats = annual_report(
                year,
                store,
                contacts,
                conversations,
                self_username,
                account_root,
                db_storage,
                options.key.as_ref(),
                options.plaintext,
                &mut progress,
            )?;
            (payload_annual(year, &stats), "annual-report.html", "annual", None)
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
            progress.set_phase(format!("读取双人消息 · {}", conversation.name));
            progress.steps(0, 1, "任务");
            let (start, end) = year_bounds(year);
            let messages = load_messages(store, conversation, contacts, self_username, start, end)?;
            if messages.is_empty() {
                bail!("no messages available for conversation {}", id);
            }
            progress.set_phase(format!("生成双人报告 · {} 条消息", messages.len()));
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
            write_dual_report(&context, &path)?;
            progress.finish();
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
    progress.set_phase("生成报告");
    let path = output_dir.join(file_name);
    write_payload(&payload, &path)?;
    progress.finish();
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
    let mut progress = Progress::new(options.silent);
    progress.set_phase("读取导出 JSON");
    progress.steps(0, 1, "任务");
    let loaded = filter_year(load_json_path(path)?, options.report_year);
    if loaded.is_empty() {
        bail!("no messages available in the selected JSON for year {}", options.report_year);
    }
    progress.set_phase("生成报告");
    match kind {
        ReportKind::Private => {
            let aggregate = aggregate_loaded(&loaded, false, true);
            if aggregate.total == 0 {
                bail!("no private conversation messages found in JSON");
            }
            let path = output_dir.join("private-report.html");
            write_payload(&payload_private(&aggregate), &path)?;
            progress.finish();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"private","report":path}))?);
        }
        ReportKind::Group => {
            let item = select_loaded(&loaded, options.conversation_ids.first(), true)?;
            if !item.conversation.is_group {
                bail!("group report requires a group conversation");
            }
            let empty = HashMap::new();
            let aggregate = group_report(&item.conversation, &item.messages, None, &[], &empty)?;
            let path = output_dir.join("group-report.html");
            write_payload(&payload_group(&item.conversation, &aggregate), &path)?;
            progress.finish();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"group","report":path,"conversation_id":item.conversation.id}))?);
        }
        ReportKind::Annual => {
            let mut stats = AnnualStats::new();
            for item in &loaded {
                if is_private_conversation(&item.conversation) {
                    stats.observe_conversation(&item.conversation, &item.messages);
                }
            }
            stats.sync_aggregate_friends();
            if stats.aggregate.total == 0 {
                bail!("no private messages found in JSON");
            }
            let path = output_dir.join("annual-report.html");
            write_payload(&payload_annual(options.report_year, &stats), &path)?;
            progress.finish();
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
            write_dual_report(&context, &path)?;
            progress.finish();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"type":"dual","report":path,"conversation_id":item.conversation.id}))?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod report_filter_tests {
    use super::is_word_text;

    #[test]
    fn rejects_metadata_and_code_tokens() {
        assert!(!is_word_text("wxid_85da986onp4h22:6:0"));
        assert!(!is_word_text("https://example.com/a"));
        assert!(!is_word_text("$ErrorActionPreference = 'Stop'"));
        assert!(!is_word_text("123456"));
        assert!(is_word_text("你手边有没有实物，让我看看"));
        assert!(is_word_text("面包板不是pcb的替代吗"));
    }
}

#[cfg(test)]
mod moments_tests {
    use super::load_moments;
    use std::fs;

    #[test]
    fn reads_self_posts_and_likes_from_sns_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let sns = dir.path().join("sns");
        fs::create_dir_all(&sns).unwrap();
        let db = rusqlite::Connection::open(sns.join("sns.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE SnsTimeLine (user_name TEXT, content TEXT);
             INSERT INTO SnsTimeLine VALUES ('me', '<timelineObject><createTime>1700000000</createTime><contentType>1</contentType><likeList><like><username>friend</username></like></likeList></timelineObject>');
             INSERT INTO SnsTimeLine VALUES ('friend', '<timelineObject><createTime>1700000001</createTime><contentType>2</contentType><likeList><like><username>me</username></like></likeList></timelineObject>');",
        )
        .unwrap();
        drop(db);
        let stats = load_moments(Some(dir.path()), None, None, true, Some("me"), 0).expect("moments");
        assert_eq!(stats.posts, 1);
        assert_eq!(stats.likes_received, 1);
        assert_eq!(stats.likes_given, 1);
        assert_eq!(stats.top_likers.get("friend"), Some(&1));
        assert_eq!(stats.top_liked_authors.get("friend"), Some(&1));
    }
}
