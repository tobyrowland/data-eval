//! A local fingerprint of who and what an item is about: people, email
//! addresses, organisations, email and web domains, and distinctive names
//! (capitalised terms such as project, product and client names).
//!
//! Used only to tell whether an item seems connected to the rest of the
//! data. Everything is hashed and stays in the private inventory; only
//! counts of what is shared reach the data map.

use regex::Regex;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

/// Caps keep memory bounded on very large items.
const MAX_IDS: usize = 50_000;
const MAX_TERMS: usize = 20_000;

#[derive(Debug, Clone, Default)]
pub struct Fingerprint {
    pub people: HashSet<u64>,
    pub addresses: HashSet<u64>,
    /// Organisation-level identifiers: email and web domains that aren't
    /// public services, code-host owners ("bitbucket.org/acme"), tenant IDs.
    pub orgs: BTreeSet<String>,
    /// Distinctive capitalised terms, with how often they appear.
    pub terms: HashMap<u64, u32>,
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9._%+\-]{1,64}@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,24}")
            .unwrap()
    })
}

fn host_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[Hh][Tt][Tt][Pp][Ss]?://([A-Za-z0-9.\-]+)(?::[0-9]+)?(/[^\s<>|\x22'`)\]\\]*)?")
            .unwrap()
    })
}

/// Domains of public services, which say nothing about whose data it is.
pub fn is_public_domain(domain: &str) -> bool {
    let d = domain.trim_start_matches("www.").to_ascii_lowercase();
    const PUBLIC: &[&str] = &[
        "gmail.com",
        "googlemail.com",
        "outlook.com",
        "hotmail.com",
        "live.com",
        "msn.com",
        "yahoo.com",
        "icloud.com",
        "me.com",
        "mac.com",
        "aol.com",
        "proton.me",
        "protonmail.com",
        "gmx.com",
        "gmx.net",
        "mail.com",
        "yandex.com",
        "zoho.com",
        "freemail.hu",
        "citromail.hu",
        "google.com",
        "youtube.com",
        "youtu.be",
        "slack.com",
        "slack-edge.com",
        "github.com",
        "githubusercontent.com",
        "gitlab.com",
        "bitbucket.org",
        "atlassian.net",
        "atlassian.com",
        "microsoft.com",
        "office.com",
        "office365.com",
        "sharepoint.com",
        "live.net",
        "microsoftonline.com",
        "teams.microsoft.com",
        "skype.com",
        "zoom.us",
        "notion.so",
        "figma.com",
        "dropbox.com",
        "box.com",
        "apple.com",
        "amazon.com",
        "amazonaws.com",
        "facebook.com",
        "linkedin.com",
        "twitter.com",
        "x.com",
        "instagram.com",
        "wikipedia.org",
        "medium.com",
        "stackoverflow.com",
        "npmjs.com",
        "pypi.org",
        "crates.io",
        "example.com",
        "example.org",
        "w3.org",
        "schema.org",
        "gstatic.com",
        "googleapis.com",
        "loom.com",
        "calendly.com",
        "trello.com",
        "asana.com",
        "miro.com",
        "sentry.io",
        "circleci.com",
        "herokuapp.com",
        "vercel.app",
        "netlify.app",
        "cloudfront.net",
        "mailchimp.com",
        "sendgrid.net",
        "hubspot.com",
        "salesforce.com",
        "zendesk.com",
        "intercom.io",
    ];
    PUBLIC
        .iter()
        .any(|p| d == *p || d.ends_with(&format!(".{p}")))
}

/// Capitalised words that are common in any company's text, so they don't
/// show a connection: sentence openers, days, months, and the names of
/// widely used tools.
fn is_common_term(w: &str) -> bool {
    const COMMON: &[&str] = &[
        "the",
        "this",
        "that",
        "these",
        "those",
        "there",
        "then",
        "thanks",
        "thank",
        "hello",
        "please",
        "great",
        "sure",
        "okay",
        "yes",
        "also",
        "just",
        "what",
        "when",
        "where",
        "which",
        "while",
        "with",
        "will",
        "would",
        "could",
        "should",
        "have",
        "here",
        "from",
        "for",
        "and",
        "but",
        "not",
        "can",
        "let",
        "our",
        "your",
        "they",
        "she",
        "his",
        "her",
        "you",
        "all",
        "any",
        "one",
        "two",
        "new",
        "next",
        "last",
        "good",
        "nice",
        "cool",
        "done",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
        "today",
        "tomorrow",
        "yesterday",
        "english",
        "google",
        "slack",
        "teams",
        "microsoft",
        "zoom",
        "github",
        "gitlab",
        "bitbucket",
        "jira",
        "confluence",
        "figma",
        "notion",
        "drive",
        "docs",
        "sheets",
        "slides",
        "gmail",
        "outlook",
        "excel",
        "word",
        "powerpoint",
        "apple",
        "android",
        "windows",
        "linux",
        "chrome",
        "safari",
        "firefox",
        "azure",
        "amazon",
        "aws",
        "docker",
        "kubernetes",
        "python",
        "javascript",
        "typescript",
        "java",
        "react",
        "node",
        "sql",
        "json",
        "html",
        "css",
        "api",
        "http",
        "https",
        "url",
        "pdf",
        "linkedin",
        "twitter",
        "youtube",
        "facebook",
        "loom",
        "miro",
        "trello",
        "asana",
        "dropbox",
        "sentry",
        "datadog",
        "circleci",
        "hubspot",
        "salesforce",
        "zendesk",
        "intercom",
        "airtable",
        "pagerduty",
        "monday.com",
        "meeting",
        "call",
        "team",
        "channel",
        "general",
        "random",
        "message",
        "update",
        "review",
        "release",
        "subject",
        "re",
        "fwd",
        "fw",
        "cc",
        "bcc",
        "sent",
        "inc",
        "ltd",
        "llc",
        "gmbh",
        "kft",
    ];
    COMMON.contains(&w)
}

impl Fingerprint {
    pub fn add_person(&mut self, name: &str) {
        let n = name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        // A real name has at least two letters and isn't an address or ID.
        if n.chars().filter(|c| c.is_alphabetic()).count() < 2
            || n.contains('@')
            || self.people.len() >= MAX_IDS
        {
            return;
        }
        self.people.insert(hash(&n));
    }

    pub fn add_address(&mut self, addr: &str) {
        let a = addr
            .trim()
            .trim_matches(|c| c == '<' || c == '>')
            .to_ascii_lowercase();
        let Some((local, domain)) = a.rsplit_once('@') else {
            return;
        };
        if local.is_empty() || !domain.contains('.') {
            return;
        }
        if self.addresses.len() < MAX_IDS {
            self.addresses.insert(hash(&a));
        }
        self.add_domain(domain);
    }

    pub fn add_domain(&mut self, domain: &str) {
        let d = domain
            .trim_start_matches("www.")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if d.contains('.') && !is_public_domain(&d) && self.orgs.len() < MAX_IDS {
            // Count a subdomain as its organisation: mail.acme.com -> acme.com.
            let parts: Vec<&str> = d.split('.').collect();
            let keep = if parts.len() > 2 && parts[parts.len() - 2].len() <= 3 {
                3
            } else {
                2
            };
            let org = parts[parts.len().saturating_sub(keep)..].join(".");
            self.orgs.insert(org);
        }
    }

    pub fn add_org(&mut self, id: &str) {
        let id = id.trim().to_ascii_lowercase();
        if !id.is_empty() && self.orgs.len() < MAX_IDS {
            self.orgs.insert(format!("org:{id}"));
        }
    }

    /// Email addresses, organisation web domains, code-host owners and
    /// distinctive terms in free text.
    pub fn add_text(&mut self, text: &str) {
        if text.contains('@') {
            for m in email_re().find_iter(text) {
                self.add_address(m.as_str());
            }
        }
        if text.contains("http") {
            for c in host_re().captures_iter(text) {
                let host = c[1].to_ascii_lowercase();
                let path = c.get(2).map_or("", |m| m.as_str());
                // Whose repository it is says more than the code host.
                let code_host = ["github.com", "gitlab.com", "bitbucket.org"]
                    .iter()
                    .find(|h| host == **h || host.ends_with(&format!(".{h}")));
                if let Some(h) = code_host {
                    if let Some(owner) = path.trim_start_matches('/').split('/').next() {
                        if !owner.is_empty() && self.orgs.len() < MAX_IDS {
                            self.orgs
                                .insert(format!("{h}/{}", owner.to_ascii_lowercase()));
                        }
                    }
                } else {
                    self.add_domain(&host);
                }
            }
        }
        self.add_terms(text);
    }

    /// Capitalised words in the middle of a sentence (names of projects,
    /// products, clients, people) and longer acronyms such as "CRANQ".
    pub fn add_terms(&mut self, text: &str) {
        let mut after_boundary = true;
        for raw in text.split_whitespace() {
            let w: &str = raw.trim_matches(|c: char| !c.is_alphanumeric());
            let ends_sentence = raw.ends_with(['.', '!', '?', ':']) || raw.ends_with("\n");
            let starts_sentence = after_boundary;
            after_boundary = ends_sentence;
            let mut chars = w.chars();
            let Some(first) = chars.next() else {
                continue;
            };
            let len = w.chars().count();
            if !(3..=30).contains(&len) || w.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            let acronym = len >= 4 && w.chars().all(|c| c.is_uppercase());
            let capitalised =
                first.is_uppercase() && chars.clone().any(|c| c.is_lowercase()) && !starts_sentence;
            if !(acronym || capitalised) {
                continue;
            }
            let lower = w.to_lowercase();
            if is_common_term(&lower) {
                continue;
            }
            if self.terms.len() < MAX_TERMS || self.terms.contains_key(&hash(&lower)) {
                *self.terms.entry(hash(&lower)).or_default() += 1;
            }
        }
    }

    /// Walks a parsed JSON document: people (display and real names next to
    /// an ID or address), addresses, tenant IDs, and text fields.
    pub fn add_json(&mut self, v: &Value) {
        self.walk(v, 0);
    }

    fn walk(&mut self, v: &Value, depth: u8) {
        if depth > 12 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|x| self.walk(x, depth + 1)),
            Value::Object(o) => {
                let personish = [
                    "email",
                    "userId",
                    "userPrincipalName",
                    "address",
                    "is_bot",
                    "real_name",
                    "profile",
                    "userIdentityType",
                    "id",
                ]
                .iter()
                .any(|k| o.contains_key(*k));
                for (k, x) in o {
                    match (k.as_str(), x) {
                        (
                            "email" | "mail" | "userPrincipalName" | "address" | "emailAddress",
                            Value::String(s),
                        ) => self.add_address(s),
                        (
                            "displayName" | "real_name" | "display_name" | "name" | "fullName",
                            Value::String(s),
                        ) if personish && s.contains(' ') => self.add_person(s),
                        ("tenantId" | "team_id" | "enterprise_id", Value::String(s)) => {
                            self.add_org(s)
                        }
                        (
                            "text" | "content" | "subject" | "description" | "bodyPreview"
                            | "topic" | "purpose" | "value",
                            Value::String(s),
                        ) => self.add_text(s),
                        (_, Value::String(s)) if s.contains('@') && s.len() < 200 => {
                            for m in email_re().find_iter(s) {
                                self.add_address(m.as_str());
                            }
                        }
                        (_, x @ (Value::Object(_) | Value::Array(_))) => self.walk(x, depth + 1),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    pub fn merge(&mut self, o: &Fingerprint) {
        self.people.extend(o.people.iter().copied());
        self.addresses.extend(o.addresses.iter().copied());
        self.orgs.extend(o.orgs.iter().cloned());
        for (k, v) in &o.terms {
            *self.terms.entry(*k).or_default() += v;
        }
    }

    /// Terms seen at least twice, most frequent first, at most `n`.
    pub fn top_terms(&self, n: usize) -> Vec<u64> {
        let mut t: Vec<(u64, u32)> = self
            .terms
            .iter()
            .filter(|(_, &c)| c >= 2)
            .map(|(k, c)| (*k, *c))
            .collect();
        t.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        t.into_iter().take(n).map(|x| x.0).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.people.is_empty()
            && self.addresses.is_empty()
            && self.orgs.is_empty()
            && self.terms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_domains_and_common_words_do_not_count() {
        let mut f = Fingerprint::default();
        f.add_text("Mail me at bob@gmail.com or see https://www.google.com/x and https://github.com/acme/app. We met on Monday with Google.");
        assert_eq!(
            f.orgs,
            ["github.com/acme".to_string()].into_iter().collect()
        );
        assert!(f.terms.is_empty(), "{:?}", f.terms.len());
        assert_eq!(f.addresses.len(), 1);
    }

    #[test]
    fn names_domains_and_terms() {
        let mut f = Fingerprint::default();
        f.add_text("Ask anna@cranq.com about the CRANQ roadmap. The Hydra release for Acme is late; Hydra needs Acme sign-off.");
        assert!(f.orgs.contains("cranq.com"));
        let top = f.top_terms(10);
        assert_eq!(top.len(), 2, "Hydra and Acme twice; CRANQ once");
        f.add_json(&json!({"value": [{"displayName": "Dev Shah", "email": "dev@pixpaint.example", "tenantId": "T1"}]}));
        assert_eq!(f.people.len(), 1);
        assert!(f.orgs.contains("pixpaint.example") && f.orgs.contains("org:t1"));
    }
}
