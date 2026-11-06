//! Content parsers for the first release. Each parser streams from a reader,
//! never executes content and never follows links.

pub mod language;
pub mod slack;
pub mod sources;
pub mod tabular;
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
        }
    }
}
