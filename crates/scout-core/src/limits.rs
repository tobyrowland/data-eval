//! Capacity and parser limits (spec §5). These are proposed launch defaults
//! that still require benchmarking on representative machines.

use serde::Serialize;

const GB: u64 = 1024 * 1024 * 1024;
const MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Limits {
    /// Total expanded logical input, measured before deduplication.
    pub max_expanded_bytes: u64,
    pub max_logical_files: u64,
    pub max_structured_records: u64,
    /// Largest individual archive accepted for content inspection (compressed).
    pub max_archive_bytes: u64,
    /// Number of nested archive levels inside a selected archive.
    pub max_archive_nesting: u8,
    /// Members enumerated per archive before stopping safely.
    pub max_archive_members: u64,
    /// Expanded/compressed ratio above which a member is treated as a bomb,
    /// once it has expanded beyond `ratio_check_floor_bytes`.
    pub max_compression_ratio: u64,
    pub ratio_check_floor_bytes: u64,
    /// Largest nested archive buffered in memory for random access.
    pub max_in_memory_archive_bytes: u64,
    /// Largest single JSON document parsed as a whole.
    pub max_json_document_bytes: u64,
    /// Largest single text/tabular object streamed by a content parser.
    pub max_content_object_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_expanded_bytes: 100 * GB,
            max_logical_files: 250_000,
            max_structured_records: 50_000_000,
            max_archive_bytes: 50 * GB,
            max_archive_nesting: 3,
            max_archive_members: 250_000,
            max_compression_ratio: 200,
            ratio_check_floor_bytes: 64 * MB,
            max_in_memory_archive_bytes: 256 * MB,
            max_json_document_bytes: 128 * MB,
            max_content_object_bytes: 8 * GB,
        }
    }
}
