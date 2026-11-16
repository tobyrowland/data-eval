//! Text from Office documents (Word, PowerPoint, Excel), OpenDocument
//! files and PDFs. Read on this computer only; nothing is rendered and no
//! embedded object, macro or link is followed.
//!
//! Office and OpenDocument files are ZIP packages of XML: the text is
//! taken from the document parts (`word/document.xml`, slides and notes,
//! shared strings, `content.xml`). PDFs are read with `pdf-extract`, which
//! decodes the page content streams and fonts. Scanned PDFs (images of
//! pages), encrypted files and legacy binary Office files (.doc, .xls,
//! .ppt) have no readable text here.

use crate::model::Format;
use std::io::{Cursor, Read};

/// The largest document read (bigger ones are inventoried only).
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
/// The largest single XML part read from a package.
const MAX_PART_BYTES: u64 = 32 * 1024 * 1024;

/// A document's text.
#[derive(Debug, Clone)]
pub struct DocText {
    pub text: String,
    /// Spreadsheet cells rather than prose.
    pub sheet: bool,
}

/// Reads a document's text. None when it has none we can read.
pub fn extract(format: Format, bytes: &[u8]) -> Option<DocText> {
    let out = match format {
        Format::Pdf => pdf(bytes).map(|text| DocText { text, sheet: false }),
        Format::OfficeDocument | Format::OpenDocument if bytes.starts_with(b"PK") => package(bytes),
        _ => None,
    }?;
    let words = out.text.split_whitespace().count();
    (words > 0).then_some(out)
}

fn pdf(bytes: &[u8]) -> Option<String> {
    // The parser can panic on malformed files: a bad PDF must never stop
    // the assessment.
    let bytes = bytes.to_vec();
    let res = std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&bytes));
    let text = res.ok()?.ok()?;
    // Collapse the layout spacing pdf-extract keeps.
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank == 1 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    Some(out)
}

fn package(bytes: &[u8]) -> Option<DocText> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    let mut read = |name: &str| -> Option<String> {
        let f = zip.by_name(name).ok()?;
        let mut s = String::new();
        f.take(MAX_PART_BYTES).read_to_string(&mut s).ok()?;
        Some(s)
    };
    // Word
    if has("word/document.xml") {
        let mut text = xml_text(&read("word/document.xml")?, &["w:t"], &["w:p"], &["w:tab"]);
        for extra in [
            "word/footnotes.xml",
            "word/endnotes.xml",
            "word/comments.xml",
        ] {
            if let Some(x) = read(extra) {
                text.push_str(&xml_text(&x, &["w:t"], &["w:p"], &["w:tab"]));
            }
        }
        return Some(DocText { text, sheet: false });
    }
    // PowerPoint: slides, then speaker notes, in slide order.
    let mut slides = numbered(&names, "ppt/slides/slide");
    if !slides.is_empty() {
        slides.extend(numbered(&names, "ppt/notesSlides/notesSlide"));
        let mut text = String::new();
        for s in slides {
            if let Some(x) = read(&s) {
                text.push_str(&xml_text(&x, &["a:t"], &["a:p"], &[]));
                text.push('\n');
            }
        }
        return Some(DocText { text, sheet: false });
    }
    // Excel: the shared strings hold the text cells.
    if has("xl/sharedStrings.xml") || names.iter().any(|n| n.starts_with("xl/worksheets/")) {
        let text = read("xl/sharedStrings.xml")
            .map(|x| xml_text(&x, &["t"], &["si"], &[]))
            .unwrap_or_default();
        return Some(DocText { text, sheet: true });
    }
    // OpenDocument (odt, odp, ods)
    if has("content.xml") {
        let sheet = read("mimetype").is_some_and(|m| m.contains("spreadsheet"));
        let text = xml_text(
            &read("content.xml")?,
            &["text:p", "text:h", "text:span", "text:a"],
            &["text:p", "text:h"],
            &["text:tab", "text:s", "text:line-break"],
        );
        return Some(DocText { text, sheet });
    }
    None
}

/// `prefix1.xml`, `prefix2.xml`, … sorted by number.
fn numbered(names: &[String], prefix: &str) -> Vec<String> {
    let mut v: Vec<(u32, String)> = names
        .iter()
        .filter_map(|n| {
            let num = n.strip_prefix(prefix)?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n.clone()))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, n)| n).collect()
}

/// Text inside the given elements of an XML document. `breaks` end a
/// paragraph (newline); `spaces` are empty elements that stand for a space.
fn xml_text(xml: &str, text_tags: &[&str], breaks: &[&str], spaces: &[&str]) -> String {
    let mut out = String::with_capacity(xml.len() / 4);
    let mut depth = 0usize; // inside a text element
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if depth > 0 {
            decode_entities(&rest[..lt], &mut out);
        }
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if spaces.contains(&name) {
            out.push(' ');
            continue;
        }
        if text_tags.contains(&name) && !self_closing {
            if closing {
                depth = depth.saturating_sub(1);
            } else {
                depth += 1;
            }
        }
        if closing && breaks.contains(&name) && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn decode_entities(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in entries {
            w.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(d.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn word_text_with_paragraphs_and_entities() {
        let doc = zip(&[
            ("[Content_Types].xml", "<Types/>"),
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Q3 pricing</w:t></w:r><w:r><w:t xml:space="preserve"> plan &amp; notes</w:t></w:r></w:p><w:p><w:r><w:tab/><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#,
            ),
        ]);
        let t = extract(Format::OfficeDocument, &doc).unwrap();
        assert_eq!(t.text, "Q3 pricing plan & notes\n Second\n");
        assert!(!t.sheet);
    }

    #[test]
    fn slides_in_order_and_sheets() {
        let pptx = zip(&[
            (
                "ppt/slides/slide10.xml",
                "<p:sld><a:p><a:r><a:t>Ten</a:t></a:r></a:p></p:sld>",
            ),
            (
                "ppt/slides/slide2.xml",
                "<p:sld><a:p><a:r><a:t>Two</a:t></a:r></a:p></p:sld>",
            ),
        ]);
        assert_eq!(
            extract(Format::OfficeDocument, &pptx).unwrap().text,
            "Two\n\nTen\n\n"
        );
        let xlsx = zip(&[
            ("xl/worksheets/sheet1.xml", "<worksheet/>"),
            (
                "xl/sharedStrings.xml",
                "<sst><si><t>Region</t></si><si><r><t>North</t></r><r><t>east</t></r></si></sst>",
            ),
        ]);
        let t = extract(Format::OfficeDocument, &xlsx).unwrap();
        assert_eq!(t.text, "Region\nNortheast\n");
        assert!(t.sheet);
    }

    #[test]
    fn opendocument_and_unreadable() {
        let odt = zip(&[
            ("mimetype", "application/vnd.oasis.opendocument.text"),
            ("content.xml", "<office:document-content><office:body><text:h>Title</text:h><text:p>Hello<text:s/>world</text:p></office:body></office:document-content>"),
        ]);
        assert_eq!(
            extract(Format::OpenDocument, &odt).unwrap().text,
            "Title\nHello world\n"
        );
        // Legacy binary Office and junk: nothing.
        assert!(extract(Format::OfficeDocument, b"\xD0\xCF\x11\xE0junk").is_none());
        assert!(extract(Format::Pdf, b"%PDF-1.7 broken").is_none());
    }
}
