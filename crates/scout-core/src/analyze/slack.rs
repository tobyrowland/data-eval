//! Slack workspace export recogniser (spec §4 support matrix, §13 fixture).
//!
//! Works from JSON documents already parsed by the generic JSON parser and
//! their logical paths. File download links are counted, never retrieved.
//! Channel names, user IDs and URLs stay inside this private local state;
//! only counts leave it through [`SlackInventory`].

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

/// Subtypes that represent membership or channel administration rather than
/// participant content.
const ADMIN_SUBTYPES: &[&str] = &[
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "channel_convert_to_private",
    "channel_convert_to_public",
    "group_join",
    "group_leave",
    "group_topic",
    "group_purpose",
    "group_name",
    "group_archive",
    "group_unarchive",
    "pinned_item",
    "unpinned_item",
    "bot_add",
    "bot_remove",
    "bot_enable",
    "bot_disable",
    "app_conversation_join",
    "tombstone",
];

/// Count-only Slack structural inventory, safe to show the owner and to
/// aggregate into a shared packet.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SlackInventory {
    pub message_records: u64,
    pub channels_in_metadata: u64,
    pub channel_folders_with_messages: u64,
    pub participant_message_records: u64,
    pub bot_integration_message_records: u64,
    pub administrative_records: u64,
    pub accounts_posting_participant_content: u64,
    pub user_accounts_in_metadata: u64,
    pub bot_accounts_in_metadata: u64,
    pub integration_log_records: u64,
    pub explicit_threads: u64,
    pub thread_replies: u64,
    pub replies_with_missing_parent: u64,
    pub distinct_file_references: u64,
    pub referenced_image_files: u64,
    pub file_records_marked_visible: u64,
    pub file_download_links_present: u64,
    /// Always zero: attachments in a Slack export are references only and
    /// the app never fetches them.
    pub file_binaries_retrieved: u64,
    pub distinct_bitbucket_pr_references: u64,
    pub distinct_github_pr_references: u64,
    pub first_message_unix: Option<i64>,
    pub last_message_unix: Option<i64>,
    /// Message records per calendar month (YYYY-MM), coarse by design.
    pub monthly_activity: BTreeMap<String, u64>,
}

#[derive(Default)]
struct ExportState {
    saw_channels_json: bool,
    saw_users_json: bool,
    channels_in_metadata: u64,
    users: u64,
    bots: u64,
    integration_logs: u64,
    message_records: u64,
    participant: u64,
    bot: u64,
    admin: u64,
    posters: HashSet<String>,
    channel_folders: HashSet<String>,
    message_keys: HashSet<(String, String)>,
    thread_parents: HashSet<(String, String)>,
    reply_parents: Vec<(String, String)>,
    replies: u64,
    file_ids: HashSet<String>,
    image_ids: HashSet<String>,
    visible_ids: HashSet<String>,
    link_ids: HashSet<String>,
    bitbucket_prs: BTreeSet<String>,
    github_prs: BTreeSet<String>,
    first: Option<i64>,
    last: Option<i64>,
    monthly: BTreeMap<String, u64>,
}

/// Accumulates Slack evidence across all JSON documents of one collection.
#[derive(Default)]
pub struct SlackRecogniser {
    exports: BTreeMap<String, ExportState>,
}

fn bitbucket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"bitbucket\.org/([\w.\-]+)/([\w.\-]+)/pull-requests/(\d+)").unwrap()
    })
}

fn github_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"github\.com/([\w.\-]+)/([\w.\-]+)/pull/(\d+)").unwrap())
}

fn is_day_file(name: &str) -> bool {
    let b = name.as_bytes();
    name.len() == 15
        && name.ends_with(".json")
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

impl SlackRecogniser {
    /// Offers one parsed JSON document with its logical path ('/'-separated).
    pub fn offer(&mut self, path: &str, doc: &Value) {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some((&file, dirs)) = parts.split_last() else {
            return;
        };
        match file {
            "channels.json" | "users.json" | "integration_logs.json" => {
                let root = dirs.join("/");
                let st = self.exports.entry(root).or_default();
                match file {
                    "channels.json" => {
                        st.saw_channels_json = true;
                        st.channels_in_metadata = doc.as_array().map_or(0, |a| a.len() as u64);
                    }
                    "users.json" => {
                        st.saw_users_json = true;
                        if let Some(a) = doc.as_array() {
                            st.users = a.len() as u64;
                            st.bots = a
                                .iter()
                                .filter(|u| {
                                    u.get("is_bot").and_then(Value::as_bool).unwrap_or(false)
                                })
                                .count() as u64;
                        }
                    }
                    _ => {
                        st.integration_logs = doc
                            .get("logs")
                            .and_then(Value::as_array)
                            .map_or(0, |a| a.len() as u64);
                    }
                }
            }
            day if is_day_file(day) && !dirs.is_empty() => {
                let Some(messages) = doc.as_array() else {
                    return;
                };
                let (&channel, root_dirs) = dirs.split_last().unwrap();
                let root = root_dirs.join("/");
                let st = self.exports.entry(root).or_default();
                st.channel_folders.insert(channel.to_string());
                for m in messages {
                    st.add_message(channel, m);
                }
            }
            _ => {}
        }
    }

    /// Count-only inventories for exports that look like real Slack exports
    /// (metadata file plus at least one day file).
    pub fn inventories(&self) -> Vec<SlackInventory> {
        self.exports
            .values()
            .filter(|s| (s.saw_channels_json || s.saw_users_json) && s.message_records > 0)
            .map(ExportState::inventory)
            .collect()
    }
}

impl ExportState {
    fn add_message(&mut self, channel: &str, m: &Value) {
        let Some(obj) = m.as_object() else { return };
        let ts = obj.get("ts").and_then(Value::as_str).unwrap_or("");
        if !ts.is_empty()
            && !self
                .message_keys
                .insert((channel.to_string(), ts.to_string()))
        {
            return; // duplicate record of the same message
        }
        self.message_records += 1;

        let subtype = obj.get("subtype").and_then(Value::as_str).unwrap_or("");
        if ADMIN_SUBTYPES.contains(&subtype) {
            self.admin += 1;
        } else if subtype == "bot_message" || obj.contains_key("bot_id") {
            self.bot += 1;
        } else {
            self.participant += 1;
            if let Some(u) = obj.get("user").and_then(Value::as_str) {
                self.posters.insert(u.to_string());
            }
        }

        if let Some(secs) = ts.split('.').next().and_then(|s| s.parse::<i64>().ok()) {
            self.first = Some(self.first.map_or(secs, |f| f.min(secs)));
            self.last = Some(self.last.map_or(secs, |l| l.max(secs)));
            *self.monthly.entry(month_key(secs)).or_default() += 1;
        }

        if let Some(tts) = obj.get("thread_ts").and_then(Value::as_str) {
            let key = (channel.to_string(), tts.to_string());
            if tts == ts {
                self.thread_parents.insert(key);
            } else {
                self.replies += 1;
                self.reply_parents.push(key);
            }
        }

        if let Some(files) = obj.get("files").and_then(Value::as_array) {
            for f in files {
                let Some(id) = f.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let id = id.to_string();
                self.file_ids.insert(id.clone());
                if f.get("mimetype")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.starts_with("image/"))
                {
                    self.image_ids.insert(id.clone());
                }
                let mode = f.get("mode").and_then(Value::as_str).unwrap_or("");
                if !matches!(mode, "hidden_by_limit" | "tombstone") {
                    self.visible_ids.insert(id.clone());
                }
                if f.get("url_private_download")
                    .and_then(Value::as_str)
                    .is_some_and(|u| !u.is_empty())
                {
                    self.link_ids.insert(id);
                }
            }
        }

        // PR references may appear in text, attachments or blocks. Scan the
        // serialised record locally; matches never leave this struct.
        let serialised = m.to_string();
        for c in bitbucket_re().captures_iter(&serialised) {
            self.bitbucket_prs
                .insert(format!("{}/{}/{}", &c[1], &c[2], &c[3]).to_ascii_lowercase());
        }
        for c in github_re().captures_iter(&serialised) {
            self.github_prs
                .insert(format!("{}/{}/{}", &c[1], &c[2], &c[3]).to_ascii_lowercase());
        }
    }

    fn inventory(&self) -> SlackInventory {
        let missing_parents = self
            .reply_parents
            .iter()
            .filter(|k| !self.message_keys.contains(*k))
            .count() as u64;
        let mut threads: HashSet<&(String, String)> = self.thread_parents.iter().collect();
        threads.extend(self.reply_parents.iter());
        SlackInventory {
            message_records: self.message_records,
            channels_in_metadata: self.channels_in_metadata,
            channel_folders_with_messages: self.channel_folders.len() as u64,
            participant_message_records: self.participant,
            bot_integration_message_records: self.bot,
            administrative_records: self.admin,
            accounts_posting_participant_content: self.posters.len() as u64,
            user_accounts_in_metadata: self.users,
            bot_accounts_in_metadata: self.bots,
            integration_log_records: self.integration_logs,
            explicit_threads: threads.len() as u64,
            thread_replies: self.replies,
            replies_with_missing_parent: missing_parents,
            distinct_file_references: self.file_ids.len() as u64,
            referenced_image_files: self.image_ids.len() as u64,
            file_records_marked_visible: self.visible_ids.len() as u64,
            file_download_links_present: self.link_ids.len() as u64,
            file_binaries_retrieved: 0,
            distinct_bitbucket_pr_references: self.bitbucket_prs.len() as u64,
            distinct_github_pr_references: self.github_prs.len() as u64,
            first_message_unix: self.first,
            last_message_unix: self.last,
            monthly_activity: self.monthly.clone(),
        }
    }
}

/// YYYY-MM for a Unix timestamp (UTC), without a date-time dependency.
pub fn month_key(secs: i64) -> String {
    let (y, m, _) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}")
}

/// Howard Hinnant's days-from-civil inverse.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates() {
        assert_eq!(month_key(0), "1970-01");
        assert_eq!(month_key(1_700_000_000), "2023-11");
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn classifies_messages() {
        let mut r = SlackRecogniser::default();
        r.offer("export/channels.json", &json!([{"name": "general"}]));
        r.offer(
            "export/general/2024-01-02.json",
            &json!([
                {"ts": "1704153600.000100", "user": "U1", "text": "see https://bitbucket.org/acme/app/pull-requests/12",
                 "thread_ts": "1704153600.000100", "reply_count": 1},
                {"ts": "1704153700.000100", "user": "U2", "thread_ts": "1704153600.000100", "text": "ok",
                 "files": [{"id": "F1", "mimetype": "image/png", "url_private_download": "https://files.slack.com/x?t=secret"}]},
                {"ts": "1704153800.000100", "subtype": "bot_message", "bot_id": "B1", "text": "build passed"},
                {"ts": "1704153900.000100", "subtype": "channel_join", "user": "U3"},
                {"ts": "1704154000.000100", "user": "U2", "thread_ts": "1704000000.000000", "text": "orphan"},
                {"ts": "1704153600.000100", "user": "U1", "text": "duplicate record"}
            ]),
        );
        let inv = &r.inventories()[0];
        assert_eq!(inv.message_records, 5);
        assert_eq!(inv.participant_message_records, 3);
        assert_eq!(inv.bot_integration_message_records, 1);
        assert_eq!(inv.administrative_records, 1);
        assert_eq!(inv.accounts_posting_participant_content, 2);
        assert_eq!(inv.explicit_threads, 2);
        assert_eq!(inv.replies_with_missing_parent, 1);
        assert_eq!(inv.referenced_image_files, 1);
        assert_eq!(inv.file_download_links_present, 1);
        assert_eq!(inv.file_binaries_retrieved, 0);
        assert_eq!(inv.distinct_bitbucket_pr_references, 1);
        assert_eq!(inv.monthly_activity.get("2024-01"), Some(&5));
    }
}
