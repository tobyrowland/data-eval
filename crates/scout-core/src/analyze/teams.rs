//! Microsoft Teams exports: Microsoft Graph JSON (eDiscovery tools, the
//! Teams Export API and tenant takeouts) and the personal "Export your data"
//! messages.json. Works from JSON already parsed by the generic parser.
//! Names and message text stay in this private state; only counts leave it.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Teams messages seen across an assessment, keyed by conversation and
/// message ID, with the collection that had each first. A chat stored in
/// both participants' files, or in two exports, counts once.
pub type SeenTeamsMessages = HashMap<(String, String), u32>;

/// Count-only summary of a Teams export, safe to show and share.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct TeamsInventory {
    /// Distinct messages in this item.
    pub messages: u64,
    pub messages_by_people: u64,
    pub bot_or_system_messages: u64,
    pub teams: u64,
    pub channels: u64,
    pub chats: u64,
    pub people: u64,
    pub meetings: u64,
    /// Messages also present in another item added earlier.
    pub messages_also_in_other_items: u64,
    #[serde(skip)]
    pub overlap_by_collection: BTreeMap<u32, u64>,
    pub first_message_unix: Option<i64>,
    pub last_message_unix: Option<i64>,
    /// Messages per month, leaving out ones counted in another item.
    pub monthly_activity: BTreeMap<String, u64>,
}

#[derive(Default)]
pub struct TeamsRecogniser {
    keys: HashSet<(String, String)>,
    by_people: u64,
    bot_or_system: u64,
    teams: BTreeSet<String>,
    channels: BTreeSet<String>,
    chats: BTreeSet<String>,
    people: HashSet<String>,
    meetings: HashSet<String>,
    overlap: BTreeMap<u32, u64>,
    first: Option<i64>,
    last: Option<i64>,
    monthly: BTreeMap<String, u64>,
    /// A PST that looks like a Teams eDiscovery export (counted, not read).
    pub pst_files: u64,
    pub languages: super::language::LanguageTally,
    pub sources: super::sources::SourceTally,
}

/// Seconds since 1970 from an ISO-8601 time such as 2026-10-30T11:39:00.918Z.
fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn is_chat_message(o: &serde_json::Map<String, Value>) -> bool {
    match o.get("@odata.type").and_then(Value::as_str) {
        Some(t) => t.ends_with("chatMessage"),
        None => {
            o.contains_key("messageType")
                && o.contains_key("createdDateTime")
                && o.contains_key("id")
        }
    }
}

impl TeamsRecogniser {
    pub fn is_teams(&self) -> bool {
        !self.keys.is_empty() || self.pst_files > 0
    }

    /// Offers one parsed JSON document with its logical path.
    pub fn offer(
        &mut self,
        path: &str,
        doc: &Value,
        collection: u32,
        seen: &mut SeenTeamsMessages,
    ) {
        let file = path.rsplit('/').next().unwrap_or("");
        // Microsoft Graph: {"@odata.context": ..., "value": [...]}
        if let Some(items) = doc.get("value").and_then(Value::as_array) {
            let lower = file.to_ascii_lowercase();
            for it in items {
                let Some(o) = it.as_object() else { continue };
                if is_chat_message(o) {
                    self.graph_message(o, collection, seen);
                } else if o.contains_key("iCalUId") || lower.starts_with("meetings_") {
                    if let Some(id) = o
                        .get("iCalUId")
                        .or_else(|| o.get("id"))
                        .and_then(Value::as_str)
                    {
                        self.meetings.insert(id.to_string());
                    }
                } else if lower.starts_with("channels_") {
                    if let Some(id) = o.get("id").and_then(Value::as_str) {
                        self.channels.insert(id.to_string());
                    }
                } else if lower == "teamslist" || o.get("resourceProvisioningOptions").is_some() {
                    if let Some(id) = o.get("id").and_then(Value::as_str) {
                        self.teams.insert(id.to_string());
                    }
                }
            }
            return;
        }
        // Personal export: {"conversations": [{"id", "MessageList": [...]}]}
        if let Some(convs) = doc.get("conversations").and_then(Value::as_array) {
            for c in convs {
                let cid = c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                for m in c
                    .get("MessageList")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let id = m.get("id").and_then(Value::as_str).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let from = m
                        .get("displayName")
                        .or_else(|| m.get("from"))
                        .and_then(Value::as_str);
                    let time = m.get("originalarrivaltime").and_then(Value::as_str);
                    let text = m.get("content").and_then(Value::as_str).unwrap_or("");
                    let kind = m.get("messagetype").and_then(Value::as_str).unwrap_or("");
                    let by_person = from.is_some()
                        && (kind.is_empty()
                            || kind.eq_ignore_ascii_case("Text")
                            || kind.eq_ignore_ascii_case("RichText/Html"));
                    if !cid.is_empty() {
                        self.chats.insert(cid.clone());
                    }
                    self.add(&cid, id, from, time, text, by_person, collection, seen);
                }
            }
        }
    }

    fn graph_message(
        &mut self,
        o: &serde_json::Map<String, Value>,
        collection: u32,
        seen: &mut SeenTeamsMessages,
    ) {
        let id = o.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            return;
        }
        let channel = o
            .get("channelIdentity")
            .and_then(|c| c.get("channelId"))
            .and_then(Value::as_str);
        let team = o
            .get("channelIdentity")
            .and_then(|c| c.get("teamId"))
            .and_then(Value::as_str);
        let chat = o.get("chatId").and_then(Value::as_str);
        let conv = channel.or(chat).unwrap_or("").to_string();
        if let Some(c) = channel {
            self.channels.insert(c.to_string());
        }
        if let Some(t) = team {
            self.teams.insert(t.to_string());
        }
        if let Some(c) = chat {
            self.chats.insert(c.to_string());
        }
        let user = o
            .get("from")
            .and_then(|f| f.get("user"))
            .filter(|u| !u.is_null());
        let from = user
            .and_then(|u| u.get("displayName"))
            .and_then(Value::as_str);
        if let Some(uid) = user.and_then(|u| u.get("id")).and_then(Value::as_str) {
            self.people.insert(uid.to_string());
        }
        let by_person =
            user.is_some() && o.get("messageType").and_then(Value::as_str) == Some("message");
        let time = o.get("createdDateTime").and_then(Value::as_str);
        let text = o
            .get("body")
            .and_then(|b| b.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("");
        self.add(&conv, id, from, time, text, by_person, collection, seen);
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        conv: &str,
        id: &str,
        from: Option<&str>,
        time: Option<&str>,
        text: &str,
        by_person: bool,
        collection: u32,
        seen: &mut SeenTeamsMessages,
    ) {
        let key = (conv.to_string(), id.to_string());
        if !self.keys.insert(key.clone()) {
            return; // the same message again in this item (e.g. both sides of a chat)
        }
        let first_seen = match seen.entry(key) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(collection);
                true
            }
            std::collections::hash_map::Entry::Occupied(o) => {
                *self.overlap.entry(*o.get()).or_default() += 1;
                false
            }
        };
        if let Some(n) = from {
            self.people.insert(format!("name:{n}"));
        }
        if by_person {
            self.by_people += 1;
        } else {
            self.bot_or_system += 1;
        }
        if let Some(t) = time.and_then(parse_time) {
            self.first = Some(self.first.map_or(t, |f| f.min(t)));
            self.last = Some(self.last.map_or(t, |l| l.max(t)));
            if first_seen {
                *self.monthly.entry(super::slack::month_key(t)).or_default() += 1;
            }
        }
        if first_seen {
            self.sources.scan_text(text, 0);
            if by_person && !text.is_empty() {
                let prose = super::language::strip_markup(text);
                self.languages
                    .add_segment(&super::language::strip_chat_code(&prose));
            }
        }
    }

    pub fn inventory(&self) -> Option<TeamsInventory> {
        if !self.is_teams() {
            return None;
        }
        // People are counted by ID where known, otherwise by name.
        let ids = self
            .people
            .iter()
            .filter(|p| !p.starts_with("name:"))
            .count();
        let names = self
            .people
            .iter()
            .filter(|p| p.starts_with("name:"))
            .count();
        Some(TeamsInventory {
            messages: self.keys.len() as u64,
            messages_by_people: self.by_people,
            bot_or_system_messages: self.bot_or_system,
            teams: self.teams.len() as u64,
            channels: self.channels.len() as u64,
            chats: self.chats.len() as u64,
            people: ids.max(names) as u64,
            meetings: self.meetings.len() as u64,
            messages_also_in_other_items: self.overlap.values().sum(),
            overlap_by_collection: self.overlap.clone(),
            first_message_unix: self.first,
            last_message_unix: self.last,
            monthly_activity: self.monthly.clone(),
        })
    }

    pub fn overlaps_with(&self) -> BTreeSet<u32> {
        self.overlap.keys().copied().collect()
    }

    /// Follows a re-numbering of collections (after some were removed).
    pub fn renumber_collections(&mut self, map: &HashMap<u32, u32>) {
        self.overlap = std::mem::take(&mut self.overlap)
            .into_iter()
            .filter_map(|(k, v)| map.get(&k).map(|&n| (n, v)))
            .collect();
        self.sources.renumber_collections(map);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(id: &str, chat: &str, who: &str, t: &str) -> Value {
        json!({"@odata.type": "#microsoft.graph.chatMessage", "id": id, "messageType": "message",
               "createdDateTime": t, "chatId": chat,
               "from": {"user": {"id": who, "displayName": who}},
               "body": {"contentType": "text", "content": "Let's ship the tile cache on Friday."}})
    }

    #[test]
    fn graph_chats_count_once_across_participants() {
        let mut r = TeamsRecogniser::default();
        let mut seen = SeenTeamsMessages::default();
        let a = json!({"@odata.context": "x", "value": [msg("1", "c1", "Ann", "2026-10-30T11:39:00.918Z"), msg("2", "c1", "Bo", "2026-11-01T09:00:00Z")]});
        let b = json!({"@odata.context": "x", "value": [msg("1", "c1", "Ann", "2026-10-30T11:39:00.918Z")]});
        r.offer("t/users/u1/messages_u1.json", &a, 0, &mut seen);
        r.offer("t/users/u2/messages_u2.json", &b, 0, &mut seen);
        let inv = r.inventory().unwrap();
        assert_eq!((inv.messages, inv.chats, inv.people), (2, 1, 2));
        assert_eq!(inv.messages_also_in_other_items, 0);
        assert_eq!(inv.monthly_activity.values().sum::<u64>(), 2);
        assert_eq!(parse_time("1970-01-02T00:00:00Z"), Some(86_400));
        // The same chat in another item counts as an overlap.
        let mut other = TeamsRecogniser::default();
        other.offer("u.json", &b, 1, &mut seen);
        assert_eq!(other.inventory().unwrap().messages_also_in_other_items, 1);
    }

    #[test]
    fn personal_export() {
        let mut r = TeamsRecogniser::default();
        let mut seen = SeenTeamsMessages::default();
        let doc = json!({"userId": "8:live:x", "conversations": [{"id": "19:abc", "MessageList": [
            {"id": "1", "displayName": "Ann", "originalarrivaltime": "2021-05-01T10:00:00.000Z", "messagetype": "Text", "content": "hello there"}]}]});
        r.offer("messages.json", &doc, 0, &mut seen);
        let inv = r.inventory().unwrap();
        assert_eq!((inv.messages, inv.messages_by_people, inv.chats), (1, 1, 1));
    }
}
