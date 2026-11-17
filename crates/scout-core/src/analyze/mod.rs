//! Content parsers for the first release. Each parser streams from a reader,
//! never executes content and never follows links.

pub mod discord;
pub mod docs;
pub mod fingerprint;
pub mod language;
pub mod slack;
pub mod sources;
pub mod tabular;
pub mod teams;
pub mod text;

use crate::model::{Reason, Status};

/// Result of examining one logical file's content.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: Status,
    pub reason: Option<Reason>,
    pub records: Option<u64>,
    pub text_bytes: u64,
    pub words: u64,
    /// Parsed JSON document, handed on to source recognisers.
    pub json: Option<serde_json::Value>,
    /// Leading text of a document for language detection, and whether the
    /// document was longer than the sample. Local only.
    pub sample: Option<(String, bool)>,
    /// Hashed Message-IDs of the emails in a mailbox. Local only.
    pub message_ids: Vec<u64>,
    /// Addresses in mailbox From/To/Cc headers. Local only.
    pub addresses: Vec<String>,
    /// A ticket tracker's CSV export (Jira, Linear, Zendesk), recognised
    /// by its header row, and whether its rows are Working Minds test data.
    pub tracker: Option<(&'static str, bool)>,
    /// Records or documents per calendar month ("YYYY-MM"), from dates
    /// inside the content (email Date headers, ticket Created dates,
    /// document metadata).
    pub months: std::collections::BTreeMap<String, u64>,
}

impl Outcome {
    pub fn assessed(records: Option<u64>) -> Self {
        Outcome {
            status: Status::Assessed,
            reason: None,
            records,
            text_bytes: 0,
            words: 0,
            json: None,
            sample: None,
            message_ids: Vec::new(),
            addresses: Vec::new(),
            tracker: None,
            months: std::collections::BTreeMap::new(),
        }
    }
    pub fn inventoried() -> Self {
        Outcome {
            status: Status::Inventoried,
            reason: Some(Reason::NoParserInThisRelease),
            records: None,
            text_bytes: 0,
            words: 0,
            json: None,
            sample: None,
            message_ids: Vec::new(),
            addresses: Vec::new(),
            tracker: None,
            months: std::collections::BTreeMap::new(),
        }
    }
    pub fn with(status: Status, reason: Reason) -> Self {
        Outcome {
            status,
            reason: Some(reason),
            records: None,
            text_bytes: 0,
            words: 0,
            json: None,
            sample: None,
            message_ids: Vec::new(),
            addresses: Vec::new(),
            tracker: None,
            months: std::collections::BTreeMap::new(),
        }
    }
}

/// "YYYY-MM" for a year and month.
pub fn month_key(y: i32, m: u32) -> String {
    format!("{y:04}-{m:02}")
}

/// The year and month in a date written as RFC 2822 ("Tue, 5 Jan 2021
/// 10:00:00 +0000"), an mbox From line ("Mon Jan 01 10:00:00 2018"), ISO
/// 8601 ("2021-03-05 10:00") or a compact PDF date ("D:20190314").
pub fn month_of_date(s: &str) -> Option<(i32, u32)> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let ok_year = |y: i32| (1980..=2100).contains(&y);
    let mut month = None;
    let mut year = None;
    for tok in s.split(|c: char| !c.is_alphanumeric() && c != '-') {
        // ISO date: YYYY-MM(-DD)
        let parts: Vec<&str> = tok.split('-').collect();
        if parts.len() >= 2 && parts[0].len() == 4 && parts[1].len() == 2 {
            if let (Ok(y), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
                if ok_year(y) && (1..=12).contains(&m) {
                    return Some((y, m));
                }
            }
        }
        for t in tok.split('-') {
            let lower = t.to_ascii_lowercase();
            if month.is_none() && t.len() >= 3 && t.chars().all(|c| c.is_ascii_alphabetic()) {
                if let Some(i) = MONTHS.iter().position(|m| lower.starts_with(m)) {
                    month = Some(i as u32 + 1);
                    continue;
                }
            }
            if year.is_none() && t.len() == 4 {
                if let Ok(y) = t.parse::<i32>() {
                    if ok_year(y) {
                        year = Some(y);
                    }
                }
            }
            // Compact: D:YYYYMM… or YYYYMMDD
            let digits = t.trim_start_matches("D:").trim_start_matches('D');
            if year.is_none()
                && digits.len() >= 6
                && digits.bytes().take(6).all(|b| b.is_ascii_digit())
            {
                if let (Ok(y), Ok(m)) = (digits[..4].parse::<i32>(), digits[4..6].parse::<u32>()) {
                    if ok_year(y) && (1..=12).contains(&m) {
                        return Some((y, m));
                    }
                }
            }
        }
    }
    year.zip(month)
}

#[cfg(test)]
mod date_tests {
    use super::month_of_date;

    #[test]
    fn months_from_common_date_formats() {
        assert_eq!(
            month_of_date("Tue, 5 Jan 2021 10:00:00 +0000"),
            Some((2021, 1))
        );
        assert_eq!(
            month_of_date("From a@x Mon Jan 01 10:00:00 2018"),
            Some((2018, 1))
        );
        assert_eq!(month_of_date("2021-03-05 10:00"), Some((2021, 3)));
        assert_eq!(month_of_date("05/Mar/21 10:00 AM"), None);
        assert_eq!(month_of_date("D:20190314120000Z"), Some((2019, 3)));
        assert_eq!(month_of_date("no date here"), None);
    }
}
