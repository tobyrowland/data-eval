//! Discord exports: the personal "Request your data" package
//! (`Messages/c<id>/channel.json` + `messages.json` or `messages.csv`,
//! `Servers/index.json`, `Account/user.json`) and whole-channel exports from
//! DiscordChatExporter (`{"guild", "channel", "messages": [...]}`).
//! Names and message text stay in this private state; only counts leave it.

use super::teams::SeenTeamsMessages as SeenChatMessages;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct DiscordInventory {
    /// Distinct messages in this item.
    pub messages: u64,
    pub servers: u64,
    pub channels: u64,
    pub direct_messages: u64,
    pub group_messages: u64,
    pub people: u64,
    /// Files linked from messages (Discord CDN), counted, never fetched.
    pub attachments_referenced: u64,
    /// A personal data package: only the requesting account's own messages.
    pub own_messages_only: bool,
    pub messages_also_in_other_items: u64,
    #[serde(skip)]
    pub overlap_by_collection: BTreeMap<u32, u64>,
    pub first_message_unix: Option<i64>,
    pub last_message_unix: Option<i64>,
    pub monthly_activity: BTreeMap<String, u64>,
}

#[derive(Default)]
struct Channel {
    kind: String,
    guild: Option<String>,
    recipients: Vec<String>,
}

#[derive(Default)]
pub struct DiscordRecogniser {
    keys: HashSet<(String, String)>,
    channels: BTreeMap<String, Channel>,
    servers: BTreeSet<String>,
    /// Server names ("PixPaint"). Local only.
    pub server_names: BTreeSet<String>,
    people: HashSet<String>,
    attachments: u64,
    package: bool,
    overlap: BTreeMap<u32, u64>,
    first: Option<i64>,
    last: Option<i64>,
    monthly: BTreeMap<String, u64>,
    pub languages: super::language::LanguageTally,
    pub sources: super::sources::SourceTally,
}

/// The channel folder of a package path: ".../Messages/c123/messages.json" -> "123".
fn package_channel(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').collect();
    let n = parts.len();
    if n >= 3 && parts[n - 3].eq_ignore_ascii_case("messages") {
        let c = parts[n - 2];
        if let Some(id) = c.strip_prefix('c') {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
                return Some(id);
            }
        }
    }
    None
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str))
}

impl DiscordRecogniser {
    pub fn is_discord(&self) -> bool {
        !self.keys.is_empty()
    }

    pub fn offer(&mut self, path: &str, doc: &Value, collection: u32, seen: &mut SeenChatMessages) {
        let file = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
        // Personal package.
        if let Some(cid) = package_channel(path) {
            match file.as_str() {
                "channel.json" => {
                    let ch = self.channels.entry(cid.to_string()).or_default();
                    ch.kind = str_of(doc, &["type"]).unwrap_or("").to_string();
                    if let Some(g) = doc
                        .get("guild")
                        .and_then(|g| g.get("id"))
                        .and_then(Value::as_str)
                    {
                        ch.guild = Some(g.to_string());
                        if let Some(n) = doc
                            .get("guild")
                            .and_then(|g| g.get("name"))
                            .and_then(Value::as_str)
                        {
                            self.server_names.insert(n.to_string());
                        }
                        self.servers.insert(g.to_string());
                    }
                    ch.recipients = doc
                        .get("recipients")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                }
                "messages.json" | "messages.csv" => {
                    let Some(rows) = doc.as_array() else { return };
                    self.package = true;
                    self.channels.entry(cid.to_string()).or_default();
                    for m in rows {
                        let Some(id) = str_of(m, &["ID", "id"]) else {
                            continue;
                        };
                        let text = str_of(m, &["Contents", "content"]).unwrap_or("");
                        let time = str_of(m, &["Timestamp", "timestamp"]);
                        let attach = str_of(m, &["Attachments", "attachments"]).unwrap_or("");
                        let n_attach = attach
                            .split_whitespace()
                            .filter(|s| s.starts_with("http"))
                            .count() as u64;
                        self.add(cid, id, None, time, text, n_attach, collection, seen);
                    }
                }
                _ => {}
            }
            return;
        }
        if file == "index.json" && path.to_ascii_lowercase().contains("servers/") {
            if let Some(o) = doc.as_object() {
                self.servers.extend(o.keys().cloned());
                self.server_names
                    .extend(o.values().filter_map(|v| v.as_str().map(String::from)));
            }
            return;
        }
        // DiscordChatExporter: {"guild": {...}, "channel": {...}, "messages": [...]}
        if let (Some(ch), Some(msgs)) = (
            doc.get("channel"),
            doc.get("messages").and_then(Value::as_array),
        ) {
            let cid = str_of(ch, &["id"]).unwrap_or("").to_string();
            if cid.is_empty() {
                return;
            }
            let entry = self.channels.entry(cid.clone()).or_default();
            entry.kind = str_of(ch, &["type"]).unwrap_or("GuildTextChat").to_string();
            if let Some(g) = doc
                .get("guild")
                .and_then(|g| g.get("id"))
                .and_then(Value::as_str)
            {
                entry.guild = Some(g.to_string());
                self.servers.insert(g.to_string());
            }
            for m in msgs {
                let Some(id) = str_of(m, &["id"]) else {
                    continue;
                };
                let author = m.get("author");
                let who = author.and_then(|a| str_of(a, &["id", "name"]));
                let text = str_of(m, &["content"]).unwrap_or("");
                let time = str_of(m, &["timestamp"]);
                let n_attach = m
                    .get("attachments")
                    .and_then(Value::as_array)
                    .map_or(0, |a| a.len() as u64);
                self.add(&cid, id, who, time, text, n_attach, collection, seen);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        channel: &str,
        id: &str,
        author: Option<&str>,
        time: Option<&str>,
        text: &str,
        attachments: u64,
        collection: u32,
        seen: &mut SeenChatMessages,
    ) {
        let key = (format!("discord:{channel}"), id.to_string());
        if !self.keys.insert(key.clone()) {
            return;
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
        if let Some(a) = author {
            self.people.insert(a.to_string());
        }
        self.attachments += attachments;
        if let Some(t) = time.and_then(super::teams::parse_time) {
            self.first = Some(self.first.map_or(t, |f| f.min(t)));
            self.last = Some(self.last.map_or(t, |l| l.max(t)));
            if first_seen {
                *self.monthly.entry(super::slack::month_key(t)).or_default() += 1;
            }
        }
        if first_seen && !text.is_empty() {
            self.sources.scan_text(text, 0);
            self.languages
                .add_segment(&super::language::strip_chat_code(text));
        }
    }

    pub fn inventory(&self) -> Option<DiscordInventory> {
        if !self.is_discord() {
            return None;
        }
        let mut people: HashSet<&str> = self.people.iter().map(String::as_str).collect();
        for ch in self.channels.values() {
            people.extend(ch.recipients.iter().map(String::as_str));
        }
        let kind = |k: &str| {
            self.channels
                .values()
                .filter(|c| c.kind.eq_ignore_ascii_case(k))
                .count() as u64
        };
        let dms = kind("DM") + kind("DirectTextChat");
        let groups = kind("GROUP_DM") + kind("DirectGroupTextChat");
        Some(DiscordInventory {
            messages: self.keys.len() as u64,
            servers: self.servers.len() as u64,
            channels: self.channels.len() as u64 - dms - groups,
            direct_messages: dms,
            group_messages: groups,
            // A package only names the owner indirectly; count them once.
            people: (people.len() as u64).max(u64::from(self.package)),
            attachments_referenced: self.attachments,
            own_messages_only: self.package,
            messages_also_in_other_items: self.overlap.values().sum(),
            overlap_by_collection: self.overlap.clone(),
            first_message_unix: self.first,
            last_message_unix: self.last,
            monthly_activity: self.monthly.clone(),
        })
    }

    /// Server (guild) IDs in this export. Local only.
    pub fn servers(&self) -> &BTreeSet<String> {
        &self.servers
    }

    pub fn overlaps_with(&self) -> BTreeSet<u32> {
        self.overlap.keys().copied().collect()
    }

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

    #[test]
    fn personal_package() {
        let mut r = DiscordRecogniser::default();
        let mut seen = SeenChatMessages::default();
        r.offer("p/Messages/c1/channel.json", &json!({"id": "1", "type": "GUILD_TEXT", "name": "general", "guild": {"id": "G1", "name": "PixPaint"}}), 0, &mut seen);
        r.offer(
            "p/Messages/c2/channel.json",
            &json!({"id": "2", "type": "DM", "recipients": ["u1", "u2"]}),
            0,
            &mut seen,
        );
        r.offer(
            "p/Messages/c3/channel.json",
            &json!({"id": "3", "type": "GROUP_DM", "recipients": ["u1", "u2", "u3"]}),
            0,
            &mut seen,
        );
        let msgs = json!([
            {"ID": "10", "Timestamp": "2026-10-26 12:23:02", "Contents": "Ship the brush engine", "Attachments": ""},
            {"ID": "11", "Timestamp": "2026-10-27 09:00:00", "Contents": "see https://github.com/pixpaint/app/pull/4", "Attachments": "https://cdn.discordapp.com/a.png"}
        ]);
        r.offer("p/Messages/c1/messages.json", &msgs, 0, &mut seen);
        r.offer("p/Messages/c2/messages.csv", &json!([{"ID": "20", "Timestamp": "2026-10-28 10:00:00", "Contents": "hi", "Attachments": ""}]), 0, &mut seen);
        r.offer(
            "p/Servers/index.json",
            &json!({"G1": "PixPaint", "G2": "Preview"}),
            0,
            &mut seen,
        );
        let inv = r.inventory().unwrap();
        assert_eq!(
            (
                inv.messages,
                inv.servers,
                inv.channels,
                inv.direct_messages,
                inv.group_messages
            ),
            (3, 2, 1, 1, 1)
        );
        assert_eq!((inv.people, inv.attachments_referenced), (3, 1));
        assert!(inv.own_messages_only);
        // The same package added again counts as an overlap.
        let mut again = DiscordRecogniser::default();
        again.offer("q/Messages/c1/messages.json", &msgs, 1, &mut seen);
        assert_eq!(again.inventory().unwrap().messages_also_in_other_items, 2);
    }

    #[test]
    fn chat_exporter() {
        let mut r = DiscordRecogniser::default();
        let mut seen = SeenChatMessages::default();
        let doc = json!({"guild": {"id": "G1", "name": "PixPaint"}, "channel": {"id": "C1", "type": "GuildTextChat", "name": "general"},
        "messages": [
            {"id": "1", "timestamp": "2026-01-02T10:00:00+00:00", "content": "hello", "author": {"id": "a", "name": "ann"}, "attachments": []},
            {"id": "2", "timestamp": "2026-01-02T10:01:00+00:00", "content": "hi", "author": {"id": "b", "name": "bo"}, "attachments": [{"url": "x"}]}
        ]});
        r.offer("general.json", &doc, 0, &mut seen);
        let inv = r.inventory().unwrap();
        assert_eq!(
            (
                inv.messages,
                inv.people,
                inv.channels,
                inv.servers,
                inv.attachments_referenced
            ),
            (2, 2, 1, 1, 1)
        );
        assert!(!inv.own_messages_only);
    }
}
