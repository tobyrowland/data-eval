//! Data sources referenced by the material already assessed.
//!
//! Links in messages and documents, apps installed in Slack, and messages
//! posted by those apps' bots point to other systems the company uses. The
//! owner sees a list of those systems with the evidence for each, so they
//! know which exports to add next.
//!
//! URLs and link targets stay inside this private local state; only service
//! names and counts leave it.

use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

/// A system the owner might export data from.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    /// What a distinct link target usually is, e.g. "tickets".
    pub target_noun: &'static str,
    /// Lowercase names this service's Slack app or bot goes by.
    #[serde(skip)]
    pub app_names: &'static [&'static str],
}

pub const CATALOG: &[Service] = &[
    svc("slack", "Slack", "Conversations", "messages", &[]),
    svc(
        "teams",
        "Microsoft Teams",
        "Conversations",
        "messages",
        &["microsoft teams", "teams"],
    ),
    svc(
        "email",
        "Email",
        "Conversations",
        "messages",
        &["gmail", "outlook"],
    ),
    svc("github", "GitHub", "Code", "pages", &["github"]),
    svc("gitlab", "GitLab", "Code", "pages", &["gitlab"]),
    svc("bitbucket", "Bitbucket", "Code", "pages", &["bitbucket"]),
    svc("jira", "Jira", "Issues and projects", "tickets", &["jira"]),
    svc(
        "linear",
        "Linear",
        "Issues and projects",
        "issues",
        &["linear"],
    ),
    svc("asana", "Asana", "Issues and projects", "tasks", &["asana"]),
    svc(
        "trello",
        "Trello",
        "Issues and projects",
        "cards",
        &["trello"],
    ),
    svc(
        "google_drive",
        "Google Drive and Docs",
        "Documents",
        "documents",
        &["google drive", "google docs", "google sheets"],
    ),
    svc("notion", "Notion", "Documents", "pages", &["notion"]),
    svc(
        "confluence",
        "Confluence",
        "Documents",
        "pages",
        &["confluence"],
    ),
    svc(
        "microsoft365",
        "OneDrive and SharePoint",
        "Documents",
        "documents",
        &["onedrive", "sharepoint", "microsoft"],
    ),
    svc("dropbox", "Dropbox", "Documents", "files", &["dropbox"]),
    svc("box", "Box", "Documents", "files", &["box"]),
    svc("figma", "Figma", "Design", "files", &["figma"]),
    svc("miro", "Miro", "Design", "boards", &["miro"]),
    svc(
        "zendesk",
        "Zendesk",
        "Customer support",
        "tickets",
        &["zendesk"],
    ),
    svc(
        "intercom",
        "Intercom",
        "Customer support",
        "conversations",
        &["intercom"],
    ),
    svc(
        "salesforce",
        "Salesforce",
        "Sales and CRM",
        "records",
        &["salesforce"],
    ),
    svc(
        "hubspot",
        "HubSpot",
        "Sales and CRM",
        "records",
        &["hubspot"],
    ),
    svc(
        "sentry",
        "Sentry",
        "Engineering operations",
        "issues",
        &["sentry"],
    ),
    svc(
        "datadog",
        "Datadog",
        "Engineering operations",
        "pages",
        &["datadog"],
    ),
    svc(
        "pagerduty",
        "PagerDuty",
        "Engineering operations",
        "incidents",
        &["pagerduty"],
    ),
    svc(
        "circleci",
        "CircleCI",
        "Engineering operations",
        "builds",
        &["circleci"],
    ),
    svc("loom", "Loom", "Recordings", "videos", &["loom"]),
    svc("zoom", "Zoom", "Recordings", "meetings", &["zoom"]),
    svc(
        "airtable",
        "Airtable",
        "Structured data",
        "bases",
        &["airtable"],
    ),
];

const fn svc(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    target_noun: &'static str,
    app_names: &'static [&'static str],
) -> Service {
    Service {
        id,
        name,
        category,
        target_noun,
        app_names,
    }
}

pub fn service(id: &str) -> Option<&'static Service> {
    CATALOG.iter().find(|s| s.id == id)
}

/// Maps a link's host and path to a service in the catalogue.
pub fn classify(host: &str, path: &str) -> Option<&'static str> {
    let h = host.trim_start_matches("www.").to_ascii_lowercase();
    let ends = |d: &str| h == d || h.ends_with(&format!(".{d}"));
    Some(match () {
        _ if ends("github.com") || ends("githubusercontent.com") => "github",
        _ if ends("gitlab.com") => "gitlab",
        _ if ends("bitbucket.org") => "bitbucket",
        _ if ends("atlassian.net")
            && (path.starts_with("/wiki") || path.contains("/confluence")) =>
        {
            "confluence"
        }
        _ if ends("atlassian.net") => "jira",
        _ if ends("linear.app") => "linear",
        _ if ends("asana.com") => "asana",
        _ if ends("trello.com") => "trello",
        _ if h == "docs.google.com"
            || h == "drive.google.com"
            || h == "sheets.google.com"
            || h == "slides.google.com" =>
        {
            "google_drive"
        }
        _ if h == "teams.microsoft.com" || h == "teams.live.com" => "teams",
        _ if ends("notion.so") || ends("notion.site") => "notion",
        _ if ends("sharepoint.com") || ends("onedrive.live.com") || h == "1drv.ms" => {
            "microsoft365"
        }
        _ if ends("dropbox.com") || h == "db.tt" => "dropbox",
        _ if ends("box.com") || h == "app.box.com" => "box",
        _ if ends("figma.com") => "figma",
        _ if ends("miro.com") => "miro",
        _ if ends("zendesk.com") => "zendesk",
        _ if ends("intercom.com") || ends("intercom.io") => "intercom",
        _ if ends("salesforce.com") || ends("force.com") => "salesforce",
        _ if ends("hubspot.com") => "hubspot",
        _ if ends("sentry.io") => "sentry",
        _ if ends("datadoghq.com") || ends("datadoghq.eu") => "datadog",
        _ if ends("pagerduty.com") => "pagerduty",
        _ if ends("circleci.com") => "circleci",
        _ if ends("loom.com") => "loom",
        _ if ends("zoom.us") => "zoom",
        _ if ends("airtable.com") => "airtable",
        _ => return None,
    })
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"https?://([A-Za-z0-9.\-]+)(?::\d+)?(/[^\s<>|"'`)\]\\]*)?"#).unwrap()
    })
}

#[derive(Debug, Clone, Default)]
pub struct ServiceStats {
    pub links: u64,
    /// Distinct link targets (host and path, without query). Local only.
    pub targets: HashSet<String>,
    pub app_installed: bool,
    pub bot_messages: u64,
    pub collections: BTreeSet<u32>,
}

/// Evidence of other systems, accumulated across an assessment.
#[derive(Debug, Clone, Default)]
pub struct SourceTally {
    pub by_service: BTreeMap<&'static str, ServiceStats>,
}

/// Links to known services in a piece of text, by service. Used for
/// per-file counts that stay local (which files tie an item to a source).
pub fn count_links(text: &str) -> BTreeMap<&'static str, u32> {
    let mut out = BTreeMap::new();
    if !text.contains("http") {
        return out;
    }
    for c in url_re().captures_iter(text) {
        let path = c.get(2).map_or("", |m| m.as_str());
        if let Some(id) = classify(&c[1], path) {
            *out.entry(id).or_default() += 1;
        }
    }
    out
}

impl SourceTally {
    /// Follows a re-numbering of collections (after some were removed).
    pub fn renumber_collections(&mut self, map: &std::collections::HashMap<u32, u32>) {
        for st in self.by_service.values_mut() {
            st.collections = st
                .collections
                .iter()
                .filter_map(|c| map.get(c).copied())
                .collect();
        }
    }

    /// Counts links to known services in a piece of text.
    pub fn scan_text(&mut self, text: &str, collection: u32) {
        if !text.contains("http") {
            return;
        }
        for c in url_re().captures_iter(text) {
            let host = &c[1];
            let path = c.get(2).map_or("", |m| m.as_str());
            let Some(id) = classify(host, path) else {
                continue;
            };
            let s = self.by_service.entry(id).or_default();
            s.links += 1;
            s.collections.insert(collection);
            let target = format!(
                "{}{}",
                host.to_ascii_lowercase(),
                path.trim_end_matches('/')
            );
            s.targets.insert(target);
        }
    }

    /// Records an app installed in Slack, by the name in integration logs.
    pub fn note_app(&mut self, name: &str, collection: u32) {
        if let Some(id) = match_app(name) {
            let s = self.by_service.entry(id).or_default();
            s.app_installed = true;
            s.collections.insert(collection);
        }
    }

    /// Records a message posted by an app's bot.
    pub fn note_bot_message(&mut self, name: &str, collection: u32) {
        if let Some(id) = match_app(name) {
            let s = self.by_service.entry(id).or_default();
            s.bot_messages += 1;
            s.collections.insert(collection);
        }
    }

    pub fn merge(&mut self, other: &SourceTally) {
        for (id, o) in &other.by_service {
            let s = self.by_service.entry(id).or_default();
            s.links += o.links;
            s.targets.extend(o.targets.iter().cloned());
            s.app_installed |= o.app_installed;
            s.bot_messages += o.bot_messages;
            s.collections.extend(o.collections.iter().copied());
        }
    }
}

/// The Google Takeout product a path belongs to, e.g. "Drive" for
/// `…/Takeout/Drive/Plans.docx`.
pub fn takeout_product(path: &str) -> Option<&str> {
    let mut segs = path.split('/');
    while let Some(s) = segs.next() {
        if s == "Takeout" {
            let product = segs.next()?;
            // A file directly under Takeout/ (e.g. archive_browser.html) is not a product.
            return segs.next().map(|_| product);
        }
    }
    None
}

/// Whether a path is a Git repository's config file (`…/.git/config`).
pub fn is_git_config(path: &str) -> bool {
    path.ends_with("/.git/config") || path == ".git/config"
}

/// Remote repositories named in a `.git/config`, as `host/owner/repo`.
pub fn git_remotes(config: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some(url) = line
            .strip_prefix("url")
            .map(str::trim_start)
            .and_then(|l| l.strip_prefix('='))
        else {
            continue;
        };
        let url = url.trim();
        // https://host/owner/repo(.git), ssh://git@host/owner/repo, git@host:owner/repo
        let rest = if let Some((_, r)) = url.split_once("://") {
            r.split_once('@')
                .map_or(r, |(_, h)| h)
                .replacen(':', "/", 0)
        } else if let Some((_, r)) = url.split_once('@') {
            r.replacen(':', "/", 1)
        } else {
            continue;
        };
        let mut parts = rest.split('/').filter(|p| !p.is_empty());
        let (Some(host), Some(owner), Some(repo)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        let repo = repo.trim_end_matches(".git");
        out.push(format!(
            "{host}/{}/{}",
            owner.to_ascii_lowercase(),
            repo.to_ascii_lowercase()
        ));
    }
    out.sort();
    out.dedup();
    out
}

fn match_app(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    // Longer names first so "google drive" wins over a shorter overlap.
    let mut best: Option<(&'static str, usize)> = None;
    for s in CATALOG {
        for a in s.app_names {
            let hit = if a.len() <= 4 {
                n.split(|c: char| !c.is_alphanumeric()).any(|w| w == *a)
            } else {
                n.contains(a)
            };
            if hit && best.is_none_or(|(_, l)| a.len() > l) {
                best = Some((s.id, a.len()));
            }
        }
    }
    best.map(|b| b.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_services() {
        assert_eq!(classify("github.com", "/acme/app/pull/12"), Some("github"));
        assert_eq!(
            classify("acme.atlassian.net", "/browse/SUP-12"),
            Some("jira")
        );
        assert_eq!(
            classify("acme.atlassian.net", "/wiki/spaces/ENG"),
            Some("confluence")
        );
        assert_eq!(
            classify("docs.google.com", "/document/d/1"),
            Some("google_drive")
        );
        assert_eq!(classify("www.notion.so", "/acme/Plan"), Some("notion"));
        assert_eq!(classify("evilgithub.com", "/x"), None);
        assert_eq!(classify("files.slack.com", "/x"), None);
    }

    #[test]
    fn recognises_takeout_and_git_remotes() {
        assert_eq!(
            takeout_product("t-1.zip/Takeout/Drive/Plans/a.docx"),
            Some("Drive")
        );
        assert_eq!(
            takeout_product("t-1.zip/Takeout/Mail/All mail.mbox"),
            Some("Mail")
        );
        assert_eq!(
            takeout_product("t-1.zip/Takeout/archive_browser.html"),
            None
        );
        assert_eq!(takeout_product("docs/notes.md"), None);
        assert!(is_git_config("app/.git/config"));
        let cfg = "[remote \"origin\"]\n\turl = git@github.com:Acme/App.git\n[remote \"bb\"]\n\turl = https://user@bitbucket.org/acme/core.git\n\turl = ssh://git@gitlab.com:2222/team/tool\n";
        assert_eq!(
            git_remotes(cfg),
            vec![
                "bitbucket.org/acme/core",
                "github.com/acme/app",
                "gitlab.com/team/tool"
            ]
        );
    }

    #[test]
    fn counts_links_targets_apps_and_bots() {
        let mut t = SourceTally::default();
        t.scan_text("see <https://github.com/acme/app/pull/12|PR 12> and https://github.com/acme/app/pull/12/ again", 0);
        t.scan_text("ticket https://acme.atlassian.net/browse/SUP-1?focused=1 and https://docs.google.com/document/d/abc/edit", 1);
        t.note_app("GitHub", 0);
        t.note_bot_message("Jira Cloud", 0);
        t.note_app("Box", 0);
        t.note_app("Dropbox", 0);
        let gh = &t.by_service["github"];
        assert_eq!((gh.links, gh.targets.len(), gh.app_installed), (2, 1, true));
        assert_eq!(t.by_service["jira"].bot_messages, 1);
        assert_eq!(t.by_service["jira"].targets.len(), 1);
        assert!(t.by_service["box"].app_installed && t.by_service["dropbox"].app_installed);
        assert_eq!(
            t.by_service["google_drive"]
                .collections
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![1]
        );
    }
}
