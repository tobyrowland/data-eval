//! Owner-facing "Your Data Map" view derived from the private inventory.
//!
//! The data map carries collection labels the owner chose, counts, coverage
//! and template sentences. It never carries local paths, content hashes,
//! message text, channel names, account IDs or URLs (spec §8).

use crate::analyze::language::{self, LanguageRow, LanguageTally};
use crate::analyze::slack::{civil_from_days, SlackInventory};
use crate::analyze::sources::{Service, SourceTally, CATALOG};
use crate::model::{empty_status_counts, Format, Reason, SourceKind, Status};
use crate::scan::Inventory;
use crate::{APP_NAME, APP_VERSION, RULESET_VERSION};
use rand::RngCore;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct DataMap {
    pub app: &'static str,
    pub app_version: &'static str,
    pub ruleset_version: &'static str,
    pub assessment_id: String,
    pub generated_unix: u64,
    pub summary: Vec<String>,
    pub totals: Totals,
    pub coverage: Coverage,
    pub formats: BTreeMap<&'static str, u64>,
    pub collections: Vec<CollectionSummary>,
    pub limits_reached: Vec<Explained>,
    pub preparation_priorities: Vec<String>,
    /// Links from collections to material they refer to, resolved or not.
    pub relationships: Vec<Relationship>,
    /// Files linked from all Slack exports together, each counted once.
    pub slack_files: Option<SlackFiles>,
    /// Content-derived activity per calendar month (YYYY-MM), all collections.
    pub activity: BTreeMap<String, u64>,
    pub stopped_by_user: bool,
    pub languages: LanguageSummary,
    /// Systems this data comes from or refers to, with the evidence for each.
    pub sources: Vec<SourceEntry>,
    /// Every service the owner can pick from when adding a source.
    pub source_catalog: &'static [Service],
    pub privacy: Privacy,
}

#[derive(Debug, Serialize)]
pub struct Totals {
    pub physical_files: u64,
    pub logical_files: u64,
    pub archives_expanded: u64,
    pub stored_bytes: u64,
    pub expanded_bytes: u64,
    pub unique_bytes: u64,
    pub exact_duplicate_files: u64,
    pub exact_duplicate_bytes: u64,
    pub structured_records: u64,
    pub words_of_text: u64,
}

#[derive(Debug, Serialize)]
pub struct Coverage {
    /// Every status, zeros included, so nothing vanishes from the totals.
    pub by_status: BTreeMap<Status, u64>,
    pub content_examined_percent: Option<f64>,
    pub reasons: Vec<ReasonCount>,
}

#[derive(Debug, Serialize)]
pub struct ReasonCount {
    pub reason: Reason,
    pub explanation: &'static str,
    pub files: u64,
}

#[derive(Debug, Serialize)]
pub struct Explained {
    pub reason: Reason,
    pub explanation: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DateCoverage {
    pub first: String,
    pub last: String,
    /// Where the dates came from. Filesystem dates are never used here.
    pub basis: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CollectionSummary {
    /// Identifies the collection (the selected file or folder name).
    pub label: String,
    /// A friendlier name where the content says what it is, e.g.
    /// "Google Takeout · Drive" for a Takeout archive.
    pub display_label: String,
    pub kind: &'static str,
    pub logical_files: u64,
    pub by_status: BTreeMap<Status, u64>,
    pub structured_records: u64,
    pub expanded_bytes: u64,
    pub date_coverage: Option<DateCoverage>,
    pub explanation: String,
    pub formats: Vec<FormatSlice>,
    pub slack: Vec<SlackInventory>,
}

/// One format family inside a collection, for the visual map.
#[derive(Debug, Serialize)]
pub struct FormatSlice {
    pub format: &'static str,
    pub category: &'static str,
    pub files: u64,
    pub bytes: u64,
    /// Files whose content was examined (assessed or partly assessed).
    pub examined: u64,
}

/// One data source: already in the map, or referenced by data that is.
#[derive(Debug, Serialize)]
pub struct SourceEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    /// Some of this source's data is already part of the assessment.
    pub in_selection: bool,
    /// Collections that are this source (for sources already in the map).
    pub collections: Vec<String>,
    /// How the source was recognised in the map, e.g. "Google Takeout (Drive)".
    pub detected_as: Option<&'static str>,
    /// For code hosts: distinct linked pages inside repositories that were added.
    pub resolved_targets: Option<u64>,
    /// For code hosts: repositories linked to that are not in the map.
    pub other_repositories: Option<u64>,
    /// For code hosts: every added repository is a download without history.
    pub snapshot_only: bool,
    pub links: u64,
    pub distinct_targets: u64,
    pub target_noun: &'static str,
    pub app_installed: bool,
    pub bot_messages: u64,
    /// Collections whose content links to or mentions this source.
    pub referenced_from: Vec<String>,
    /// One plain-English line summarising the evidence.
    pub evidence: String,
}

/// Apparent language mix of the readable text, with its method and limits.
#[derive(Debug, Serialize)]
pub struct LanguageSummary {
    pub method: &'static str,
    /// Words examined for language, including undetermined ones.
    pub words_examined: u64,
    pub undetermined_words: u64,
    /// Share of all examined words that could not be classified.
    pub undetermined_percent: f64,
    /// Share of all examined words that are English (same basis as `rows`).
    /// None when too little text could be classified to say.
    pub english_percent: Option<f64>,
    pub rows: Vec<LanguageRow>,
    pub by_collection: Vec<CollectionLanguages>,
    /// Documents whose text was sampled from the start rather than read in full.
    pub sampled_documents: u64,
    /// Documents with text this release cannot read yet (PDF, Office, OpenDocument).
    pub unreadable_documents: u64,
    /// Media and binary files, which carry no readable text.
    pub non_text_files: u64,
    /// Words in code, logs, data or placeholder text, left out of the shares.
    pub code_or_data_words: u64,
    /// Files skipped as code or data by their type (e.g. .js, .csv, .log).
    pub code_or_data_files: u64,
}

#[derive(Debug, Serialize)]
pub struct CollectionLanguages {
    pub label: String,
    /// "messages" for Slack conversations, "documents" otherwise.
    pub basis: &'static str,
    pub words_examined: u64,
    pub undetermined_words: u64,
    pub code_or_data_words: u64,
    pub rows: Vec<LanguageRow>,
}

#[derive(Debug, Serialize)]
pub struct Relationship {
    /// Collection label the references come from.
    pub from: String,
    /// What is referred to, in plain words.
    pub to: &'static str,
    pub count: u64,
    /// Stable identifier the interface uses to pick a "how to get these" guide.
    pub kind: &'static str,
    /// How many of the referenced originals were found in the selection.
    pub resolved: u64,
    /// Whether all referenced originals are present in the selection.
    pub available_locally: bool,
    /// Collections where matched originals were found.
    pub found_in: Vec<String>,
    pub explanation: String,
}

/// Files referred to by every Slack export in the assessment, each counted
/// once however many exports mention it, and how many are present.
#[derive(Debug, Serialize)]
pub struct SlackFiles {
    pub images: u64,
    pub images_found: u64,
    pub documents: u64,
    pub documents_found: u64,
    /// The Slack exports these come from.
    pub exports: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Privacy {
    pub statement: &'static str,
    pub network: &'static str,
}

fn is_logical_file(status: Status, format: Format) -> bool {
    // An archive whose children were enumerated is a container, not a file.
    !(format.is_container() && status.content_examined())
}

fn ymd(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", group(n), if n == 1 { one } else { many })
}

/// Thousands separators for owner-facing numbers.
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn new_assessment_id() -> String {
    let mut b = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02X}")).collect();
    format!("WM-{}-{}-{}", &hex[0..4], &hex[4..8], &hex[8..12])
}

pub fn build_data_map(inv: &Inventory) -> DataMap {
    let mut by_status = empty_status_counts();
    let mut formats: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut reasons: BTreeMap<Reason, u64> = BTreeMap::new();
    let mut hashes: HashMap<&str, u64> = HashMap::new();
    let mut t = Totals {
        physical_files: 0,
        logical_files: 0,
        archives_expanded: 0,
        stored_bytes: 0,
        expanded_bytes: 0,
        unique_bytes: 0,
        exact_duplicate_files: 0,
        exact_duplicate_bytes: 0,
        structured_records: 0,
        words_of_text: inv.text.iter().map(|x| x.words).sum(),
    };
    let ncol = inv.collections.len();
    let mut col_status: Vec<BTreeMap<Status, u64>> =
        (0..ncol).map(|_| empty_status_counts()).collect();
    let mut col_files = vec![0u64; ncol];
    let mut col_records = vec![0u64; ncol];
    let mut col_bytes = vec![0u64; ncol];
    let mut col_formats: Vec<BTreeMap<Format, (u64, u64, u64)>> =
        (0..ncol).map(|_| BTreeMap::new()).collect();

    for it in &inv.items {
        if it.parent.is_none() {
            t.physical_files += 1;
            t.stored_bytes += it.stored_bytes;
        }
        if !is_logical_file(it.status, it.format) {
            t.archives_expanded += 1;
            continue;
        }
        let c = it.collection as usize;
        t.logical_files += 1;
        t.expanded_bytes += it.expanded_bytes;
        t.structured_records += it.records.unwrap_or(0);
        *by_status.get_mut(&it.status).unwrap() += 1;
        *col_status[c].get_mut(&it.status).unwrap() += 1;
        *formats.entry(it.format.label()).or_default() += 1;
        col_files[c] += 1;
        col_records[c] += it.records.unwrap_or(0);
        col_bytes[c] += it.expanded_bytes;
        let f = col_formats[c].entry(it.format).or_default();
        f.0 += 1;
        f.1 += it.expanded_bytes;
        f.2 += u64::from(it.status.content_examined());
        if let Some(r) = &it.reason {
            *reasons.entry(r.clone()).or_default() += 1;
        }
        match &it.sha256 {
            Some(h) => {
                let seen = hashes.entry(h.as_str()).or_default();
                *seen += 1;
                if *seen > 1 {
                    t.exact_duplicate_files += 1;
                    t.exact_duplicate_bytes += it.expanded_bytes;
                } else {
                    t.unique_bytes += it.expanded_bytes;
                }
            }
            None => t.unique_bytes += it.expanded_bytes,
        }
    }

    let excluded = by_status[&Status::Excluded];
    let examined = by_status[&Status::Assessed] + by_status[&Status::PartlyAssessed];
    let denom = t.logical_files - excluded;
    let pct = (denom > 0).then(|| (examined as f64 * 1000.0 / denom as f64).round() / 10.0);

    let mut first_all: Option<i64> = None;
    let mut last_all: Option<i64> = None;
    let mut collections = Vec::new();
    let snapshots = code_snapshots(inv);
    for c in &inv.collections {
        let i = c.id as usize;
        let slack = inv.slack[i].inventories();
        let first = slack.iter().filter_map(|s| s.first_message_unix).min();
        let last = slack.iter().filter_map(|s| s.last_message_unix).max();
        if let (Some(f), Some(l)) = (first, last) {
            first_all = Some(first_all.map_or(f, |x| x.min(f)));
            last_all = Some(last_all.map_or(l, |x| x.max(l)));
        }
        let explanation = match c.source_kind {
            SourceKind::SlackExport => {
                let s = slack.iter().fold(SlackInventory::default(), |mut a, s| {
                    a.message_records += s.message_records;
                    a.channels_in_metadata += s.channels_in_metadata;
                    a.explicit_threads += s.explicit_threads;
                    a.distinct_file_references += s.distinct_file_references;
                    a.records_also_in_other_exports += s.records_also_in_other_exports;
                    for (&k, &v) in &s.overlap_by_collection {
                        *a.overlap_by_collection.entry(k).or_default() += v;
                    }
                    a
                });
                let mut text = format!(
                    "A Slack workspace export with {} across {}, including {}. It refers to {} not included in the export.",
                    plural(s.message_records, "message record", "message records"),
                    plural(s.channels_in_metadata, "channel", "channels"),
                    plural(s.explicit_threads, "thread", "threads"),
                    plural(s.distinct_file_references, "shared file", "shared files"),
                );
                if s.records_also_in_other_exports > 0 {
                    let others: Vec<String> = s
                        .overlap_by_collection
                        .keys()
                        .filter(|&&k| k != c.id)
                        .map(|&k| inv.collections[k as usize].label.clone())
                        .collect();
                    let where_ = if others.is_empty() {
                        "another export in this selection".to_string()
                    } else {
                        others.join(", ")
                    };
                    text.push_str(&format!(
                        " {} of its messages are also in {}, so they are counted once.",
                        group(s.records_also_in_other_exports),
                        where_
                    ));
                }
                text
            }
            SourceKind::Generic => format!(
                "{} containing {}.",
                plural(col_files[i], "file", "files"),
                plural(col_records[i], "structured record", "structured records")
            ),
        };
        collections.push(CollectionSummary {
            display_label: match snapshots.get(&c.id) {
                Some(snap) => snap.display_label(),
                None => display_label_for(c, &inv.text[i]),
            },
            label: c.label.clone(),
            kind: c.source_kind.label(),
            logical_files: col_files[i],
            by_status: col_status[i].clone(),
            structured_records: col_records[i],
            expanded_bytes: col_bytes[i],
            date_coverage: first.zip(last).map(|(f, l)| DateCoverage {
                first: ymd(f),
                last: ymd(l),
                basis: "message timestamps inside the export",
            }),
            explanation,
            formats: col_formats[i]
                .iter()
                .map(|(f, (files, bytes, examined))| FormatSlice {
                    format: f.label(),
                    category: f.category(),
                    files: *files,
                    bytes: *bytes,
                    examined: *examined,
                })
                .collect(),
            slack,
        });
    }

    // ---- template sentences, tied to measured values only
    let mut summary = Vec::new();
    let span = match (first_all, last_all) {
        (Some(f), Some(l)) => {
            let years = (l - f) as f64 / (365.25 * 86_400.0);
            let y = (years * 10.0).round() / 10.0;
            if y >= 1.0 {
                format!(
                    " covering approximately {y} {}",
                    if y == 1.0 { "year" } else { "years" }
                )
            } else {
                let months = ((l - f) as f64 / (30.44 * 86_400.0)).round().max(1.0) as u64;
                format!(
                    " covering approximately {}",
                    plural(months, "month", "months")
                )
            }
        }
        _ => String::new(),
    };
    summary.push(format!(
        "We found {}{} with {}.",
        plural(ncol as u64, "data collection", "data collections"),
        span,
        plural(t.logical_files, "file", "files")
    ));
    if let Some(p) = pct {
        summary.push(format!("We examined the contents of {p}% of the files."));
    }
    let relationships = relationships(inv, &collections);
    let languages = language_summary(inv);
    if let Some(en) = languages.english_percent {
        let others: Vec<String> = languages
            .rows
            .iter()
            .filter(|r| r.code != "eng")
            .take(2)
            .map(|r| format!("{} ({}%)", r.language, r.share_percent))
            .collect();
        summary.push(if others.is_empty() {
            format!("{en}% of the readable text is in English; the rest is too short to tell.")
        } else if en >= 50.0 {
            format!(
                "About {en}% of the readable text is in English; the largest other languages are {}.",
                others.join(" and ")
            )
        } else {
            format!(
                "Only {en}% of the readable text is in English; the largest other languages are {}.",
                others.join(" and ")
            )
        });
    }
    let all_slack: Vec<SlackInventory> = collections.iter().flat_map(|c| c.slack.clone()).collect();
    let refs: u64 = all_slack.iter().map(|s| s.distinct_file_references).sum();
    let images: u64 = all_slack.iter().map(|s| s.referenced_image_files).sum();
    let resolved_of = |kind: &str| -> u64 {
        relationships
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.resolved)
            .sum()
    };
    let resolved_files = resolved_of("slack_images") + resolved_of("slack_files");
    let missing_images = images.saturating_sub(resolved_of("slack_images"));
    if refs > 0 && resolved_files == 0 {
        summary.push(format!(
            "The conversations refer to {} (including {}) whose originals are not included.",
            plural(refs, "shared file", "shared files"),
            plural(images, "image", "images")
        ));
    } else if refs > 0 {
        summary.push(format!(
            "The conversations refer to {}; {} included in your selection and {} still missing.",
            plural(refs, "shared file", "shared files"),
            if resolved_files == 1 {
                "1 is".to_string()
            } else {
                format!("{} are", group(resolved_files))
            },
            group(refs.saturating_sub(resolved_files))
        ));
    }
    let not_read = by_status[&Status::Inventoried];
    if not_read > 0 {
        summary.push(format!(
            "{} counted, but the contents need a parser that is not in this release.",
            if not_read == 1 {
                "1 file was".to_string()
            } else {
                format!("{} files were", group(not_read))
            }
        ));
    }
    if !inv.limits_reached.is_empty() {
        summary.push(if inv.cancelled {
            "You stopped the assessment, so this map covers only what was read before then.".into()
        } else {
            "A capacity or safety limit was reached, so this is not a full assessment of everything selected.".into()
        });
    }

    // ---- preparation priorities, ranked by how much they change coverage
    let mut prep: Vec<(u64, String)> = Vec::new();
    if missing_images > 0 {
        prep.push((
            missing_images,
            format!(
                "Add the original attachments: {} referenced by conversations {} no local copy. The Working Minds Attachment Downloader can fetch them from Slack for you.",
                plural(missing_images, "image", "images"),
                if missing_images == 1 { "has" } else { "have" }
            ),
        ));
    }
    let locked = by_status[&Status::Locked];
    if locked > 0 {
        prep.push((
            locked,
            format!(
                "Add locally decrypted or readable copies of {}.",
                plural(locked, "locked file", "locked files")
            ),
        ));
    }
    let unavailable = by_status[&Status::Unavailable];
    if unavailable > 0 {
        prep.push((
            unavailable,
            format!(
                "Make local copies of {} that are online-only placeholders.",
                plural(unavailable, "file", "files")
            ),
        ));
    }
    let prs: u64 = all_slack
        .iter()
        .map(|s| s.distinct_bitbucket_pr_references + s.distinct_github_pr_references)
        .sum();
    if prs > 0 {
        prep.push((prs, format!("If you hold the repositories behind the {} referenced in conversations, add a local copy with its history.", plural(prs, "pull request", "pull requests"))));
    }
    if not_read > 0 {
        prep.push((not_read, format!(
            "{} counted only. A future release, or an export in a more common format, can add {} to the assessment.",
            if not_read == 1 { "1 file was".to_string() } else { format!("{} files were", group(not_read)) },
            if not_read == 1 { "it" } else { "them" }
        )));
    }
    if !inv.limits_reached.is_empty() {
        prep.push((
            u64::MAX,
            "Select a smaller scope, or split the selection, so every item can be fully assessed."
                .into(),
        ));
    }
    prep.sort_by_key(|p| std::cmp::Reverse(p.0));

    let activity: BTreeMap<String, u64> = collections
        .iter()
        .flat_map(|c| c.slack.iter())
        .flat_map(|s| s.monthly_activity.iter())
        .fold(BTreeMap::new(), |mut acc, (k, v)| {
            *acc.entry(k.clone()).or_default() += v;
            acc
        });

    DataMap {
        app: APP_NAME,
        app_version: APP_VERSION,
        ruleset_version: RULESET_VERSION,
        assessment_id: inv.assessment_id.clone(),
        generated_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        summary,
        totals: t,
        coverage: Coverage {
            by_status: by_status.clone(),
            content_examined_percent: pct,
            reasons: reasons
                .into_iter()
                .map(|(reason, files)| ReasonCount { explanation: reason.explain(), reason, files })
                .collect(),
        },
        formats,
        collections,
        limits_reached: inv
            .limits_reached
            .iter()
            .map(|r| Explained { reason: r.clone(), explanation: r.explain() })
            .collect(),
        preparation_priorities: prep.into_iter().map(|p| p.1).collect(),
        relationships,
        slack_files: slack_files(inv),
        activity,
        stopped_by_user: inv.cancelled,
        languages,
        sources: source_entries(inv),
        source_catalog: CATALOG,
        privacy: Privacy {
            statement: "Everything was assessed on this computer. No files or contents were sent to us or to an AI service.",
            network: "Local assessment · Nothing is uploaded",
        },
    }
}

/// A readable name for a collection when its content says what it is.
pub fn display_label_for(c: &crate::model::Collection, t: &crate::scan::TextTotals) -> String {
    if !t.takeout_products.is_empty() {
        let products: Vec<&str> = t
            .takeout_products
            .iter()
            .map(String::as_str)
            .take(3)
            .collect();
        let more = t.takeout_products.len().saturating_sub(3);
        let mut name = format!("Google Takeout · {}", products.join(", "));
        if more > 0 {
            name.push_str(&format!(" +{more}"));
        }
        // Takeout splits large exports into numbered archives.
        if let Some(part) = takeout_part(&c.label) {
            name.push_str(&format!(" · part {part}"));
        }
        return name;
    }
    if t.git_remotes.len() == 1 {
        let r = t.git_remotes.iter().next().unwrap();
        let repo = r.split_once('/').map_or(r.as_str(), |x| x.1);
        return format!("{repo} (Git repository)");
    }
    c.label.clone()
}

/// "3-001" from "takeout-20261106T153241Z-3-001.zip".
fn takeout_part(label: &str) -> Option<String> {
    let rest = label.strip_prefix("takeout-")?;
    let (_, tail) = rest.split_once('Z')?;
    let tail = tail.trim_start_matches('-');
    let part = tail.split('.').next()?;
    (!part.is_empty()).then(|| part.to_string())
}

/// A repository added as a code download (for example Bitbucket's or
/// GitHub's "Download ZIP"), recognised by matching the archive or folder
/// name against repositories the data links to. Downloads carry no history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSnapshot {
    pub service: &'static str,
    /// "host/owner/repo", lower case.
    pub remote: String,
}

impl CodeSnapshot {
    /// "owner/repo".
    pub fn repo(&self) -> &str {
        self.remote
            .split_once('/')
            .map_or(self.remote.as_str(), |x| x.1)
    }
    pub fn display_label(&self) -> String {
        format!("{} (code download)", self.repo())
    }
}

/// "host/owner/repo" for a linked page on a code host, lower case.
fn repo_of_target(target: &str) -> Option<String> {
    let t = target.to_ascii_lowercase();
    let mut it = t.split('/');
    let (host, owner, repo) = (it.next()?, it.next()?, it.next()?);
    let repo = repo.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{host}/{owner}/{repo}"))
}

/// Names a code download might be saved under, from its archive or folder
/// name: "acme-app-e326f50e8134.zip" (Bitbucket: workspace, repository and
/// commit) and "app-main.zip" (GitHub and GitLab: repository and branch).
fn snapshot_stems(label: &str) -> Vec<String> {
    let mut stem = label.to_ascii_lowercase();
    for ext in [".tar.gz", ".tgz", ".tar.bz2", ".tar", ".zip"] {
        if let Some(s) = stem.strip_suffix(ext) {
            stem = s.to_string();
            break;
        }
    }
    let mut out = Vec::new();
    if let Some((head, tail)) = stem.rsplit_once('-') {
        let is_commit =
            (7..=40).contains(&tail.len()) && tail.bytes().all(|b| b.is_ascii_hexdigit());
        let is_branch = matches!(
            tail,
            "main" | "master" | "develop" | "development" | "trunk"
        );
        if is_commit || is_branch {
            out.push(head.to_string());
        }
    }
    out.push(stem);
    out
}

/// Collections that are code downloads of repositories the data links to.
pub fn code_snapshots(inv: &Inventory) -> HashMap<u32, CodeSnapshot> {
    // Repositories linked to from anywhere in the assessment.
    // Repository -> (service, distinct linked pages in it).
    let mut repos: BTreeMap<String, (&'static str, u64)> = BTreeMap::new();
    for c in &inv.collections {
        let i = c.id as usize;
        for tally in [&inv.slack[i].sources, &inv.text[i].sources] {
            for (&id, st) in &tally.by_service {
                if !matches!(id, "github" | "gitlab" | "bitbucket") {
                    continue;
                }
                for t in &st.targets {
                    if let Some(r) = repo_of_target(t) {
                        repos.entry(r).or_insert((id, 0)).1 += 1;
                    }
                }
            }
        }
    }
    // Collections holding at least one source-code file.
    let mut has_code: BTreeSet<u32> = BTreeSet::new();
    for it in &inv.items {
        let ext = it
            .local_path
            .rsplit('/')
            .next()
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !ext.is_empty()
            && !matches!(ext.as_str(), "json" | "csv" | "tsv" | "xml" | "txt" | "md")
            && language::is_code_or_data_ext(&ext)
        {
            has_code.insert(it.collection);
        }
    }
    let mut out = HashMap::new();
    for c in &inv.collections {
        let t = &inv.text[c.id as usize];
        if !has_code.contains(&c.id)
            || c.source_kind == SourceKind::SlackExport
            || !t.git_remotes.is_empty()
            || !t.takeout_products.is_empty()
        {
            continue;
        }
        let stems = snapshot_stems(&c.label);
        // Prefer "owner-repo" (unambiguous) over a bare repository name.
        let found = repos
            .iter()
            .find(|(r, _)| {
                let mut p = r.splitn(3, '/').skip(1);
                let (o, n) = (p.next().unwrap_or(""), p.next().unwrap_or(""));
                stems.iter().any(|s| *s == format!("{o}-{n}"))
            })
            .or_else(|| {
                // Otherwise the longest repository name the archive name is
                // or ends with ("<anything>-<repo>"); among forks of the same
                // name, the one the data links to most.
                let name = |r: &str| r.rsplit('/').next().unwrap_or("").to_string();
                repos
                    .iter()
                    .filter(|(r, _)| {
                        let n = name(r);
                        stems
                            .iter()
                            .any(|s| *s == n || s.ends_with(&format!("-{n}")))
                    })
                    .max_by_key(|(r, (_, pages))| (name(r).len(), *pages))
            });
        if let Some((r, &(service, _))) = found {
            out.insert(
                c.id,
                CodeSnapshot {
                    service,
                    remote: r.clone(),
                },
            );
        }
    }
    out
}

fn source_entries(inv: &Inventory) -> Vec<SourceEntry> {
    let mut all = SourceTally::default();
    for c in &inv.collections {
        let i = c.id as usize;
        // The Slack recogniser does not know its collection number.
        let mut slack = inv.slack[i].sources.clone();
        for s in slack.by_service.values_mut() {
            s.collections = [c.id].into_iter().collect();
        }
        all.merge(&slack);
        all.merge(&inv.text[i].sources);
    }
    // Sources whose data is already in the assessment.
    let mut present: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for c in &inv.collections {
        if c.source_kind == SourceKind::SlackExport {
            present.entry("slack").or_default().insert(c.label.clone());
        }
    }
    for it in &inv.items {
        if matches!(it.format, Format::Mbox | Format::Eml) && it.status != Status::Excluded {
            present
                .entry("email")
                .or_default()
                .insert(inv.collections[it.collection as usize].label.clone());
        }
    }
    let mut detected: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    let mut remotes: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut cloned: BTreeSet<&'static str> = BTreeSet::new();
    for c in &inv.collections {
        let t = &inv.text[c.id as usize];
        if t.takeout_products.contains("Drive") {
            present
                .entry("google_drive")
                .or_default()
                .insert(c.label.clone());
            detected.insert("google_drive", "Google Takeout (Drive)");
        }
        if t.takeout_products.contains("Mail") {
            present.entry("email").or_default().insert(c.label.clone());
            detected.insert("email", "Google Takeout (Mail)");
        }
        for r in &t.git_remotes {
            let (host, path) = r.split_once('/').unwrap_or((r.as_str(), ""));
            if let Some(id) = crate::analyze::sources::classify(host, &format!("/{path}")) {
                present.entry(id).or_default().insert(c.label.clone());
                detected.insert(id, "a Git repository");
                remotes.entry(id).or_default().push(r.to_ascii_lowercase());
                cloned.insert(id);
            }
        }
    }
    for (cid, snap) in code_snapshots(inv) {
        present
            .entry(snap.service)
            .or_default()
            .insert(inv.collections[cid as usize].label.clone());
        detected.entry(snap.service).or_insert("a code download");
        remotes.entry(snap.service).or_default().push(snap.remote);
    }
    detected.entry("slack").or_insert("a Slack export");
    detected.entry("email").or_insert("a mailbox");
    let label = |c: u32| inv.collections[c as usize].label.clone();
    let mut ids: BTreeSet<&'static str> = all.by_service.keys().copied().collect();
    ids.extend(present.keys().copied());
    let mut out: Vec<(u64, SourceEntry)> = ids
        .into_iter()
        .filter_map(|id| {
            let svc = crate::analyze::sources::service(id)?;
            let st = all.by_service.get(id).cloned().unwrap_or_default();
            let in_selection = present.contains_key(id);
            let distinct = st.targets.len() as u64;
            // Linked pages that sit inside a repository that was added.
            let resolved_targets = remotes.get(id).map(|rs| {
                st.targets
                    .iter()
                    .filter(|t| {
                        let t = t.to_ascii_lowercase();
                        rs.iter()
                            .any(|r| t == *r || t.starts_with(&format!("{r}/")))
                    })
                    .count() as u64
            });
            let other_repositories = remotes.get(id).map(|rs| {
                st.targets
                    .iter()
                    .filter_map(|t| repo_of_target(t))
                    .filter(|r| !rs.contains(r))
                    .collect::<BTreeSet<_>>()
                    .len() as u64
            });
            let snapshot_only = remotes.contains_key(id) && !cloned.contains(id);
            let mut parts = Vec::new();
            if st.links > 0 {
                let noun = if distinct == 1 {
                    svc.target_noun.trim_end_matches('s')
                } else {
                    svc.target_noun
                };
                parts.push(format!(
                    "{} to {} distinct {}",
                    plural(st.links, "link", "links"),
                    group(distinct),
                    noun
                ));
            }
            if st.app_installed {
                parts.push("app installed in Slack".to_string());
            }
            if st.bot_messages > 0 {
                parts.push(format!(
                    "{} from its bot",
                    plural(st.bot_messages, "message", "messages")
                ));
            }
            let collections: Vec<String> = present
                .get(id)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            let evidence = if parts.is_empty() {
                format!("In your map: {}", collections.join(", "))
            } else {
                let mut e = parts.join(" · ");
                e[..1].make_ascii_uppercase();
                e
            };
            let score = if in_selection {
                u64::MAX
            } else {
                distinct * 3
                    + st.links
                    + st.bot_messages / 2
                    + if st.app_installed { 50 } else { 0 }
            };
            Some((
                score,
                SourceEntry {
                    id,
                    name: svc.name,
                    category: svc.category,
                    in_selection,
                    detected_as: if in_selection {
                        detected.get(id).copied()
                    } else {
                        None
                    },
                    resolved_targets,
                    other_repositories,
                    snapshot_only,
                    collections,
                    links: st.links,
                    distinct_targets: distinct,
                    target_noun: svc.target_noun,
                    app_installed: st.app_installed,
                    bot_messages: st.bot_messages,
                    referenced_from: st.collections.iter().map(|&c| label(c)).collect(),
                    evidence,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(b.1.name)));
    out.into_iter().map(|e| e.1).collect()
}

fn language_summary(inv: &Inventory) -> LanguageSummary {
    let mut all = LanguageTally::default();
    let mut by_collection = Vec::new();
    for c in &inv.collections {
        let i = c.id as usize;
        let slack = &inv.slack[i].languages;
        let docs = &inv.text[i].languages;
        let (tally, basis) = if slack.determined_words() + slack.undetermined_words > 0 {
            let mut t = slack.clone();
            t.merge(docs);
            (t, "messages")
        } else {
            (docs.clone(), "documents")
        };
        all.merge(&tally);
        let examined = tally.determined_words() + tally.undetermined_words;
        if examined > 0 {
            by_collection.push(CollectionLanguages {
                label: c.label.clone(),
                basis,
                words_examined: examined,
                undetermined_words: tally.undetermined_words,
                code_or_data_words: tally.non_prose_words,
                rows: tally.rows(),
            });
        }
    }
    let examined = all.determined_words() + all.undetermined_words;
    let classified = all.determined_words();
    let english = all.by_code.get("eng").map_or(0, |v| v.0);
    let mut unreadable = 0;
    let mut non_text = 0;
    for it in &inv.items {
        if it.status == Status::Excluded {
            continue;
        }
        match it.format {
            Format::Pdf | Format::OfficeDocument | Format::OpenDocument => unreadable += 1,
            Format::Image | Format::Audio | Format::Video | Format::Executable => non_text += 1,
            _ => {}
        }
    }
    LanguageSummary {
        method: language::METHOD,
        words_examined: examined,
        undetermined_words: all.undetermined_words,
        undetermined_percent: language::pct(all.undetermined_words, examined),
        // Below this, a percentage would suggest more certainty than there is.
        english_percent: (classified >= 50).then(|| language::pct(english, examined)),
        rows: all.rows(),
        by_collection,
        sampled_documents: all.sampled_documents,
        unreadable_documents: unreadable,
        non_text_files: non_text,
        code_or_data_words: all.non_prose_words,
        code_or_data_files: all.non_prose_files,
    }
}

fn slack_files(inv: &Inventory) -> Option<SlackFiles> {
    let mut images = std::collections::HashSet::new();
    let mut others = std::collections::HashSet::new();
    let mut exports = Vec::new();
    for c in &inv.collections {
        if c.source_kind != SourceKind::SlackExport {
            continue;
        }
        let (i, o) = inv.slack[c.id as usize].referenced_files();
        if i.is_empty() && o.is_empty() {
            continue;
        }
        exports.push(c.label.clone());
        images.extend(i);
        others.extend(o);
    }
    // A file another export shows as an image counts as an image.
    others.retain(|id| !images.contains(id));
    if exports.is_empty() {
        return None;
    }
    let present = |id: &String| {
        inv.attachments.get(id).is_some_and(|ids| {
            ids.iter().any(|&i| {
                !matches!(
                    inv.items[i as usize].status,
                    Status::Excluded | Status::Unavailable
                )
            })
        })
    };
    Some(SlackFiles {
        images: images.len() as u64,
        images_found: images.iter().filter(|id| present(id)).count() as u64,
        documents: others.len() as u64,
        documents_found: others.iter().filter(|id| present(id)).count() as u64,
        exports,
    })
}

fn relationships(inv: &Inventory, collections: &[CollectionSummary]) -> Vec<Relationship> {
    // Attachments present on disk (not excluded or unavailable), by file ID,
    // with the collections they were found in.
    let mut present: HashMap<&str, Vec<u32>> = HashMap::new();
    for (fid, ids) in &inv.attachments {
        for &id in ids {
            let it = &inv.items[id as usize];
            if !matches!(it.status, Status::Excluded | Status::Unavailable) {
                present.entry(fid.as_str()).or_default().push(it.collection);
            }
        }
    }
    let label = |c: u32| inv.collections[c as usize].label.clone();
    let mut out = Vec::new();
    for c in collections {
        let Some(col) = inv.collections.iter().find(|x| x.label == c.label) else {
            continue;
        };
        let (images, others) = inv.slack[col.id as usize].referenced_files();
        let matched = |ids: &std::collections::HashSet<String>| {
            let mut found = std::collections::BTreeSet::new();
            let n = ids
                .iter()
                .filter(|id| match present.get(id.as_str()) {
                    Some(cols) => {
                        found.extend(cols.iter().filter(|&&x| x != col.id).map(|&x| label(x)));
                        true
                    }
                    None => false,
                })
                .count() as u64;
            (n, found.into_iter().collect::<Vec<_>>())
        };
        let (img_ok, img_found) = matched(&images);
        let (doc_ok, doc_found) = matched(&others);
        let n_img = images.len() as u64;
        let n_doc = others.len() as u64;
        if n_img > 0 {
            out.push(Relationship {
                from: c.label.clone(),
                to: "Images shared in conversations",
                kind: "slack_images",
                count: n_img,
                resolved: img_ok,
                available_locally: img_ok >= n_img,
                found_in: img_found,
                explanation: if img_ok == 0 {
                    format!(
                        "{} referred to in messages; the image files are not in the export.",
                        plural(n_img, "image is", "images are")
                    )
                } else {
                    format!(
                        "{} of {} images referred to in messages were found in your selection.",
                        group(img_ok),
                        group(n_img)
                    )
                },
            });
        }
        if n_doc > 0 {
            out.push(Relationship {
                from: c.label.clone(),
                to: "Documents shared in conversations",
                kind: "slack_files",
                count: n_doc,
                resolved: doc_ok,
                available_locally: doc_ok >= n_doc,
                found_in: doc_found,
                explanation: if doc_ok == 0 {
                    format!(
                        "{} referred to in messages; the originals are not in the export.",
                        plural(n_doc, "other file is", "other files are")
                    )
                } else {
                    format!(
                        "{} of {} other files referred to in messages were found in your selection.",
                        group(doc_ok),
                        group(n_doc)
                    )
                },
            });
        }
        let prs: u64 = c
            .slack
            .iter()
            .map(|s| s.distinct_bitbucket_pr_references + s.distinct_github_pr_references)
            .sum();
        if prs > 0 {
            out.push(Relationship {
                from: c.label.clone(),
                to: "Code reviews and repositories",
                kind: "code_reviews",
                count: prs,
                resolved: 0,
                available_locally: false,
                found_in: Vec::new(),
                explanation: format!(
                    "{} linked from conversations. Adding the repositories would connect discussion to code.",
                    plural(prs, "pull request is", "pull requests are")
                ),
            });
        }
    }
    out
}

/// Plain-text rendering of the data map for the terminal.
pub fn render_text(m: &DataMap) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} {} · Your Data Map · {}",
        m.app, m.app_version, m.assessment_id
    );
    let _ = writeln!(s, "{}\n", m.privacy.network);
    for line in &m.summary {
        let _ = writeln!(s, "{line}");
    }
    let _ = writeln!(s, "\nCollections");
    for c in &m.collections {
        let _ = writeln!(s, "  • {} — {}", c.label, c.kind);
        let _ = writeln!(s, "    {}", c.explanation);
        if let Some(d) = &c.date_coverage {
            let _ = writeln!(s, "    Dates: {} to {} ({})", d.first, d.last, d.basis);
        }
    }
    let _ = writeln!(s, "\nCoverage");
    for (k, v) in &m.coverage.by_status {
        let _ = writeln!(s, "  {:<16} {}", k.label(), group(*v));
    }
    if !m.coverage.reasons.is_empty() {
        let _ = writeln!(s, "\nWhy some files were not fully assessed");
        for r in &m.coverage.reasons {
            let _ = writeln!(s, "  {:>8}  {}", group(r.files), r.explanation);
        }
    }
    let _ = writeln!(
        s,
        "\nSize: {} logical files, {} bytes expanded, {} exact duplicate files",
        group(m.totals.logical_files),
        group(m.totals.expanded_bytes),
        group(m.totals.exact_duplicate_files)
    );
    let l = &m.languages;
    if l.words_examined > 0 {
        let _ = writeln!(s, "\nLanguages (words of readable text)");
        for r in l.rows.iter().take(8) {
            let _ = writeln!(
                s,
                "  {:<22} {:>12}  {:>5.1}%",
                r.language,
                group(r.words),
                r.share_percent
            );
        }
        if l.undetermined_words > 0 {
            let _ = writeln!(
                s,
                "  {:<22} {:>12}  {:>5.1}%",
                "Too short to tell",
                group(l.undetermined_words),
                l.undetermined_percent
            );
        }
        if l.code_or_data_words > 0 || l.code_or_data_files > 0 {
            let _ = writeln!(
                s,
                "  Set aside: {} words of code, data or placeholder text; {} code and data files skipped",
                group(l.code_or_data_words),
                group(l.code_or_data_files)
            );
        }
        if l.unreadable_documents > 0 {
            let _ = writeln!(
                s,
                "  Not yet readable: {} PDF and Office documents",
                group(l.unreadable_documents)
            );
        }
    }
    if !m.preparation_priorities.is_empty() {
        let _ = writeln!(s, "\nUseful next steps");
        for (i, p) in m.preparation_priorities.iter().enumerate() {
            let _ = writeln!(s, "  {}. {p}", i + 1);
        }
    }
    let _ = writeln!(s, "\n{}", m.privacy.statement);
    let _ = writeln!(s, "Rules {} · nothing has been sent.", m.ruleset_version);
    s
}
