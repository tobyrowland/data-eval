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
    /// The tickets' plain keys (prefix, key hash, key), kept only in
    /// capture mode, for generating test data on this computer.
    ticket_text: Vec<(u64, u64, String)>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty() && self.tickets.is_empty()
    }
    pub fn extend(&mut self, o: Found) {
        self.refs.extend(o.refs);
        self.tickets.extend(o.tickets);
        self.confirmed.extend(o.confirmed);
        self.ticket_text.extend(o.ticket_text);
    }
}

#[derive(Debug, Default)]
struct Rec {
    system: &'static str,
    roles: u8,
    year: Option<i32>,
    /// When the record happened (unix seconds), if known.
    time: Option<i64>,
    tokens: u64,
    refs: Vec<u64>,
    tickets: Vec<(u64, u64)>,
    owns: Vec<u64>,
    /// Evidence for each type of work (see [`WORK_TYPES`]).
    cues: [u16; 7],
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
    test_tickets: u64,
    /// Keep the plain ticket keys mentioned, to generate test data locally
    /// (off for normal evaluations: then only hashes are kept).
    pub capture_tickets: bool,
    mentions: HashMap<u64, TicketMention>,
}

/// A ticket key mentioned in the owner's data (capture mode only; never
/// part of an evaluation's output).
#[derive(Debug, Clone)]
pub struct TicketMention {
    pub key: String,
    prefix: u64,
    pub mentions: u32,
    /// First and last mention (unix seconds), when the records had a time.
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub systems: Vec<&'static str>,
    /// Mentions known only by year (middle of that year), used when no
    /// mention has an exact time.
    approx: Option<(i64, i64)>,
}

impl TicketMention {
    #[doc(hidden)]
    pub fn for_test(
        key: &str,
        mentions: u32,
        first: Option<i64>,
        last: Option<i64>,
        systems: Vec<&'static str>,
    ) -> Self {
        TicketMention {
            key: key.to_string(),
            prefix: 0,
            mentions,
            first,
            last,
            systems,
            approx: None,
        }
    }
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
            test_tickets: 0,
            capture_tickets: false,
            mentions: HashMap::new(),
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

    /// When a record happened; the earliest time is kept.
    pub fn time(&mut self, unit: u64, t: Option<i64>) {
        if let (Some(r), Some(t)) = (self.units.get_mut(&unit), t) {
            r.time = Some(r.time.map_or(t, |x: i64| x.min(t)));
        }
    }

    pub fn has(&self, unit: u64) -> bool {
        self.units.contains_key(&unit)
    }

    /// Adds the keys found in a text to a record (which must exist).
    pub fn add_found(&mut self, unit: u64, mut f: Found) {
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
        if self.capture_tickets && !f.ticket_text.is_empty() {
            let (system, exact, mid_year) = self.units.get(&unit).map_or(("", None, None), |r| {
                // Without an exact time, the middle of the record's year.
                let mid_year = r.year.map(|y| days_from_civil(y as i64, 7, 1) * 86_400);
                (r.system, r.time, mid_year)
            });
            for (p, k, key) in std::mem::take(&mut f.ticket_text) {
                let m = self.mentions.entry(k).or_insert_with(|| TicketMention {
                    key,
                    prefix: p,
                    mentions: 0,
                    first: None,
                    last: None,
                    systems: Vec::new(),
                    approx: None,
                });
                m.mentions += 1;
                if let Some(t) = exact {
                    m.first = Some(m.first.map_or(t, |x| x.min(t)));
                    m.last = Some(m.last.map_or(t, |x| x.max(t)));
                } else if let Some(t) = mid_year {
                    m.approx = Some(m.approx.map_or((t, t), |(a, b)| (a.min(t), b.max(t))));
                }
                if !system.is_empty() && !m.systems.contains(&system) {
                    m.systems.push(system);
                }
            }
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
    /// Adds evidence of the type of work to a record (which must exist).
    pub fn add_cues(&mut self, unit: u64, cues: [u16; 7]) {
        if let Some(r) = self.units.get_mut(&unit) {
            for (a, b) in r.cues.iter_mut().zip(cues) {
                *a = a.saturating_add(b);
            }
        }
    }

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
    /// A ticket row marked as Working Minds test data.
    pub fn note_test_ticket(&mut self) {
        self.test_tickets += 1;
    }

    /// The ticket keys mentioned (capture mode), for real ticket projects
    /// only, most mentioned first.
    pub fn take_mentions(&mut self) -> Vec<TicketMention> {
        let real = |p: &u64| self.prefix_numbers.get(p).is_some_and(|n| n.len() >= 3);
        let mut v: Vec<TicketMention> = std::mem::take(&mut self.mentions)
            .into_values()
            .filter(|m| real(&m.prefix))
            .map(|mut m| {
                if m.first.is_none() {
                    if let Some((a, b)) = m.approx {
                        m.first = Some(a);
                        m.last = Some(b);
                    }
                }
                m
            })
            .collect();
        v.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.key.cmp(&b.key)));
        v
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
                let (p, k) = (self.hash(&["tp", prefix]), self.hash(&["t", &key]));
                f.tickets.push((p, k));
                if self.capture_tickets {
                    f.ticket_text.push((p, k, key));
                }
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
                    if self.capture_tickets {
                        f.ticket_text.push((p, self.hash(&["t", k]), k.to_string()));
                    }
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
            test_tickets,
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
            test_tickets,
            ..Default::default()
        };
        let mut pairs: BTreeMap<(&'static str, &'static str), u64> = BTreeMap::new();
        let mut per_system: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
        for r in &recs {
            per_system.entry(r.system).or_default().0 += 1;
        }
        // Systems referenced but not uploaded, and which keys point there.
        let present: HashSet<&'static str> = recs.iter().map(|r| r.system).collect();
        let present_git = present.iter().any(|s| is_git_system(s));
        let gap_of = |k: &u64| -> Option<&'static str> {
            if owned.contains(k) {
                return None;
            }
            let sys = key_system.get(k).copied().unwrap_or("jira");
            let missing = if is_git_system(sys) {
                !present_git
            } else {
                !present.contains(sys)
            };
            (missing && !NO_GAP.contains(&sys)).then_some(sys)
        };
        let ci_gap = commits > 0 && ci_logs == 0;
        // Per gap: (records, keys, pieces it would complete, pieces it would add).
        let mut gap_stats: BTreeMap<&'static str, GapTally> = BTreeMap::new();
        let mut shapes: BTreeMap<(Vec<&'static str>, u8), ShapeTally> = BTreeMap::new();
        // Per type of work: pieces, complete, [gold, silver, bronze], tokens.
        let mut by_type = [(0u64, 0u64, [0u64; 3], 0u64); 8];
        let complete_with = |roles: u8| {
            let has = |r: Role| roles & r as u8 != 0;
            has(Role::Action) && has(Role::Outcome) && (has(Role::Intent) || has(Role::Discussion))
        };
        for members in groups.values() {
            if members.len() > CLUSTER {
                if members.len() >= 2 {
                    out.clusters += 1;
                }
                continue;
            }
            let mut roles = 0u8;
            let mut systems: Vec<&'static str> = Vec::new();
            let mut tokens = 0u64;
            let mut first_year: Option<i32> = None;
            let mut has_commit = false;
            let mut touched: BTreeMap<&'static str, ()> = BTreeMap::new();
            let mut cues = [0u32; 7];
            for &i in members {
                let r = &recs[i];
                roles |= r.roles;
                for (a, b) in cues.iter_mut().zip(r.cues) {
                    *a += b as u32;
                }
                if r.system == "zendesk" {
                    cues[3] += 5;
                }
                if r.refs.iter().any(|k| {
                    matches!(
                        key_system.get(k).copied(),
                        Some("sentry" | "pagerduty" | "datadog")
                    )
                }) {
                    cues[2] += 3;
                }
                if !systems.contains(&r.system) {
                    systems.push(r.system);
                }
                tokens += r.tokens;
                if let Some(y) = r.year {
                    first_year = Some(first_year.map_or(y, |f| f.min(y)));
                }
                has_commit |= is_git_system(r.system) && r.roles & Role::Action as u8 != 0;
                for k in &r.refs {
                    if let Some(sys) = gap_of(k) {
                        // A hub (a page cited everywhere) still shows the
                        // gap, but says nothing about this piece of work.
                        if !users.get(k).is_some_and(|u| u.len() > HUB) {
                            touched.insert(sys, ());
                        }
                        let g = gap_stats.entry(sys).or_default();
                        g.0.insert(i);
                        g.1.insert(*k);
                    }
                }
            }
            let role_count = roles.count_ones();
            let is_episode = members.len() >= 2 && (systems.len() >= 2 || role_count >= 2);
            let complete = is_episode && complete_with(roles);
            // What each missing system would do for this group.
            let mut gains: Vec<(&'static str, u8)> =
                touched.keys().map(|s| (*s, gap_role(s))).collect();
            if ci_gap && has_commit {
                gains.push(("ci", Role::Outcome as u8));
            }
            for (sys, add) in gains {
                let g = gap_stats.entry(sys).or_default();
                let new_roles = roles | add;
                if !complete && complete_with(new_roles) {
                    g.2 += 1;
                }
                if !is_episode {
                    g.3 += 1;
                }
            }
            if !is_episode {
                continue;
            }
            out.episodes += 1;
            out.records_linked += members.len() as u64;
            out.tokens += tokens;
            for &i in members {
                per_system.entry(recs[i].system).or_default().1 += 1;
            }
            let has = |r: Role| roles & r as u8 != 0;
            if complete {
                out.complete += 1;
            } else if role_count >= 2 {
                out.partial += 1;
            } else {
                out.linked_discussion += 1;
            }
            if has(Role::Intent) || has(Role::Discussion) {
                out.with_why += 1;
            }
            if has(Role::Action) {
                out.with_action += 1;
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
            // Quality of this piece of work, 0-100, and its tier.
            let score = trajectory_score(
                roles,
                complete,
                members.len(),
                systems.len(),
                tokens,
                first_year.is_some_and(|y| y < 2023),
            );
            out.score_sum += score as u64;
            let tier = if score >= GOLD {
                &mut out.gold
            } else if score >= SILVER {
                &mut out.silver
            } else {
                &mut out.bronze
            };
            tier.pieces += 1;
            tier.tokens += tokens;
            let kind = classify_work(&cues);
            let bt = &mut by_type[kind];
            bt.0 += 1;
            bt.1 += u64::from(complete);
            bt.2[usize::from(score < GOLD) + usize::from(score < SILVER)] += 1;
            bt.3 += tokens;
            systems.sort();
            for (a, x) in systems.iter().enumerate() {
                for y in &systems[a + 1..] {
                    *pairs.entry((x, y)).or_default() += 1;
                }
            }
            let sh = shapes.entry((systems.clone(), roles)).or_default();
            sh.0 += 1;
            sh.1 += score as u64;
            sh.2[kind] += 1;
        }
        out.by_type = by_type
            .iter()
            .enumerate()
            .filter(|(_, t)| t.0 > 0)
            .map(|(i, t)| TypeCount {
                id: WORK_TYPES[i].0,
                name: WORK_TYPES[i].1,
                pieces: t.0,
                complete: t.1,
                gold: t.2[0],
                silver: t.2[1],
                bronze: t.2[2],
                tokens: t.3,
            })
            .collect();
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
        let mut shapes: Vec<Shape> = shapes
            .into_iter()
            .map(|((systems, roles), (n, score_sum, kinds))| Shape {
                systems: systems.iter().map(|s| system_name(s)).collect(),
                why: roles & (Role::Intent as u8 | Role::Discussion as u8) != 0,
                action: roles & Role::Action as u8 != 0,
                outcome: roles & Role::Outcome as u8 != 0,
                episodes: n,
                mean_score: (score_sum / n.max(1)) as u32,
                top_type: {
                    let i = (0..8)
                        .max_by_key(|&i| (kinds[i], std::cmp::Reverse(i)))
                        .unwrap_or(7);
                    WORK_TYPES[i].1
                },
            })
            .collect();
        shapes.sort_by(|a, b| b.episodes.cmp(&a.episodes).then(a.systems.cmp(&b.systems)));
        shapes.truncate(5);
        out.shapes = shapes;
        out.systems = per_system
            .iter()
            .map(|(id, (records, linked))| SystemCount {
                id,
                name: system_name(id),
                records: *records,
                linked: *linked,
            })
            .collect();
        let mut gaps: Vec<Gap> = gap_stats
            .into_iter()
            .map(|(id, (rs, ks, completes, adds))| Gap {
                id,
                name: system_name(id),
                records: if id == "ci" { commits } else { rs.len() as u64 },
                distinct_targets: ks.len() as u64,
                would_complete: completes,
                would_add: adds,
                adds: role_names(gap_role(id)),
                advice: advice(id),
            })
            .collect();
        // Most pieces completed first, then most added, then most records.
        gaps.sort_by(|a, b| {
            (b.would_complete, b.would_add, b.records)
                .cmp(&(a.would_complete, a.would_add, a.records))
                .then(a.id.cmp(b.id))
        });
        out.gaps = gaps;
        out
    }
}

/// Per kind of linked work: pieces, score sum, pieces per type of work.
type ShapeTally = (u64, u64, [u64; 8]);

/// Per missing system: records citing it, keys cited, pieces of work it
/// would complete, pieces it would add.
type GapTally = (HashSet<usize>, HashSet<u64>, u64, u64);

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn is_git_system(s: &str) -> bool {
    matches!(s, "github" | "bitbucket" | "gitlab" | "git")
}

/// The part of a piece of work a missing system would add: a ticket
/// tracker the request and (once resolved) the result, a repository the
/// code change, CI logs the result, documents the work, meetings the talk.
fn gap_role(sys: &str) -> u8 {
    match sys {
        "jira" | "linear" | "asana" | "trello" | "zendesk" | "intercom" => {
            Role::Intent as u8 | Role::Outcome as u8
        }
        "ci" | "circleci" | "sentry" | "datadog" | "pagerduty" => Role::Outcome as u8,
        s if is_git_system(s) => Role::Action as u8,
        "google_drive" | "confluence" | "notion" | "dropbox" | "box" | "figma" | "miro"
        | "microsoft365" | "airtable" => Role::Action as u8,
        _ => Role::Discussion as u8,
    }
}

fn role_names(roles: u8) -> Vec<&'static str> {
    let mut v = Vec::new();
    if roles & (Role::Intent as u8 | Role::Discussion as u8) != 0 {
        v.push("why");
    }
    if roles & Role::Action as u8 != 0 {
        v.push("what was done");
    }
    if roles & Role::Outcome as u8 != 0 {
        v.push("how it turned out");
    }
    v
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
    let mut cut = text.len().min(64 * 1024);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &text[..cut];
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
    /// Episodes with a request or discussion (why).
    pub with_why: u64,
    /// Episodes with work done (a commit, a document).
    pub with_action: u64,
    /// Episodes with a result (a CI run, a ticket resolved).
    pub with_outcome: u64,
    pub multi_system: u64,
    pub pre_llm: u64,
    pub records_linked: u64,
    pub tokens: u64,
    /// Episodes by records: 2, 3–5, 6–20, 21+.
    pub by_size: [u64; 4],
    /// Pieces of work by quality tier (see [`trajectory_score`]): gold 70+,
    /// silver 45-69, bronze below 45.
    pub gold: Tier,
    pub silver: Tier,
    pub bronze: Tier,
    /// Sum of the pieces' scores (mean = score_sum / episodes).
    pub score_sum: u64,
    /// Groups too large to be one piece of work (not counted as episodes).
    pub clusters: u64,
    pub system_pairs: Vec<SystemPair>,
    /// The most common kinds of episode: systems and parts present.
    pub shapes: Vec<Shape>,
    /// Pieces of work by type of work (see [`WORK_TYPES`]).
    pub by_type: Vec<TypeCount>,
    pub systems: Vec<SystemCount>,
    /// Missing systems that would link records, most first.
    pub gaps: Vec<Gap>,
    pub commits: u64,
    pub tickets: u64,
    /// Ticket rows marked as Working Minds test data (fake): if any, the
    /// owner must remove that file before sending a report.
    pub test_tickets: u64,
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

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tier {
    pub pieces: u64,
    pub tokens: u64,
}

/// Score a piece of linked work 0-100 for training value:
/// - completeness, up to 40: why, what was done and how it turned out
///   (complete 40; why + what 25; other two-part 15; linked talk only 5)
/// - steps, up to 20: records in the piece, 12 or more scoring full marks
/// - systems, up to 15: each system beyond the first adds 5, up to 3 more
/// - human reasoning, up to 15: tokens in the piece, log scale, 10,000+
/// - written before 2023: 10
pub fn trajectory_score(
    roles: u8,
    complete: bool,
    records: usize,
    systems: usize,
    tokens: u64,
    pre_llm: bool,
) -> u32 {
    let has = |r: Role| roles & r as u8 != 0;
    let why = has(Role::Intent) || has(Role::Discussion);
    let completeness = if complete {
        40.0
    } else if why && has(Role::Action) {
        25.0
    } else if roles.count_ones() >= 2 {
        15.0
    } else {
        5.0
    };
    let steps = (records.min(12) as f64 / 12.0) * 20.0;
    let systems = (systems.saturating_sub(1).min(3) as f64) * 5.0;
    let reasoning = ((tokens as f64 + 1.0).log10() / 4.0).min(1.0) * 15.0;
    let age = if pre_llm { 10.0 } else { 0.0 };
    (completeness + steps + systems + reasoning + age).round() as u32
}

pub const GOLD: u32 = 70;
pub const SILVER: u32 = 45;

#[derive(Debug, Clone, Serialize)]
pub struct Shape {
    pub systems: Vec<String>,
    pub why: bool,
    pub action: bool,
    pub outcome: bool,
    pub episodes: u64,
    /// Mean quality score (0-100) of these pieces of work.
    pub mean_score: u32,
    /// The most common type of work among them.
    pub top_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeCount {
    pub id: &'static str,
    pub name: &'static str,
    pub pieces: u64,
    pub complete: u64,
    pub gold: u64,
    pub silver: u64,
    pub bronze: u64,
    pub tokens: u64,
}

/// Types of work, in display order; the last is the fallback.
pub const WORK_TYPES: [(&str, &str); 8] = [
    ("bug_fix", "Bug fix"),
    ("feature", "Feature work"),
    ("incident", "Incident"),
    ("support", "Customer support"),
    ("sales", "Sales and deals"),
    ("planning", "Planning and decisions"),
    ("maintenance", "Maintenance"),
    ("mixed", "Mixed or unclear"),
];

const LEXICON: [&[&str]; 7] = [
    &[
        "bug",
        "bugs",
        "fix",
        "fixed",
        "fixes",
        "fixing",
        "crash",
        "crashes",
        "crashed",
        "error",
        "errors",
        "exception",
        "regression",
        "broken",
        "failing",
        "hotfix",
        "defect",
        "stacktrace",
    ],
    &[
        "feature",
        "features",
        "implement",
        "implemented",
        "implementing",
        "launch",
        "launched",
        "release",
        "released",
        "spec",
        "prototype",
        "mvp",
        "endpoint",
        "mockup",
        "wireframe",
    ],
    &[
        "outage",
        "incident",
        "incidents",
        "down",
        "alert",
        "alerts",
        "paged",
        "sev1",
        "sev2",
        "postmortem",
        "post-mortem",
        "rollback",
        "degraded",
        "downtime",
        "on-call",
    ],
    &[
        "customer",
        "customers",
        "client",
        "clients",
        "refund",
        "complaint",
        "complaints",
        "helpdesk",
        "escalation",
        "escalated",
    ],
    &[
        "deal",
        "deals",
        "contract",
        "contracts",
        "proposal",
        "pricing",
        "quote",
        "invoice",
        "prospect",
        "prospects",
        "renewal",
        "demo",
        "procurement",
    ],
    &[
        "roadmap",
        "plan",
        "planning",
        "decision",
        "decide",
        "decided",
        "priority",
        "priorities",
        "okr",
        "okrs",
        "sprint",
        "retro",
        "strategy",
        "agenda",
    ],
    &[
        "refactor",
        "refactoring",
        "cleanup",
        "upgrade",
        "upgraded",
        "bump",
        "dependency",
        "dependencies",
        "migrate",
        "migration",
        "chore",
        "docs",
        "lint",
        "tests",
    ],
];

/// Evidence of the type of work in a text: whole-word matches against
/// small English word lists, at most 5 per type so one long text can't
/// decide alone.
pub fn work_cues(text: &str) -> [u16; 7] {
    let mut out = [0u16; 7];
    for w in text
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| (2..=14).contains(&w.len()))
    {
        let w = w.trim_matches('-').to_lowercase();
        for (i, words) in LEXICON.iter().enumerate() {
            if out[i] < 5 && words.contains(&w.as_str()) {
                out[i] += 1;
            }
        }
    }
    out
}

/// A commit message's evidence: its words, plus the conventional prefix
/// (fix:, feat:, chore: …) counted strongly.
pub fn commit_cues(message: &str) -> [u16; 7] {
    let mut c = work_cues(message);
    let first = message.lines().next().unwrap_or("").trim().to_lowercase();
    let prefix: String = first
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect();
    let rest = &first[prefix.len()..];
    let conventional = rest.starts_with(':') || rest.starts_with('(') || rest.starts_with('!');
    let idx = match prefix.as_str() {
        "fix" | "bugfix" if conventional => Some(0),
        "hotfix" | "revert" => Some(0),
        "feat" | "feature" if conventional => Some(1),
        "chore" | "refactor" | "build" | "ci" | "docs" | "test" | "tests" | "perf" | "style"
            if conventional =>
        {
            Some(6)
        }
        _ => None,
    };
    if let Some(i) = idx {
        c[i] = c[i].saturating_add(3);
    }
    c
}

/// A ticket's evidence from its tracker type (Bug, Story, Incident …).
pub fn issue_type_cues(issue_type: &str) -> [u16; 7] {
    let mut c = [0u16; 7];
    match issue_type.trim().to_lowercase().as_str() {
        "bug" | "defect" => c[0] = 5,
        "story" | "feature" | "new feature" | "improvement" | "enhancement" => c[1] = 5,
        "incident" | "problem" | "outage" => c[2] = 5,
        "epic" | "initiative" => c[5] = 3,
        "technical debt" | "tech debt" | "chore" => c[6] = 5,
        "question" | "service request" => c[3] = 3,
        _ => {}
    }
    c
}

/// The type of a piece of work: the strongest evidence, if it is at least
/// 3 and at least 40% of all evidence; otherwise mixed (index 7).
pub fn classify_work(cues: &[u32; 7]) -> usize {
    let total: u32 = cues.iter().sum();
    let (i, &max) = cues
        .iter()
        .enumerate()
        .max_by_key(|(i, v)| (**v, std::cmp::Reverse(*i)))
        .unwrap_or((7, &0));
    if max >= 3 && max * 10 >= total * 4 {
        i
    } else {
        7
    }
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
    /// Existing pieces of work it would complete (if the cited items are
    /// in the export and, for tickets, resolved): an upper estimate.
    pub would_complete: u64,
    /// Records or groups that would become new pieces of linked work.
    pub would_add: u64,
    /// The parts it adds: "why", "what was done", "how it turned out".
    pub adds: Vec<&'static str>,
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
    fn trajectory_scores_reward_complete_long_cross_system_older_work() {
        let all =
            Role::Intent as u8 | Role::Discussion as u8 | Role::Action as u8 | Role::Outcome as u8;
        // A 12-step bug fix across 4 systems with plenty of discussion, 2021.
        assert_eq!(trajectory_score(all, true, 12, 4, 10_000, true), 100);
        // Two Slack threads about the same ticket, recent, short.
        let talk = Role::Discussion as u8;
        let s = trajectory_score(talk, false, 2, 1, 60, false);
        assert!(s < SILVER, "{s}");
        // Why + what, no result, a few systems: silver at most.
        let s = trajectory_score(talk | Role::Action as u8, false, 4, 3, 300, true);
        assert!((SILVER..GOLD).contains(&s), "{s}");
    }

    #[test]
    fn work_cues_find_types_by_whole_words() {
        let c = work_cues(
            "The export crashes with a null error; fixed in the hotfix. Fixture data downloaded.",
        );
        assert_eq!(c[0], 4, "{c:?}"); // crashes, error, fixed, hotfix (not fixture)
        assert_eq!(c[2], 0, "{c:?}"); // not 'down' in downloaded
        assert_eq!(classify_work(&c.map(u32::from)), 0);
        let c = commit_cues("feat(api): add the pricing endpoint");
        assert_eq!(classify_work(&c.map(u32::from)), 1);
        assert_eq!(commit_cues("chore: bump deps")[6], 5); // chore + bump words, +3 for the prefix
                                                           // Weak or split evidence is mixed.
        assert_eq!(classify_work(&[1, 1, 0, 0, 0, 0, 0]), 7);
        assert_eq!(classify_work(&[3, 3, 3, 0, 0, 0, 0]), 7);
        assert_eq!(issue_type_cues("Bug")[0], 5);
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
        // A long text with a multi-byte character across the 64 KB mark.
        let long = format!("{}é{}", "a".repeat(64 * 1024 - 1), "b".repeat(10));
        assert!(ci_log(&long).is_none());
    }
}
