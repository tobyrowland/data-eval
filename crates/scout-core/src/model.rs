//! Core data model: logical items, their explicit coverage status and the
//! aggregate inventory. Every accessible item gets exactly one [`Status`];
//! nothing silently disappears from the totals (spec §4).

use serde::Serialize;
use std::collections::BTreeMap;

/// Explicit per-item coverage status (spec §4, "Every item has one explicit status").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Supported content was fully examined within the configured scope.
    Assessed,
    /// Some content was examined, with the coverage and reason recorded.
    PartlyAssessed,
    /// Counted and identified; contents need a different parser.
    Inventoried,
    /// Encryption or access permissions prevented reading content.
    Locked,
    /// A referenced file, online-only placeholder or original is absent.
    Unavailable,
    /// The user excluded the item from this assessment.
    Excluded,
    /// A limit, corrupt structure or unsafe parser behaviour stopped reading.
    StoppedSafely,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Assessed,
        Status::PartlyAssessed,
        Status::Inventoried,
        Status::Locked,
        Status::Unavailable,
        Status::Excluded,
        Status::StoppedSafely,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Status::Assessed => "Assessed",
            Status::PartlyAssessed => "Partly assessed",
            Status::Inventoried => "Inventoried",
            Status::Locked => "Locked",
            Status::Unavailable => "Unavailable",
            Status::Excluded => "Excluded",
            Status::StoppedSafely => "Stopped safely",
        }
    }

    /// Whether any content of the item was examined.
    pub fn content_examined(self) -> bool {
        matches!(self, Status::Assessed | Status::PartlyAssessed)
    }
}

/// Detected format family. Detection uses signatures and structure before names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Zip,
    Tar,
    Gzip,
    SevenZip,
    Rar,
    OfficeDocument,
    OpenDocument,
    Pdf,
    Json,
    Jsonl,
    Csv,
    Tsv,
    Xml,
    Html,
    Markdown,
    PlainText,
    Mbox,
    Eml,
    Sqlite,
    Image,
    Audio,
    Video,
    Executable,
    Pst,
    Unknown,
}

impl Format {
    pub fn is_container(self) -> bool {
        matches!(
            self,
            Format::Zip | Format::Tar | Format::Gzip | Format::SevenZip | Format::Rar
        )
    }

    /// Broad family used to group formats on the data map.
    pub fn category(self) -> &'static str {
        match self {
            Format::Csv | Format::Tsv => "Tables",
            Format::Json | Format::Jsonl | Format::Xml | Format::Sqlite => "Structured data",
            Format::OfficeDocument
            | Format::OpenDocument
            | Format::Pdf
            | Format::Html
            | Format::Markdown
            | Format::PlainText => "Documents",
            Format::Mbox | Format::Eml => "Email",
            Format::Image | Format::Audio | Format::Video => "Media",
            Format::Zip | Format::Tar | Format::Gzip | Format::SevenZip | Format::Rar => "Archives",
            Format::Executable | Format::Pst | Format::Unknown => "Other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP archive",
            Format::Tar => "TAR archive",
            Format::Gzip => "GZIP stream",
            Format::SevenZip => "7z archive",
            Format::Rar => "RAR archive",
            Format::OfficeDocument => "Office document",
            Format::OpenDocument => "OpenDocument file",
            Format::Pdf => "PDF",
            Format::Json => "JSON",
            Format::Jsonl => "JSON Lines",
            Format::Csv => "CSV",
            Format::Tsv => "TSV",
            Format::Xml => "XML",
            Format::Html => "HTML",
            Format::Markdown => "Markdown",
            Format::PlainText => "Plain text",
            Format::Mbox => "MBOX mailbox",
            Format::Eml => "Email message",
            Format::Sqlite => "SQLite database",
            Format::Image => "Image",
            Format::Audio => "Audio",
            Format::Video => "Video",
            Format::Executable => "Executable",
            Format::Pst => "Outlook PST",
            Format::Unknown => "Unrecognised",
        }
    }
}

/// One logical item: a file on disk or a member of an archive.
///
/// `local_path` is private to the local workspace and must never enter a
/// shared packet or the owner PDF.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: u64,
    pub parent: Option<u64>,
    pub collection: u32,
    pub local_path: String,
    pub archive_depth: u8,
    pub format: Format,
    pub status: Status,
    pub reason: Option<Reason>,
    /// Bytes as stored (compressed size for archive members when known).
    pub stored_bytes: u64,
    /// Bytes after expansion that were actually observed or declared.
    pub expanded_bytes: u64,
    /// Lowercase hex SHA-256 of full content; local only, for exact dedup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Structured record count, when the parser produced one.
    pub records: Option<u64>,
}

/// Machine-readable reason attached to non-`Assessed` statuses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NoParserInThisRelease,
    PasswordProtected,
    PermissionDenied,
    OnlineOnlyPlaceholder,
    UserExcluded,
    NestingLimit,
    ExpansionLimit,
    CompressionRatio,
    MemberCountLimit,
    LogicalFileLimit,
    RecordLimit,
    ObjectTooLarge,
    CorruptStructure,
    UnsafeMemberPath,
    SymlinkNotFollowed,
    ChangedDuringScan,
    StoppedByUser,
    ReadError,
}

impl Reason {
    pub fn explain(&self) -> &'static str {
        match self {
            Reason::NoParserInThisRelease => "contents need a parser that is not in this release",
            Reason::PasswordProtected => "password protected; add a locally decrypted export",
            Reason::PermissionDenied => "the operating system denied read access",
            Reason::OnlineOnlyPlaceholder => "online-only placeholder; make a local copy first",
            Reason::UserExcluded => "excluded by you",
            Reason::NestingLimit => "archive nesting limit reached",
            Reason::ExpansionLimit => "expanded-size budget reached",
            Reason::CompressionRatio => "suspicious compression ratio",
            Reason::MemberCountLimit => "archive member-count limit reached",
            Reason::LogicalFileLimit => "logical-file limit reached",
            Reason::RecordLimit => "structured-record limit reached",
            Reason::ObjectTooLarge => "object larger than the parser limit",
            Reason::CorruptStructure => "damaged or unreadable structure",
            Reason::UnsafeMemberPath => "archive member path escapes its container",
            Reason::SymlinkNotFollowed => "symbolic link not followed",
            Reason::ChangedDuringScan => "changed while being read; run the assessment again",
            Reason::StoppedByUser => "you stopped the assessment before this was read",
            Reason::ReadError => "read error",
        }
    }
}

/// A user selection (file, folder or archive) and what was recognised in it.
#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub id: u32,
    /// Owner-facing label (the selection's file name). Local only.
    pub label: String,
    pub source_kind: SourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    SlackExport,
    Generic,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::SlackExport => "Slack export",
            SourceKind::Generic => "Files and documents",
        }
    }
}

/// Counter keyed by status, always containing every status (zeros included).
pub fn empty_status_counts() -> BTreeMap<Status, u64> {
    Status::ALL.iter().map(|s| (*s, 0)).collect()
}
