//! Coverage over time: records per month for each platform, the company's
//! active span, and plain-English warnings where a source starts late,
//! stops early or has a hole, so the owner can see what is missing.
//! Counts per month only.

use serde::Serialize;
use std::collections::BTreeMap;

/// Records per month ("YYYY-MM") for one item, by platform.
pub type ItemMonths = BTreeMap<&'static str, BTreeMap<String, u64>>;

#[derive(Debug, Clone, Serialize)]
pub struct CoverageRow {
    pub platform: &'static str,
    pub name: String,
    pub months: BTreeMap<String, u64>,
    /// First and last busy month (the first and last 0.5% of records are
    /// ignored, so one mis-dated email doesn't stretch the span).
    pub first: Option<String>,
    pub last: Option<String>,
    pub total: u64,
    /// Whether gaps are checked (dense sources: chat, email, code history,
    /// tickets), not sparse ones such as documents.
    pub dense: bool,
    pub items: Vec<CoverageItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageItem {
    pub label: String,
    pub months: BTreeMap<String, u64>,
    pub first: Option<String>,
    pub last: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageGap {
    pub platform: &'static str,
    /// "starts_late", "ends_early" or "gap".
    pub kind: &'static str,
    pub from: String,
    pub to: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Coverage {
    pub rows: Vec<CoverageRow>,
    /// First and last busy month across the dense sources.
    pub company_first: Option<String>,
    pub company_last: Option<String>,
    pub gaps: Vec<CoverageGap>,
}

const DENSE: &[&str] = &[
    "slack",
    "teams",
    "discord",
    "email",
    "github",
    "bitbucket",
    "gitlab",
    "git",
    "jira",
    "linear",
    "zendesk",
];

const ORDER: &[&str] = &[
    "slack",
    "teams",
    "discord",
    "email",
    "github",
    "gitlab",
    "bitbucket",
    "git",
    "jira",
    "linear",
    "zendesk",
    "google_drive",
    "google_other",
    "other",
];

fn ym(key: &str) -> Option<(i32, u32)> {
    let (y, m) = key.split_once('-')?;
    Some((y.parse().ok()?, m.parse().ok()?))
}

fn index(key: &str) -> Option<i64> {
    ym(key).map(|(y, m)| y as i64 * 12 + m as i64 - 1)
}

fn key_of(i: i64) -> String {
    format!("{:04}-{:02}", i.div_euclid(12), i.rem_euclid(12) + 1)
}

/// "June 2021"
pub fn month_name(key: &str) -> String {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    match ym(key) {
        Some((y, m)) if (1..=12).contains(&m) => format!("{} {y}", NAMES[m as usize - 1]),
        _ => key.to_string(),
    }
}

/// First and last month, ignoring the outer 0.5% of records at each end.
fn span(months: &BTreeMap<String, u64>) -> (Option<String>, Option<String>) {
    let total: u64 = months.values().sum();
    if total == 0 {
        return (None, None);
    }
    let cut = total / 200;
    let mut acc = 0;
    let mut first = None;
    for (k, v) in months {
        acc += v;
        if acc > cut && *v > 0 {
            first = Some(k.clone());
            break;
        }
    }
    let mut acc = 0;
    let mut last = None;
    for (k, v) in months.iter().rev() {
        acc += v;
        if acc > cut && *v > 0 {
            last = Some(k.clone());
            break;
        }
    }
    (first, last)
}

pub fn platform_name(id: &str) -> String {
    match id {
        "google_other" => "Google (other products)".into(),
        "google_drive" => "Google Drive".into(),
        "git" => "Code history".into(),
        "other" => "Other files".into(),
        "email" => "Email".into(),
        _ => crate::analyze::sources::service(id)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| id.to_string()),
    }
}

/// Builds the coverage rows and gap warnings from each item's months.
pub fn build(items: &[(String, ItemMonths)]) -> Coverage {
    let mut rows: BTreeMap<&'static str, CoverageRow> = BTreeMap::new();
    for (label, by_platform) in items {
        for (platform, months) in by_platform {
            if months.values().all(|v| *v == 0) {
                continue;
            }
            let row = rows.entry(platform).or_insert_with(|| CoverageRow {
                platform,
                name: platform_name(platform),
                months: BTreeMap::new(),
                first: None,
                last: None,
                total: 0,
                dense: DENSE.contains(platform),
                items: Vec::new(),
            });
            for (k, v) in months {
                *row.months.entry(k.clone()).or_default() += v;
            }
            let (first, last) = span(months);
            row.items.push(CoverageItem {
                label: label.clone(),
                months: months.clone(),
                first,
                last,
            });
        }
    }
    let mut out = Coverage::default();
    let mut all_dense: BTreeMap<String, u64> = BTreeMap::new();
    for r in rows.values_mut() {
        r.total = r.months.values().sum();
        let (f, l) = span(&r.months);
        r.first = f;
        r.last = l;
        if r.dense {
            for (k, v) in &r.months {
                *all_dense.entry(k.clone()).or_default() += v;
            }
        }
    }
    let (cf, cl) = span(&all_dense);
    out.company_first = cf.clone();
    out.company_last = cl.clone();
    let mut rows: Vec<CoverageRow> = rows.into_values().collect();
    rows.sort_by_key(|r| ORDER.iter().position(|p| *p == r.platform).unwrap_or(99));
    // Gaps, for dense sources with enough records to tell.
    if let (Some(cf), Some(cl)) = (cf.as_deref().and_then(index), cl.as_deref().and_then(index)) {
        let several = rows.iter().filter(|r| r.dense && r.total >= 20).count() >= 2;
        for r in rows.iter().filter(|r| r.dense && r.total >= 20) {
            let (Some(pf), Some(pl)) = (
                r.first.as_deref().and_then(index),
                r.last.as_deref().and_then(index),
            ) else {
                continue;
            };
            // Only compare against the rest when there is a rest.
            if several && pf - cf > 3 {
                out.gaps.push(CoverageGap {
                    platform: r.platform,
                    kind: "starts_late",
                    from: key_of(cf),
                    to: key_of(pf),
                    text: format!(
                        "{} starts in {}, but your other data goes back to {}. Is an earlier export missing?",
                        r.name,
                        month_name(&key_of(pf)),
                        month_name(&key_of(cf))
                    ),
                });
            }
            if several && cl - pl > 3 {
                out.gaps.push(CoverageGap {
                    platform: r.platform,
                    kind: "ends_early",
                    from: key_of(pl),
                    to: key_of(cl),
                    text: format!(
                        "{} stops in {}, but your other data runs to {}. Is a later export missing?",
                        r.name,
                        month_name(&key_of(pl)),
                        month_name(&key_of(cl))
                    ),
                });
            }
            // Holes inside the platform's own span.
            let mut busy: Vec<u64> = r.months.values().copied().filter(|v| *v > 0).collect();
            busy.sort_unstable();
            let median = busy.get(busy.len() / 2).copied().unwrap_or(0);
            if median < 5 {
                continue;
            }
            let mut holes: Vec<CoverageGap> = Vec::new();
            let mut run_start: Option<i64> = None;
            for i in pf..=pl + 1 {
                let empty = i <= pl && r.months.get(&key_of(i)).copied().unwrap_or(0) == 0;
                match (empty, run_start) {
                    (true, None) => run_start = Some(i),
                    (false, Some(s)) => {
                        if i - s >= 3 {
                            holes.push(CoverageGap {
                                platform: r.platform,
                                kind: "gap",
                                from: key_of(s),
                                to: key_of(i - 1),
                                text: format!(
                                    "{} has nothing from {} to {} ({} months). Was that period left out of the export?",
                                    r.name,
                                    month_name(&key_of(s)),
                                    month_name(&key_of(i - 1)),
                                    i - s
                                ),
                            });
                        }
                        run_start = None;
                    }
                    _ => {}
                }
            }
            // Many holes: one warning that the source is patchy.
            if holes.len() > 2 {
                let empty: i64 = holes
                    .iter()
                    .filter_map(|g| Some(index(&g.to)? - index(&g.from)? + 1))
                    .sum();
                out.gaps.push(CoverageGap {
                    platform: r.platform,
                    kind: "patchy",
                    from: holes[0].from.clone(),
                    to: holes[holes.len() - 1].to.clone(),
                    text: format!(
                        "{} is patchy: {} gaps of 3 months or more ({} empty months in all, the first from {} to {}). Was it exported with a date filter or in parts?",
                        r.name,
                        holes.len(),
                        empty,
                        month_name(&holes[0].from),
                        month_name(&holes[0].to)
                    ),
                });
            } else {
                out.gaps.extend(holes);
            }
        }
    }
    out.rows = rows;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn months(
        from: (i32, u32),
        to: (i32, u32),
        n: u64,
        skip: &[(i32, u32)],
    ) -> BTreeMap<String, u64> {
        let (a, b) = (
            from.0 as i64 * 12 + from.1 as i64 - 1,
            to.0 as i64 * 12 + to.1 as i64 - 1,
        );
        (a..=b)
            .filter(|i| {
                !skip
                    .iter()
                    .any(|(y, m)| *y as i64 * 12 + *m as i64 - 1 == *i)
            })
            .map(|i| (key_of(i), n))
            .collect()
    }

    fn item(label: &str, platform: &'static str, m: BTreeMap<String, u64>) -> (String, ItemMonths) {
        (label.to_string(), BTreeMap::from([(platform, m)]))
    }

    #[test]
    fn email_that_stops_early_is_flagged() {
        let c = build(&[
            item(
                "slack.zip",
                "slack",
                months((2016, 9), (2025, 11), 300, &[]),
            ),
            item("mail.mbox", "email", months((2016, 9), (2021, 6), 200, &[])),
        ]);
        assert_eq!(c.company_first.as_deref(), Some("2016-09"));
        assert_eq!(c.company_last.as_deref(), Some("2025-11"));
        assert_eq!(c.gaps.len(), 1, "{:?}", c.gaps);
        let g = &c.gaps[0];
        assert_eq!((g.platform, g.kind), ("email", "ends_early"));
        assert!(
            g.text.contains("June 2021") && g.text.contains("November 2025"),
            "{}",
            g.text
        );
        assert_eq!(c.rows[0].platform, "slack");
    }

    #[test]
    fn holes_are_flagged_and_sparse_documents_are_not() {
        let hole: Vec<(i32, u32)> = (3..=8).map(|m| (2019, m)).collect();
        let c = build(&[
            item(
                "slack.zip",
                "slack",
                months((2018, 1), (2020, 12), 100, &hole),
            ),
            item(
                "drive",
                "google_drive",
                BTreeMap::from([("2015-01".to_string(), 1), ("2024-05".to_string(), 2)]),
            ),
        ]);
        assert_eq!(c.gaps.len(), 1, "{:?}", c.gaps);
        assert_eq!(
            (
                c.gaps[0].kind,
                c.gaps[0].from.as_str(),
                c.gaps[0].to.as_str()
            ),
            ("gap", "2019-03", "2019-08")
        );
        // Drive is shown but not dense; it doesn't stretch the company span.
        assert!(c
            .rows
            .iter()
            .any(|r| r.platform == "google_drive" && !r.dense));
        assert_eq!(c.company_first.as_deref(), Some("2018-01"));
    }

    #[test]
    fn many_holes_make_one_patchy_warning() {
        // Only January of each year.
        let m: BTreeMap<String, u64> = (2015..=2023).map(|y| (format!("{y}-01"), 50)).collect();
        let c = build(&[item("mail", "email", m)]);
        assert_eq!(c.gaps.len(), 1, "{:?}", c.gaps);
        assert_eq!(c.gaps[0].kind, "patchy");
        assert!(c.gaps[0].text.contains("8 gaps"), "{}", c.gaps[0].text);
    }

    #[test]
    fn one_stray_date_does_not_stretch_a_span() {
        let mut m = months((2018, 1), (2020, 12), 100, &[]);
        m.insert("1999-01".into(), 1);
        let c = build(&[item("mail", "email", m)]);
        assert_eq!(c.rows[0].first.as_deref(), Some("2018-01"));
    }
}
