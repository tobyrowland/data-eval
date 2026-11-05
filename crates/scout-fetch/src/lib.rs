//! DataPack Scout Attachment Downloader.
//!
//! A Slack workspace export contains links to shared files, not the files
//! themselves. This library reads an export, lists the referenced files and
//! downloads them using the links already in the export.
//!
//! It is deliberately separate from the DataPack Scout assessment app, which
//! has no network capability. This tool connects only to Slack's file hosts,
//! enforced for every request and every redirect by [`HostPolicy`]. It never
//! contacts the DataPack Scout operator. Links are never written to disk or
//! shown in full: they carry access tokens.

use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub const TOOL_NAME: &str = "DataPack Scout Attachment Downloader";
pub const MANIFEST_NAME: &str = "datapack-scout-attachments.json";
/// Folder inside the output directory that DataPack Scout recognises.
pub const ATTACHMENTS_DIR: &str = "attachments";

// ------------------------------------------------------------------ policy

/// Which hosts the downloader may contact.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    exact: Vec<String>,
    suffixes: Vec<String>,
    allow_http: bool,
}

impl HostPolicy {
    /// Slack's own file hosts, HTTPS only. This is the only policy the app uses.
    pub fn slack() -> Self {
        HostPolicy {
            exact: vec!["slack.com".into(), "files.slack.com".into()],
            suffixes: vec![".slack.com".into(), ".slack-edge.com".into()],
            allow_http: false,
        }
    }

    /// Plain-HTTP access to one local host, for tests only.
    #[doc(hidden)]
    pub fn local_test(host: &str) -> Self {
        HostPolicy {
            exact: vec![host.to_ascii_lowercase()],
            suffixes: vec![],
            allow_http: true,
        }
    }

    pub fn allows(&self, url: &str) -> bool {
        match parse_url(url) {
            Some((scheme, host)) => {
                (scheme == "https" || (self.allow_http && scheme == "http"))
                    && (self.exact.contains(&host)
                        || self.suffixes.iter().any(|s| host.ends_with(s.as_str())))
            }
            None => false,
        }
    }
}

/// Returns (scheme, lowercase host). Rejects user-info and odd schemes.
fn parse_url(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = if authority.starts_with('[') {
        authority.split(']').next()?.to_string() + "]"
    } else {
        authority.split(':').next()?.to_string()
    };
    Some((scheme, host.to_ascii_lowercase()))
}

/// A link with its query (which holds the access token) removed, for display.
pub fn redact(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?…"),
        None => url.to_string(),
    }
}

fn resolve_location(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if location.starts_with("//") {
        format!("{scheme}:{location}")
    } else if location.starts_with('/') {
        format!("{scheme}://{authority}{location}")
    } else {
        let dir = rest.rsplit_once('/').map(|(d, _)| d).unwrap_or(authority);
        format!("{scheme}://{dir}/{location}")
    }
}

// ------------------------------------------------------------------- plan

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Document,
}

/// One referenced file. The link is private and never serialised.
#[derive(Debug, Clone, Serialize)]
pub struct FileRef {
    pub id: String,
    pub name: String,
    pub mimetype: String,
    pub size: Option<u64>,
    pub kind: Kind,
    #[serde(skip)]
    url: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub export_name: String,
    pub files: Vec<FileRef>,
    /// Hidden by a plan limit or deleted (no link Slack will honour).
    pub skipped_hidden: u64,
    /// Stored outside Slack (e.g. Google Drive links); not downloaded.
    pub skipped_external: u64,
    /// Records without a usable Slack link or ID.
    pub skipped_unusable: u64,
}

impl Plan {
    pub fn images(&self) -> u64 {
        self.files.iter().filter(|f| f.kind == Kind::Image).count() as u64
    }
    pub fn known_bytes(&self) -> u64 {
        self.files.iter().filter_map(|f| f.size).sum()
    }
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

fn valid_id(id: &str) -> bool {
    (3..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

const MAX_JSON_BYTES: u64 = 128 * 1024 * 1024;

/// Reads a Slack export (ZIP or unzipped folder) and lists the files it
/// refers to. Nothing is downloaded here.
pub fn plan_from_export(path: &Path) -> Result<Plan, String> {
    let mut plan = Plan {
        export_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Plan::default()
    };
    let mut seen = HashSet::new();
    let mut saw_metadata = false;
    let mut add_doc = |plan: &mut Plan, bytes: &[u8]| {
        if let Ok(serde_json::Value::Array(msgs)) =
            serde_json::from_slice::<serde_json::Value>(bytes)
        {
            for m in &msgs {
                collect_files(plan, &mut seen, m);
            }
        }
    };
    if path.is_dir() {
        let mut stack = vec![(path.to_path_buf(), 0u8)];
        while let Some((dir, depth)) = stack.pop() {
            let rd =
                fs::read_dir(&dir).map_err(|e| format!("Could not open {}: {e}", dir.display()))?;
            for e in rd.flatten() {
                let p = e.path();
                let Ok(md) = fs::symlink_metadata(&p) else {
                    continue;
                };
                let name = e.file_name().to_string_lossy().into_owned();
                if md.is_dir() && depth < 3 {
                    stack.push((p, depth + 1));
                } else if md.is_file() {
                    if name == "channels.json" || name == "users.json" {
                        saw_metadata = true;
                    } else if is_day_file(&name) && md.len() <= MAX_JSON_BYTES {
                        if let Ok(b) = fs::read(&p) {
                            add_doc(&mut plan, &b);
                        }
                    }
                }
            }
        }
    } else {
        let f = File::open(path).map_err(|e| format!("Could not open the export: {e}"))?;
        let mut zip = zip::ZipArchive::new(f)
            .map_err(|_| "This does not look like a ZIP file.".to_string())?;
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let name = entry.name().to_string();
            let base = name.rsplit('/').next().unwrap_or("");
            if base == "channels.json" || base == "users.json" {
                saw_metadata = true;
            } else if is_day_file(base) && name.contains('/') && entry.size() <= MAX_JSON_BYTES {
                let mut b = Vec::new();
                if (&mut entry)
                    .take(MAX_JSON_BYTES + 1)
                    .read_to_end(&mut b)
                    .is_ok()
                    && b.len() as u64 <= MAX_JSON_BYTES
                {
                    add_doc(&mut plan, &b);
                }
            }
        }
    }
    if !saw_metadata {
        return Err(
            "This does not look like a Slack workspace export (no channels.json or users.json)."
                .into(),
        );
    }
    plan.files.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plan)
}

fn collect_files(plan: &mut Plan, seen: &mut HashSet<String>, m: &serde_json::Value) {
    let Some(files) = m.get("files").and_then(|f| f.as_array()) else {
        return;
    };
    for f in files {
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !valid_id(id) {
            plan.skipped_unusable += 1;
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }
        let mode = f.get("mode").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(mode, "hidden_by_limit" | "tombstone") {
            plan.skipped_hidden += 1;
            continue;
        }
        if mode == "external"
            || f.get("is_external")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            plan.skipped_external += 1;
            continue;
        }
        let url = ["url_private_download", "url_private"]
            .iter()
            .find_map(|k| f.get(*k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or("");
        if url.is_empty() {
            plan.skipped_unusable += 1;
            continue;
        }
        let mimetype = f
            .get("mimetype")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = f
            .get("name")
            .and_then(|v| v.as_str())
            .or_else(|| f.get("title").and_then(|v| v.as_str()))
            .unwrap_or("file")
            .to_string();
        plan.files.push(FileRef {
            id: id.to_string(),
            name,
            kind: if mimetype.starts_with("image/") {
                Kind::Image
            } else {
                Kind::Document
            },
            mimetype,
            size: f.get("size").and_then(|v| v.as_u64()),
            url: url.to_string(),
        });
    }
}

/// A file name that is safe on Windows, macOS and Linux.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut s: String = base
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.chars().count() > 120 {
        let ext = s
            .rsplit_once('.')
            .map(|(_, e)| e.to_string())
            .filter(|e| e.len() <= 10);
        let stem: String = s.chars().take(100).collect();
        s = match ext {
            Some(e) => format!("{stem}.{e}"),
            None => stem,
        };
    }
    let upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4);
    if s.is_empty() || s == "." || s == ".." {
        s = "file".into();
    } else if reserved {
        s = format!("_{s}");
    }
    s
}

// --------------------------------------------------------------- download

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum Outcome {
    Downloaded { bytes: u64 },
    AlreadyPresent,
    Failed { reason: FailReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// The link pointed (or redirected) outside Slack, so it was not followed.
    NotSlack,
    /// Slack asked for a sign-in, or the link's access was revoked or expired.
    NeedsSignIn,
    /// The file was deleted from Slack.
    Deleted,
    TooLarge,
    Http(u16),
    Network,
    Disk,
    Stopped,
}

impl FailReason {
    pub fn explain(&self) -> &'static str {
        match self {
            FailReason::NotSlack => "the link points outside Slack, so it was not followed",
            FailReason::NeedsSignIn => {
                "Slack asked for a sign-in, or the link's access was revoked or has expired"
            }
            FailReason::Deleted => "the file has been deleted from Slack",
            FailReason::TooLarge => "the file is larger than the download limit",
            FailReason::Http(_) => "Slack returned an error",
            FailReason::Network => "the connection failed",
            FailReason::Disk => "the file could not be saved",
            FailReason::Stopped => "you stopped the download",
        }
    }
}

pub struct Options {
    pub out_dir: PathBuf,
    pub policy: HostPolicy,
    pub workers: usize,
    pub max_file_bytes: u64,
    pub cancel: Arc<AtomicBool>,
}

impl Options {
    pub fn new(out_dir: PathBuf) -> Self {
        Options {
            out_dir,
            policy: HostPolicy::slack(),
            workers: 4,
            max_file_bytes: 4 * 1024 * 1024 * 1024,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub total: u64,
    pub finished: u64,
    pub downloaded: u64,
    pub already_present: u64,
    pub failed: u64,
    pub bytes: u64,
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureCount {
    pub reason: FailReason,
    pub explanation: &'static str,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub progress: Progress,
    pub failures: Vec<FailureCount>,
    pub out_dir: String,
    pub stopped: bool,
}

#[derive(Serialize)]
struct ManifestEntry<'a> {
    id: &'a str,
    kind: Kind,
    path: Option<String>,
    #[serde(flatten)]
    outcome: &'a Outcome,
}

fn target_dir(out: &Path, id: &str) -> PathBuf {
    out.join(ATTACHMENTS_DIR).join(id)
}

fn existing_file(dir: &Path, expected: Option<u64>) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "part") {
            continue;
        }
        let len = e.metadata().ok()?.len();
        if expected.is_none_or(|n| n == len) {
            return Some(p);
        }
    }
    None
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .user_agent(TOOL_NAME)
        .build()
        .into()
}

fn fetch_one(agent: &ureq::Agent, f: &FileRef, opts: &Options, on_bytes: &dyn Fn(u64)) -> Outcome {
    let dir = target_dir(&opts.out_dir, &f.id);
    if existing_file(&dir, f.size).is_some() {
        return Outcome::AlreadyPresent;
    }
    let mut url = f.url.clone();
    let mut resp = None;
    for _ in 0..6 {
        if !opts.policy.allows(&url) {
            return Outcome::Failed {
                reason: FailReason::NotSlack,
            };
        }
        if opts.cancel.load(Ordering::Relaxed) {
            return Outcome::Failed {
                reason: FailReason::Stopped,
            };
        }
        let r = match agent.get(&url).call() {
            Ok(r) => r,
            Err(_) => {
                return Outcome::Failed {
                    reason: FailReason::Network,
                }
            }
        };
        let status = r.status().as_u16();
        if (300..400).contains(&status) {
            let Some(loc) = r.headers().get("location").and_then(|v| v.to_str().ok()) else {
                return Outcome::Failed {
                    reason: FailReason::Http(status),
                };
            };
            url = resolve_location(&url, loc);
            continue;
        }
        resp = Some(r);
        break;
    }
    let Some(resp) = resp else {
        return Outcome::Failed {
            reason: FailReason::Http(310),
        };
    };
    match resp.status().as_u16() {
        200 => {}
        401 | 403 => {
            return Outcome::Failed {
                reason: FailReason::NeedsSignIn,
            }
        }
        404 | 410 => {
            return Outcome::Failed {
                reason: FailReason::Deleted,
            }
        }
        s => {
            return Outcome::Failed {
                reason: FailReason::Http(s),
            }
        }
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Slack answers an expired or revoked link with an HTML sign-in page.
    if ctype.starts_with("text/html") && !f.mimetype.contains("html") {
        return Outcome::Failed {
            reason: FailReason::NeedsSignIn,
        };
    }
    if fs::create_dir_all(&dir).is_err() {
        return Outcome::Failed {
            reason: FailReason::Disk,
        };
    }
    let final_path = dir.join(safe_file_name(&f.name));
    let part = dir.join(format!("{}.part", safe_file_name(&f.name)));
    let result = (|| -> Result<u64, FailReason> {
        let mut out = File::create(&part).map_err(|_| FailReason::Disk)?;
        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            if opts.cancel.load(Ordering::Relaxed) {
                return Err(FailReason::Stopped);
            }
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(FailReason::Network),
            };
            total += n as u64;
            if total > opts.max_file_bytes {
                return Err(FailReason::TooLarge);
            }
            out.write_all(&buf[..n]).map_err(|_| FailReason::Disk)?;
            on_bytes(n as u64);
        }
        out.sync_all().map_err(|_| FailReason::Disk)?;
        Ok(total)
    })();
    match result {
        Ok(bytes) => match fs::rename(&part, &final_path) {
            Ok(()) => Outcome::Downloaded { bytes },
            Err(_) => {
                let _ = fs::remove_file(&part);
                Outcome::Failed {
                    reason: FailReason::Disk,
                }
            }
        },
        Err(reason) => {
            let _ = fs::remove_file(&part);
            Outcome::Failed { reason }
        }
    }
}

/// Downloads every file in `plan` into `opts.out_dir/attachments/<ID>/`,
/// skipping files already there, then writes a manifest without links.
pub fn download(plan: &Plan, opts: &Options, progress: &mut dyn FnMut(&Progress)) -> Summary {
    let _ = fs::create_dir_all(&opts.out_dir);
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<Msg>();
    enum Msg {
        Bytes(u64),
        Start(usize),
        Done(usize, Outcome),
    }
    let mut outcomes: Vec<Option<Outcome>> = vec![None; plan.files.len()];
    let mut p = Progress {
        total: plan.files.len() as u64,
        ..Progress::default()
    };
    progress(&p);
    std::thread::scope(|scope| {
        for _ in 0..opts.workers.max(1) {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                let agent = agent();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(f) = plan.files.get(i) else { break };
                    let _ = tx.send(Msg::Start(i));
                    let outcome = if opts.cancel.load(Ordering::Relaxed) {
                        Outcome::Failed {
                            reason: FailReason::Stopped,
                        }
                    } else {
                        let txb = tx.clone();
                        fetch_one(&agent, f, opts, &move |n| {
                            let _ = txb.send(Msg::Bytes(n));
                        })
                    };
                    let _ = tx.send(Msg::Done(i, outcome));
                }
            });
        }
        drop(tx);
        let mut last = std::time::Instant::now();
        for msg in rx {
            match msg {
                Msg::Bytes(n) => p.bytes += n,
                Msg::Start(i) => p.current = plan.files[i].name.clone(),
                Msg::Done(i, o) => {
                    p.finished += 1;
                    match &o {
                        Outcome::Downloaded { .. } => p.downloaded += 1,
                        Outcome::AlreadyPresent => p.already_present += 1,
                        Outcome::Failed { .. } => p.failed += 1,
                    }
                    outcomes[i] = Some(o);
                }
            }
            if last.elapsed() >= Duration::from_millis(80) || p.finished == p.total {
                progress(&p);
                last = std::time::Instant::now();
            }
        }
    });
    progress(&p);

    let mut failures: Vec<FailureCount> = Vec::new();
    let mut entries = Vec::new();
    let fallback = Outcome::Failed {
        reason: FailReason::Stopped,
    };
    for (f, o) in plan.files.iter().zip(outcomes.iter()) {
        let o = o.as_ref().unwrap_or(&fallback);
        if let Outcome::Failed { reason } = o {
            let key = match reason {
                FailReason::Http(_) => FailReason::Http(0),
                r => r.clone(),
            };
            match failures.iter_mut().find(|x| x.reason == key) {
                Some(x) => x.files += 1,
                None => failures.push(FailureCount {
                    explanation: key.explain(),
                    reason: key,
                    files: 1,
                }),
            }
        }
        let path = existing_file(&target_dir(&opts.out_dir, &f.id), None).and_then(|p| {
            p.strip_prefix(&opts.out_dir)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        });
        entries.push(ManifestEntry {
            id: &f.id,
            kind: f.kind,
            path,
            outcome: o,
        });
    }
    failures.sort_by_key(|f| std::cmp::Reverse(f.files));
    let manifest = serde_json::json!({
        "tool": TOOL_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "export": plan.export_name,
        "note": "Attachments downloaded from Slack using the links in this export. Add this folder to DataPack Scout alongside the export.",
        "files": entries,
    });
    let _ = fs::write(
        opts.out_dir.join(MANIFEST_NAME),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    );
    let _ = fs::write(
        opts.out_dir.join("README.txt"),
        format!(
            "These files were downloaded from Slack by the {TOOL_NAME}, using the links in \"{}\".\n\n\
             To include them in your assessment, add this whole folder to DataPack Scout together with the export.\n\
             Each file sits in a folder named after its Slack file ID, so DataPack Scout can match it to the messages that shared it.\n",
            plan.export_name
        ),
    );
    Summary {
        stopped: opts.cancel.load(Ordering::Relaxed),
        progress: p,
        failures,
        out_dir: opts.out_dir.display().to_string(),
    }
}

/// Default output folder: next to the export, "<export name> attachments".
pub fn default_out_dir(export: &Path) -> PathBuf {
    let stem = export
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Slack export".into());
    export
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{stem} attachments"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_allows_only_slack_https() {
        let p = HostPolicy::slack();
        assert!(p.allows("https://files.slack.com/files-pri/T1-F1/download/a.png?t=xoxe-1"));
        assert!(p.allows("https://acme.slack.com/files/x"));
        assert!(p.allows("https://a.slack-edge.com/x"));
        assert!(!p.allows("http://files.slack.com/x"));
        assert!(!p.allows("https://files.slack.com.evil.example/x"));
        assert!(!p.allows("https://evilslack.com/x"));
        assert!(!p.allows("https://user@files.slack.com/x"));
        assert!(!p.allows("https://drive.google.com/file/d/1"));
        assert!(!p.allows("file:///etc/passwd"));
        assert!(!p.allows("javascript:alert(1)"));
    }

    #[test]
    fn redirects_resolve() {
        assert_eq!(
            resolve_location("https://files.slack.com/a/b?t=1", "/c"),
            "https://files.slack.com/c"
        );
        assert_eq!(
            resolve_location("https://files.slack.com/a/b", "https://x.slack-edge.com/d"),
            "https://x.slack-edge.com/d"
        );
        assert_eq!(
            resolve_location("https://files.slack.com/a/b", "//evil.example/x"),
            "https://evil.example/x"
        );
    }

    #[test]
    fn redaction_hides_tokens() {
        assert_eq!(
            redact("https://files.slack.com/x.png?t=xoxe-secret"),
            "https://files.slack.com/x.png?…"
        );
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name("a<b>:c?.png"), "a_b__c_.png");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("..."), "file");
        assert_eq!(safe_file_name("name. "), "name");
        assert!(safe_file_name(&"x".repeat(300)).len() <= 120);
    }
}
