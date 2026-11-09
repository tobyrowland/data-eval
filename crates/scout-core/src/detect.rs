//! Format detection from file signatures and content structure, with names
//! used only as a fallback or tie-breaker (spec §4).

use crate::model::Format;

/// Number of leading bytes callers should supply to [`detect`].
pub const SNIFF_LEN: usize = 4096;

pub fn detect(name: &str, head: &[u8]) -> Format {
    let ext = extension(name);
    if let Some(f) = by_signature(head, &ext) {
        return f;
    }
    by_text_structure(head, &ext).unwrap_or_else(|| by_extension(&ext))
}

pub fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => base[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn by_signature(h: &[u8], ext: &str) -> Option<Format> {
    let starts = |sig: &[u8]| h.starts_with(sig);
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        // OOXML/ODF packages are ZIPs but represent one logical document.
        return Some(match ext {
            "docx" | "docm" | "xlsx" | "xlsm" | "pptx" | "pptm" => Format::OfficeDocument,
            "odt" | "ods" | "odp" => Format::OpenDocument,
            _ if contains(h, b"[Content_Types].xml") => Format::OfficeDocument,
            _ if contains(h, b"mimetypeapplication/vnd.oasis.opendocument") => Format::OpenDocument,
            _ => Format::Zip,
        });
    }
    if starts(b"\x1f\x8b") {
        return Some(Format::Gzip);
    }
    if h.len() >= 262 && &h[257..262] == b"ustar" {
        return Some(Format::Tar);
    }
    if starts(b"7z\xBC\xAF\x27\x1C") {
        return Some(Format::SevenZip);
    }
    if starts(b"Rar!\x1A\x07") {
        return Some(Format::Rar);
    }
    if starts(b"%PDF-") {
        return Some(Format::Pdf);
    }
    if starts(b"SQLite format 3\0") {
        return Some(Format::Sqlite);
    }
    if starts(b"!BDN") {
        return Some(Format::Pst);
    }
    if starts(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
        // Legacy OLE compound file: .doc/.xls/.ppt/.msg.
        return Some(Format::OfficeDocument);
    }
    if starts(b"\x89PNG\r\n\x1a\n")
        || starts(b"\xFF\xD8\xFF")
        || starts(b"GIF87a")
        || starts(b"GIF89a")
        || (starts(b"RIFF") && h.len() >= 12 && &h[8..12] == b"WEBP")
        || starts(b"II*\0")
        || starts(b"MM\0*")
    {
        return Some(Format::Image);
    }
    if starts(b"ID3")
        || starts(b"fLaC")
        || starts(b"OggS")
        || (starts(b"RIFF") && h.len() >= 12 && &h[8..12] == b"WAVE")
    {
        return Some(Format::Audio);
    }
    if h.len() >= 12 && &h[4..8] == b"ftyp" {
        return Some(if matches!(ext, "m4a" | "aac") {
            Format::Audio
        } else {
            Format::Video
        });
    }
    if starts(b"\x1A\x45\xDF\xA3") {
        return Some(Format::Video);
    }
    if starts(b"MZ")
        || starts(b"\x7FELF")
        || starts(b"\xCF\xFA\xED\xFE")
        || starts(b"\xCE\xFA\xED\xFE")
        || starts(b"\xCA\xFE\xBA\xBE")
    {
        return Some(Format::Executable);
    }
    None
}

fn by_text_structure(h: &[u8], ext: &str) -> Option<Format> {
    if h.is_empty() || !looks_textual(h) {
        return None;
    }
    let text = String::from_utf8_lossy(h);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("From ")
        && (ext == "mbox" || ext.is_empty() || t.contains("\nFrom:") || t.contains("\nDate:"))
    {
        return Some(Format::Mbox);
    }
    if ext == "eml" || (looks_like_rfc822(t) && !matches!(ext, "txt" | "md")) {
        return Some(Format::Eml);
    }
    if ext == "jsonl" || ext == "ndjson" {
        return Some(Format::Jsonl);
    }
    // "[" opens JSON only when followed by a value; "[core]" is an INI
    // section header (e.g. a .git/config), not an array.
    let json_array = t.starts_with('[')
        && t[1..].trim_start().chars().next().is_some_and(|c| {
            matches!(c, '{' | '[' | '"' | ']' | '-' | '0'..='9' | 't' | 'f' | 'n')
        });
    if t.starts_with('{') || json_array {
        let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).take(3).collect();
        let multi_object_lines = lines.len() >= 2
            && lines.iter().all(|l| {
                let l = l.trim();
                l.starts_with('{') && l.ends_with('}')
            });
        return Some(if multi_object_lines {
            Format::Jsonl
        } else {
            Format::Json
        });
    }
    let lower = t.get(..t.len().min(512)).unwrap_or("").to_ascii_lowercase();
    if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
        return Some(Format::Html);
    }
    if lower.starts_with("<?xml") || (lower.starts_with('<') && ext == "xml") {
        return Some(Format::Xml);
    }
    match ext {
        "csv" => Some(Format::Csv),
        "tsv" | "tab" => Some(Format::Tsv),
        "md" | "markdown" => Some(Format::Markdown),
        "htm" | "html" => Some(Format::Html),
        _ => Some(Format::PlainText),
    }
}

fn by_extension(ext: &str) -> Format {
    match ext {
        "zip" => Format::Zip,
        "tar" => Format::Tar,
        "gz" | "tgz" => Format::Gzip,
        "7z" => Format::SevenZip,
        "rar" => Format::Rar,
        "pst" | "ost" => Format::Pst,
        "exe" | "dll" | "msi" | "dmg" | "pkg" | "app" => Format::Executable,
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "webp" | "tif" | "tiff" | "bmp" | "svg" => {
            Format::Image
        }
        "mp3" | "wav" | "m4a" | "flac" | "ogg" => Format::Audio,
        "mp4" | "mov" | "mkv" | "avi" | "webm" => Format::Video,
        _ => Format::Unknown,
    }
}

fn looks_textual(h: &[u8]) -> bool {
    let sample = &h[..h.len().min(1024)];
    if sample.contains(&0) {
        return false;
    }
    let ctrl = sample
        .iter()
        .filter(|b| **b < 0x09 || (**b > 0x0D && **b < 0x20))
        .count();
    ctrl * 100 <= sample.len().max(1) * 2
}

fn looks_like_rfc822(t: &str) -> bool {
    let mut header_lines = 0;
    for line in t.lines().take(20) {
        if line.is_empty() {
            break;
        }
        let key = line.split(':').next().unwrap_or("");
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "from"
                | "to"
                | "subject"
                | "date"
                | "message-id"
                | "received"
                | "mime-version"
                | "return-path"
        ) {
            header_lines += 1;
        }
    }
    header_lines >= 3
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_beat_names() {
        assert_eq!(detect("report.txt", b"%PDF-1.7\n"), Format::Pdf);
        assert_eq!(detect("notes", b"PK\x03\x04rest"), Format::Zip);
        assert_eq!(
            detect("letter.docx", b"PK\x03\x04rest"),
            Format::OfficeDocument
        );
        assert_eq!(detect("a.bin", b"\x1f\x8b\x08"), Format::Gzip);
        assert_eq!(detect("pic.dat", b"\x89PNG\r\n\x1a\nxxxx"), Format::Image);
    }

    #[test]
    fn text_structure() {
        assert_eq!(detect("x", b"[{\"a\":1}]"), Format::Json);
        assert_eq!(detect("x", b"[ 1, 2 ]"), Format::Json);
        assert_eq!(
            detect("app/.git/config", b"[core]\n\tbare = false\n"),
            Format::PlainText
        );
        assert_eq!(detect("x", b"{\"a\":1}\n{\"a\":2}\n"), Format::Jsonl);
        assert_eq!(detect("data.csv", b"a,b\n1,2\n"), Format::Csv);
        assert_eq!(
            detect("mail", b"From a@b Mon Jan 1\nFrom: x\nDate: y\n\nhi"),
            Format::Mbox
        );
        assert_eq!(
            detect("m", b"From: a\nTo: b\nSubject: c\nDate: d\n\nbody"),
            Format::Eml
        );
        assert_eq!(
            detect("blob.unknownext", &[0u8, 1, 2, 3, 200, 201]),
            Format::Unknown
        );
    }
}
