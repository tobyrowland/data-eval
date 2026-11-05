//! Owner-facing "Your Data Map" view derived from the private inventory.
//!
//! The data map carries collection labels the owner chose, counts, coverage
//! and template sentences. It never carries local paths, content hashes,
//! message text, channel names, account IDs or URLs (spec §8).

use crate::analyze::slack::{civil_from_days, SlackInventory};
use crate::model::{empty_status_counts, Format, Reason, SourceKind, Status};
use crate::scan::Inventory;
use crate::{APP_NAME, APP_VERSION, RULESET_VERSION};
use rand::RngCore;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
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
    /// Content-derived activity per calendar month (YYYY-MM), all collections.
    pub activity: BTreeMap<String, u64>,
    pub stopped_by_user: bool,
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
    pub label: String,
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
    format!("DPS-{}-{}-{}", &hex[0..4], &hex[4..8], &hex[8..12])
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
                    a
                });
                format!(
                    "A Slack workspace export with {} across {}, including {}. It refers to {} not included in the export.",
                    plural(s.message_records, "message record", "message records"),
                    plural(s.channels_in_metadata, "channel", "channels"),
                    plural(s.explicit_threads, "thread", "threads"),
                    plural(s.distinct_file_references, "shared file", "shared files"),
                )
            }
            SourceKind::Generic => format!(
                "{} containing {}.",
                plural(col_files[i], "file", "files"),
                plural(col_records[i], "structured record", "structured records")
            ),
        };
        collections.push(CollectionSummary {
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
                "Add the original attachments: {} referenced by conversations {} no local copy. The DataPack Scout Attachment Downloader can fetch them from Slack for you.",
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
        activity,
        stopped_by_user: inv.cancelled,
        privacy: Privacy {
            statement: "Everything was assessed on this computer. No files or contents were sent to us or to an AI service.",
            network: "Local assessment · Nothing is uploaded",
        },
    }
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
