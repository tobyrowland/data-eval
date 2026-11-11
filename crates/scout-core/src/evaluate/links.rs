//! Work episodes: records from different systems joined by the references
//! they share (ticket keys, links to pull requests and documents, commit
//! hashes, email threads, shared files).
//!
//! A record is one unit of work in one system: a Slack thread, an email
//! thread, a ticket, a commit, a document, a CI log. Records that share a
//! strong key are joined into an episode: intent → discussion → action →
//! outcome. Keys are hashed with a salt chosen for each evaluation and
//! never leave this module; only counts do.

use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

/// What a record contributes to a piece of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A request: a ticket being opened.
    Intent = 1,
    /// People talking about it: chat, email.
    Discussion = 2,
    /// Work done: a commit, a document.
    Action = 4,
    /// The result: a CI run, a ticket resolved.
    Outcome = 8,
}

/// A key found in a text, with the system it points to.
#[derive(Debug, Clone, Default)]
pub struct Found {
    /// (key, system the key belongs to).
    refs: Vec<(u64, &'static str)>,
    /// Ticket-like keys (prefix, key): kept only if the prefix proves to
    /// be a real ticket project.
    tickets: Vec<(u64, u64)>,
    /// Ticket prefixes seen in a ticket tracker's own link.
    confirmed: Vec<(u64, &'static str)>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty() && self.tickets.is_empty()
    }
    pub fn extend(&mut self, o: Found) {
        self.refs.extend(o.refs);
        self.tickets.extend(o.tickets);
        self.confirmed.extend(o.confirmed);
    }
}

#[derive(Debug, Default)]
struct Rec {
    system: &'static str,
    roles: u8,
    year: Option<i32>,
    tokens: u64,
    refs: Vec<u64>,
    tickets: Vec<(u64, u64)>,
    owns: Vec<u64>,
}

/// Collects records during the evaluation pass.
pub struct Links {
    salt: u64,
    units: HashMap<u64, Rec>,
    key_system: HashMap<u64, &'static str>,
    /// Distinct numbers seen per ticket prefix (up to 3).
    prefix_numbers: HashMap<u64, Vec<u64>>,
    prefix_system: HashMap<u64, &'static str>,
    ci_passed: u64,
    ci_failed: u64,
    ci_logs: u64,
    commits: u64,
    tickets: u64,
}

impl Default for Links {
    fn default() -> Self {
        Links {
            salt: rand::random(),
            units: HashMap::new(),
            key_system: HashMap::new(),
            prefix_numbers: HashMap::new(),
            prefix_system: HashMap::new(),
            ci_passed: 0,
            ci_failed: 0,
            ci_logs: 0,
            commits: 0,
            tickets: 0,
        }
    }
}

const MAX_UNITS: usize = 5_000_000;
/// A key shared by more records than this (a pinned epic, a wiki home
/// page) says little about any one piece of work: it does not join.
const HUB: usize = 50;
/// A record citing more keys than this (a digest, a stand-up summary)
/// does not join the work it lists.
const DIGEST: usize = 8;
/// Groups larger than this are clusters, not episodes.
const CLUSTER: usize = 200;

impl Links {
    pub fn hash(&self, parts: &[&str]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.salt.hash(&mut h);
        for p in parts {
            p.hash(&mut h);
        }
        h.finish()
    }

    /// Adds to (or creates) the record for a unit of work.
    pub fn record(
        &mut self,
        unit: u64,
        system: &'static str,
        role: Role,
        year: Option<i32>,
        tokens: u64,
    ) {
        if self.units.len() >= MAX_UNITS && !self.units.contains_key(&unit) {
            return;
        }
        let r = self.units.entry(unit).or_default();
        if r.system.is_empty() {
            r.system = system;
        }
        r.roles |= role as u8;
        if r.year.is_none() {
            r.year = year;
        }
        r.tokens += tokens;
    }

    pub fn has(&self, unit: u64) -> bool {
        self.units.contains_key(&unit)
    }

    /// Adds the keys found in a text to a record (which must exist).
    pub fn add_found(&mut self, unit: u64, f: Found) {
        for (p, sys) in &f.confirmed {
            self.prefix_system.insert(*p, sys);
            let n = self.prefix_numbers.entry(*p).or_default();
            while n.len() < 3 {
                n.push(u64::MAX - n.len() as u64);
            }
        }
        for (p, k) in &f.tickets {
            let n = self.prefix_numbers.entry(*p).or_default();
            if n.len() < 3 && !n.contains(k) {
                n.push(*k);
            }
        }
        for (k, sys) in &f.refs {
            self.key_system.entry(*k).or_insert(sys);
        }
        if let Some(r) = self.units.get_mut(&unit) {
            for (k, _) in f.refs {
                if !r.refs.contains(&k) && r.refs.len() < 64 {
                    r.refs.push(k);
                }
            }
            for t in f.tickets {
                if !r.tickets.contains(&t) && r.tickets.len() < 64 {
                    r.tickets.push(t);
                }
            }
        }
    }

    /// A reference to a known key (a shared file, a commit).
    pub fn add_ref(&mut self, unit: u64, key: u64, system: &'static str) {
        self.key_system.entry(key).or_insert(system);
        if let Some(r) = self.units.get_mut(&unit) {
            if !r.refs.contains(&key) && r.refs.len() < 64 {
                r.refs.push(key);
            }
        }
    }

    /// A key this record *is* (the ticket a ticket row describes, the
    /// commit a commit is).
    pub fn own(&mut self, unit: u64, key: u64, system: &'static str) {
        self.key_system.entry(key).or_insert(system);
        if let Some(r) = self.units.get_mut(&unit) {
            if !r.owns.contains(&key) {
                r.owns.push(key);
            }
        }
    }

    /// A ticket record owns its key and proves the prefix is a project.
    pub fn own_ticket(&mut self, unit: u64, key: &str, system: &'static str) {
        let Some((prefix, _)) = key.split_once('-') else {
            return;
        };
        let p = self.hash(&["tp", prefix]);
        self.prefix_system.insert(p, system);
        let n = self.prefix_numbers.entry(p).or_default();
        while n.len() < 3 {
            n.push(u64::MAX - n.len() as u64);
        }
        let k = self.hash(&["t", key]);
        self.own(unit, k, system);
    }

    pub fn note_commit(&mut self) {
        self.commits += 1;
    }
    pub fn note_ticket(&mut self) {
        self.tickets += 1;
    }
    pub fn note_ci(&mut self, passed: Option<bool>) {
        self.ci_logs += 1;
        match passed {
            Some(true) => self.ci_passed += 1,
            Some(false) => self.ci_failed += 1,
            None => {}
        }
    }

    /// Finds the keys in a piece of text.
    pub fn find(&self, text: &str) -> Found {
        let mut f = Found::default();
        if text.contains("http") {
            for c in url_re().captures_iter(text) {
                let host = c[1].trim_start_matches("www.").to_ascii_lowercase();
                let path = c.get(2).map_or("", |m| m.as_str());
                self.url_key(&host, path, &mut f);
            }
        }
        if text.bytes().any(|b| b == b'-') {
            for m in ticket_re().captures_iter(text) {
                let whole = m.get(0).unwrap();
                let prefix = &m[1];
                if BLOCKED_PREFIXES.contains(&prefix) {
                    continue;
                }
                let before = text[..whole.start()].chars().next_back();
                if matches!(before, Some('-' | '.' | '_' | '@' | '#' | '=' | '+')) {
                    continue;
                }
                let after = &text[whole.end()..];
                let mut ac = after.chars();
                match (ac.next(), ac.next()) {
                    (Some('.'), Some(d)) if d.is_ascii_digit() => continue,
                    (Some(c), _) if c.is_alphanumeric() || c == '_' => continue,
                    _ => {}
                }
                let key = format!("{prefix}-{}", &m[2]);
                f.tickets
                    .push((self.hash(&["tp", prefix]), self.hash(&["t", &key])));
            }
        }
        f
    }

    fn url_key(&self, host: &str, path: &str, f: &mut Found) {
        let Some(system) = crate::analyze::sources::classify(host, path) else {
            return;
        };
        let path = path.split(['?', '#']).next().unwrap_or("");
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let seg = |i: usize| segs.get(i).copied().unwrap_or("");
        let lower = |s: &str| s.to_ascii_lowercase();
        let pr = |repo: &str, n: &str| self.hash(&["pr", &lower(repo), n]);
        let mut add = |k: u64, sys: &'static str| f.refs.push((k, sys));
        match system {
            "github" => match seg(2) {
                "pull" | "issues" if is_num(seg(3)) => add(pr(seg(1), seg(3)), system),
                "commit" | "commits" if is_sha(seg(3)) => add(sha_key(self, seg(3)), system),
                _ => {}
            },
            "bitbucket" => match seg(2) {
                "pull-requests" if is_num(seg(3)) => add(pr(seg(1), seg(3)), system),
                "commits" if is_sha(seg(3)) => add(sha_key(self, seg(3)), system),
                "issues" if is_num(seg(3)) => {
                    add(self.hash(&["bbi", &lower(seg(1)), seg(3)]), system)
                }
                _ => {}
            },
            "gitlab" => {
                if let Some(i) = segs.iter().position(|s| *s == "-") {
                    let repo = if i > 0 { segs[i - 1] } else { "" };
                    match seg(i + 1) {
                        "merge_requests" if is_num(seg(i + 2)) => add(pr(repo, seg(i + 2)), system),
                        "commit" | "commits" if is_sha(seg(i + 2)) => {
                            add(sha_key(self, seg(i + 2)), system)
                        }
                        "issues" if is_num(seg(i + 2)) => {
                            add(self.hash(&["gli", &lower(repo), seg(i + 2)]), system)
                        }
                        _ => {}
                    }
                }
            }
            "jira" | "linear" => {
                // …/browse/KEY-1, …/issue/KEY-1/title, ?selectedIssue=KEY-1
                let key = segs
                    .iter()
                    .zip(segs.iter().skip(1))
                    .find(|(a, _)| matches!(**a, "browse" | "issue"))
                    .map(|(_, k)| *k);
                if let Some(k) = key.filter(|k| is_ticket_key(k)) {
                    let prefix = k.split('-').next().unwrap_or("");
                    let p = self.hash(&["tp", prefix]);
                    f.confirmed.push((p, system));
                    f.tickets.push((p, self.hash(&["t", k])));
                }
            }
            "google_drive" => {
                if let Some(i) = segs.iter().position(|s| *s == "d") {
                    let id = seg(i + 1);
                    if id.len() >= 20 {
                        add(self.hash(&["gd", id]), system);
                    }
                }
            }
            "zendesk" => {
                if let Some(i) = segs.iter().position(|s| *s == "tickets") {
                    if is_num(seg(i + 1)) {
                        add(self.hash(&["zd", seg(i + 1)]), system);
                    }
                }
            }
            "sentry" => {
                if let Some(i) = segs.iter().position(|s| *s == "issues") {
                    if is_num(seg(i + 1)) {
                        add(self.hash(&["sentry", seg(i + 1)]), system);
                    }
                }
            }
            "confluence" => {
                if let Some(i) = segs.iter().position(|s| *s == "pages") {
                    if is_num(seg(i + 1)) {
                        add(self.hash(&["conf", seg(i + 1)]), system);
                    }
                }
            }
            "notion" => {
                let last = segs.last().copied().unwrap_or("");
                let id: String = last
                    .rsplit('-')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .filter(char::is_ascii_hexdigit)
                    .collect();
                if id.len() == 32 {
                    add(self.hash(&["notion", &id]), system);
                }
            }
            // Other known services: the same object link in two places.
            _ if segs.len() >= 2 => {
                let canon = format!("{host}/{}", segs.join("/").to_ascii_lowercase());
                add(self.hash(&["url", &canon]), system);
            }
            _ => {}
        }
    }

    /// Joins records into episodes and measures them.
    pub fn finish(self) -> Episodes {
        let Links {
            units,
            key_system,
            prefix_numbers,
            prefix_system,
            ci_passed,
            ci_failed,
            ci_logs,
            commits,
            tickets,
            ..
        } = self;
        let mut key_system = key_system;
        let real_prefix = |p: &u64| prefix_numbers.get(p).is_some_and(|n| n.len() >= 3);
        // Records with keys; ticket keys kept only for real projects, and
        // belong to the tracker their prefix was seen in (Jira otherwise).
        let mut recs: Vec<Rec> = units
            .into_values()
            .map(|mut r| {
                for (p, k) in std::mem::take(&mut r.tickets) {
                    if real_prefix(&p) && !r.refs.contains(&k) {
                        r.refs.push(k);
                        key_system
                            .entry(k)
                            .or_insert_with(|| prefix_system.get(&p).copied().unwrap_or("jira"));
                    }
                }
                r
            })
            .filter(|r| !r.refs.is_empty() || !r.owns.is_empty())
            .collect();
        recs.sort_by_key(|r| (r.system, r.year, r.tokens));
        // Who refers to and who owns each key.
        let mut users: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut owned: HashSet<u64> = HashSet::new();
        for (i, r) in recs.iter().enumerate() {
            for k in r.refs.iter().chain(&r.owns) {
                let u = users.entry(*k).or_default();
                if u.last() != Some(&i) {
                    u.push(i);
                }
            }
            owned.extend(r.owns.iter().copied());
        }
        // Union records that share a key (hubs and digests excepted).
        let mut parent: Vec<usize> = (0..recs.len()).collect();
        fn root(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for us in users.values() {
            if us.len() < 2 || us.len() > HUB {
                continue;
            }
            let joiners: Vec<usize> = us
                .iter()
                .copied()
                .filter(|&i| recs[i].refs.len() <= DIGEST || !recs[i].owns.is_empty())
                .collect();
            for w in joiners.windows(2) {
                let (a, b) = (root(&mut parent, w[0]), root(&mut parent, w[1]));
                if a != b {
                    parent[a] = b;
                }
            }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..recs.len() {
            let r = root(&mut parent, i);
            groups.entry(r).or_default().push(i);
        }
        let mut out = Episodes {
            ci_logs,
            ci_passed,
            ci_failed,
            commits,
            tickets,
            ..Default::default()
        };
        let mut pairs: BTreeMap<(&'static str, &'static str), u64> = BTreeMap::new();
        let mut per_system: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
        for r in &recs {
            per_system.entry(r.system).or_default().0 += 1;
        }
        for members in groups.values() {
            if members.len() < 2 {
                continue;
            }
            if members.len() > CLUSTER {
                out.clusters += 1;
                continue;
            }
            let mut roles = 0u8;
            let mut systems: Vec<&'static str> = Vec::new();
            let mut tokens = 0u64;
            let mut first_year: Option<i32> = None;
            for &i in members {
                let r = &recs[i];
                roles |= r.roles;
                if !systems.contains(&r.system) {
                    systems.push(r.system);
                }
                tokens += r.tokens;
                if let Some(y) = r.year {
                    first_year = Some(first_year.map_or(y, |f| f.min(y)));
                }
            }
            let role_count = roles.count_ones();
            if systems.len() < 2 && role_count < 2 {
                continue;
            }
            out.episodes += 1;
            out.records_linked += members.len() as u64;
            out.tokens += tokens;
            for &i in members {
                per_system.entry(recs[i].system).or_default().1 += 1;
            }
            let has = |r: Role| roles & r as u8 != 0;
            if has(Role::Action)
                && has(Role::Outcome)
                && (has(Role::Intent) || has(Role::Discussion))
            {
                out.complete += 1;
            } else if role_count >= 2 {
                out.partial += 1;
            } else {
                out.linked_discussion += 1;
            }
            if has(Role::Outcome) {
                out.with_outcome += 1;
            }
            if systems.len() >= 2 {
                out.multi_system += 1;
            }
            if first_year.is_some_and(|y| y < 2023) {
                out.pre_llm += 1;
            }
            let size = match members.len() {
                2 => 0,
                3..=5 => 1,
                6..=20 => 2,
                _ => 3,
            };
            out.by_size[size] += 1;
            systems.sort();
            for (a, x) in systems.iter().enumerate() {
                for y in &systems[a + 1..] {
                    *pairs.entry((x, y)).or_default() += 1;
                }
            }
        }
        let mut pairs: Vec<SystemPair> = pairs
            .into_iter()
            .map(|((a, b), n)| SystemPair {
                a,
                b,
                a_name: system_name(a),
                b_name: system_name(b),
                episodes: n,
            })
            .collect();
        pairs.sort_by(|x, y| y.episodes.cmp(&x.episodes).then(x.a.cmp(y.a)));
        pairs.truncate(10);
        out.system_pairs = pairs;
        out.systems = per_system
            .iter()
            .map(|(id, (records, linked))| SystemCount {
                id,
                name: system_name(id),
                records: *records,
                linked: *linked,
            })
            .collect();
        // Gaps: keys pointing at a system with no records at all.
        let present: HashSet<&'static str> = recs.iter().map(|r| r.system).collect();
        let present_git = present
            .iter()
            .any(|s| matches!(*s, "github" | "bitbucket" | "gitlab" | "git"));
        let mut gap_records: BTreeMap<&'static str, (HashSet<usize>, HashSet<u64>)> =
            BTreeMap::new();
        for (i, r) in recs.iter().enumerate() {
            for k in &r.refs {
                if owned.contains(k) {
                    continue;
                }
                let sys = key_system.get(k).copied().unwrap_or("jira");
                let missing = if matches!(sys, "github" | "bitbucket" | "gitlab" | "git") {
                    !present_git
                } else {
                    !present.contains(sys)
                };
                if !missing || NO_GAP.contains(&sys) {
                    continue;
                }
                let g = gap_records.entry(sys).or_default();
                g.0.insert(i);
                g.1.insert(*k);
            }
        }
        let mut gaps: Vec<Gap> = gap_records
            .into_iter()
            .map(|(id, (rs, ks))| Gap {
                id,
                name: system_name(id),
                records: rs.len() as u64,
                distinct_targets: ks.len() as u64,
                advice: advice(id),
            })
            .collect();
        if commits > 0 && ci_logs == 0 {
            gaps.push(Gap {
                id: "ci",
                name: system_name("ci"),
                records: commits,
                distinct_targets: 0,
                advice: advice("ci"),
            });
        }
        gaps.sort_by(|a, b| b.records.cmp(&a.records).then(a.id.cmp(b.id)));
        out.gaps = gaps;
        out
    }
}

/// Systems that are never suggested as gaps (no export gives them).
const NO_GAP: &[&str] = &["email", "slack", "teams", "discord", "documents"];

/// Ticket-like prefixes that are standards and units, not projects.
const BLOCKED_PREFIXES: &[&str] = &[
    "UTF", "ISO", "SHA", "RFC", "CVE", "COVID", "MP", "PEP", "HTTP", "TLS", "SSL", "GPT", "AES",
    "RSA", "MD", "IPV", "TCP", "UDP", "PCI", "GDPR", "SOC", "NIST", "WCAG", "ECMA", "ES", "API",
    "CSS", "HTML", "IEEE", "EN", "BS", "DIN", "ASTM", "USB", "HDMI", "LTE", "WPA", "PHP", "UTC",
    "GMT", "CET", "PST", "EST", "COV", "SARS", "H", "F", "MIG", "B", "A", "T", "X", "Q", "V",
];

fn is_num(s: &str) -> bool {
    !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn is_ticket_key(s: &str) -> bool {
    let Some((p, n)) = s.split_once('-') else {
        return false;
    };
    let pb = p.as_bytes();
    (2..=10).contains(&pb.len())
        && pb[0].is_ascii_uppercase()
        && pb
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && is_num(n)
}

/// A commit's key: its first seven hex digits, lower case.
pub fn sha_key(l: &Links, sha: &str) -> u64 {
    l.hash(&["sha", &sha[..7.min(sha.len())].to_ascii_lowercase()])
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"https?://([A-Za-z0-9.\-]+)(?::\d+)?(/[^\s<>|"'`)\]\\]*)?"#).unwrap()
    })
}

fn ticket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b([A-Z][A-Z0-9]{1,9})-([1-9][0-9]{0,5})\b").unwrap())
}

pub fn system_name(id: &str) -> String {
    match id {
        "git" => "Git history".into(),
        "ci" => "CI logs".into(),
        "documents" => "Documents".into(),
        _ => crate::analyze::sources::service(id)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| id.to_string()),
    }
}

/// What to export to fill a gap.
pub fn advice(id: &str) -> String {
    match id {
        "jira" => "Export your Jira issues as CSV (Filters › View all issues › Export › CSV, all fields).".into(),
        "linear" => "Export your Linear issues (Settings › Import / Export › Export CSV).".into(),
        "github" | "bitbucket" | "gitlab" | "git" => "Add the repository with its history: a git clone folder (with its .git folder), not a code download ZIP.".into(),
        "zendesk" => "Export your Zendesk tickets (Admin Center › Account › Tools › Reports › Export, CSV or JSON).".into(),
        "google_drive" => "Add Google Drive from Google Takeout (takeout.google.com).".into(),
        "ci" => "Add CI logs: in GitHub Actions open a run › ⋯ › Download log archive; in Bitbucket Pipelines download each step's log.".into(),
        "confluence" => "Export your Confluence spaces (Space settings › Export space › HTML).".into(),
        "notion" => "Export your Notion workspace (Settings › Export all workspace content, Markdown).".into(),
        _ => format!("Export your {} data.", system_name(id)),
    }
}

/// A CI log's measurements.
pub struct CiLog {
    pub passed: Option<bool>,
    pub shas: Vec<String>,
    pub year: Option<i32>,
}

/// Recognises a CI log (GitHub Actions, GitLab, Jenkins, Bitbucket
/// Pipelines) and reads whether it passed and which commit it built.
pub fn ci_log(text: &str) -> Option<CiLog> {
    let head = &text[..text.len().min(64 * 1024)];
    let gha = head.contains("##[") && gha_ts_re().is_match(head);
    let gitlab = head.contains("Running with gitlab-runner");
    let jenkins = head.contains("Started by ") && text.contains("Finished: ");
    let bitbucket = head.contains("+ umask 000") && head.contains("git clone");
    if !(gha || gitlab || jenkins || bitbucket) {
        return None;
    }
    let failed = text.contains("##[error]")
        || exit_re().is_match(text)
        || text.contains("ERROR: Job failed")
        || text.contains("Finished: FAILURE")
        || text.contains("Finished: UNSTABLE");
    let passed = text.contains("Job succeeded")
        || text.contains("Finished: SUCCESS")
        || (gha && text.contains("Cleaning up orphan processes"));
    let passed = if failed {
        Some(false)
    } else if passed || gha {
        Some(true)
    } else {
        None
    };
    let mut shas: Vec<String> = sha_re()
        .captures_iter(head)
        .map(|c| c[1].to_ascii_lowercase())
        .collect();
    shas.sort();
    shas.dedup();
    let year = head
        .get(..4)
        .and_then(|y| y.parse::<i32>().ok())
        .filter(|y| (2000..=2100).contains(y))
        .or_else(|| {
            gha_ts_re()
                .find(head)
                .and_then(|m| head[m.start()..m.start() + 4].parse().ok())
        });
    Some(CiLog { passed, shas, year })
}

fn gha_ts_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z ").unwrap())
}

fn exit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:Process completed with exit code|exit code:?) [1-9]").unwrap()
    })
}

fn sha_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:HEAD is now at|Checking out|git checkout (?:-f |--force |--progress --force )*|git reset --hard|Revision:?|Commit:?) ([0-9a-f]{7,40})\b").unwrap()
    })
}

/// A document title as a key: lower case, words only, without copy markers.
pub fn title_key(name: &str) -> Option<String> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = match file.rsplit_once('.') {
        Some((s, ext)) if ext.len() <= 5 && !s.is_empty() => s,
        _ => file,
    };
    let mut words: Vec<String> = stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    // "Copy of …", "… (1)", "… copy"
    if words.first().is_some_and(|w| w == "copy") && words.get(1).is_some_and(|w| w == "of") {
        words.drain(..2);
    }
    while words.last().is_some_and(|w| {
        w == "copy" || w == "final" || (w.len() <= 2 && w.bytes().all(|b| b.is_ascii_digit()))
    }) {
        words.pop();
    }
    let t = words.join(" ");
    let letters = t.chars().filter(|c| c.is_alphabetic()).count();
    if t.len() < 10 || letters < 6 || words.len() < 2 {
        return None;
    }
    const GENERIC: &[&str] = &[
        "untitled document",
        "untitled spreadsheet",
        "untitled presentation",
        "screen shot",
        "screenshot",
        "image",
        "scan",
        "new document",
        "meeting notes",
        "notes",
    ];
    if GENERIC.iter().any(|g| t.starts_with(g)) {
        return None;
    }
    Some(t)
}

/// Episodes found in the owner's data. Measurements only.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Episodes {
    /// Groups of two or more linked records spanning two or more systems
    /// or two or more roles.
    pub episodes: u64,
    /// Action, outcome and intent or discussion.
    pub complete: u64,
    /// Two or more roles, not complete.
    pub partial: u64,
    /// Linked conversations in several systems, no work recorded.
    pub linked_discussion: u64,
    pub with_outcome: u64,
    pub multi_system: u64,
    pub pre_llm: u64,
    pub records_linked: u64,
    pub tokens: u64,
    /// Episodes by records: 2, 3–5, 6–20, 21+.
    pub by_size: [u64; 4],
    /// Groups too large to be one piece of work (not counted as episodes).
    pub clusters: u64,
    pub system_pairs: Vec<SystemPair>,
    pub systems: Vec<SystemCount>,
    /// Missing systems that would link records, most first.
    pub gaps: Vec<Gap>,
    pub commits: u64,
    pub tickets: u64,
    pub ci_logs: u64,
    pub ci_passed: u64,
    pub ci_failed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemPair {
    pub a: &'static str,
    pub b: &'static str,
    pub a_name: String,
    pub b_name: String,
    pub episodes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemCount {
    pub id: &'static str,
    pub name: String,
    /// Records with at least one reference.
    pub records: u64,
    /// Of which in an episode.
    pub linked: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    pub id: &'static str,
    pub name: String,
    /// Records that would gain a link.
    pub records: u64,
    /// Distinct tickets, pages or commits they point to.
    pub distinct_targets: u64,
    pub advice: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tickets_links_and_filters_noise() {
        let l = Links::default();
        let f = l.find("Fixed in BUG-812 (see https://acme.atlassian.net/browse/BUG-812) and UTF-8, ISO-8601, v-1.2, x-BUG-9");
        let mut t = f.tickets.clone();
        t.sort();
        t.dedup();
        // BUG-812 (in text and link); not UTF-8, ISO-8601, v-1.2 or x-BUG-9.
        assert_eq!(t.len(), 1, "{f:?}");
        assert_eq!(f.confirmed.len(), 1);
        let f = l.find("PR https://bitbucket.org/acme/platform/pull-requests/12/overview and doc https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz/edit");
        assert_eq!(f.refs.len(), 2);
        assert_eq!(f.refs[0].1, "bitbucket");
        assert_eq!(f.refs[1].1, "google_drive");
    }

    #[test]
    fn titles() {
        assert_eq!(
            title_key("Takeout/Drive/Copy of Q3 Pricing Plan (1).docx").as_deref(),
            Some("q3 pricing plan")
        );
        assert_eq!(title_key("Untitled document.docx"), None);
        assert_eq!(title_key("a.pdf"), None);
    }

    #[test]
    fn ci_logs() {
        let gha = "2024-03-01T10:00:00.1234567Z ##[group]Run actions/checkout@v4\n2024-03-01T10:00:01.0000000Z HEAD is now at 1a2b3c4d fix\n2024-03-01T10:00:09.0000000Z ##[error]Process completed with exit code 1.\n";
        let c = ci_log(gha).unwrap();
        assert_eq!(c.passed, Some(false));
        assert_eq!(c.shas, vec!["1a2b3c4d"]);
        assert_eq!(c.year, Some(2024));
        assert!(ci_log("just some notes about ##[ things").is_none());
    }
}
