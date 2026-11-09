//! Apparent language mix (spec §6: "Apparent language mix, its method and
//! the proportion that could be assessed").
//!
//! Detection is local and deterministic: `whatlang` compares character
//! trigram statistics against bundled profiles. No model calls, no network.
//! Short or ambiguous text is reported as "too short to tell" rather than
//! guessed, and a channel name is never treated as a language label.

use serde::Serialize;
use std::collections::BTreeMap;

pub const METHOD: &str = "whatlang 0.18 trigram detector, run locally on this computer, on prose only: code and data files (such as .js, .json, .csv, .log) are skipped, and passages that read like code, logs or placeholder text are set aside. Segments of 60+ letters take the detector's top language, 20 to 59 letters need a confident result, and anything shorter is left undetermined";

/// Segments with fewer letters than this are not classified.
const MIN_LETTERS: usize = 20;
/// From this length the detector's top guess is reliable enough to use even
/// when a related language scores close behind (whatlang's confidence is a
/// margin over the runner-up, so it is low for e.g. English vs Dutch).
const SENTENCE_LETTERS: usize = 60;
/// Documents are split into segments of about this many characters.
const SEGMENT_CHARS: usize = 1200;
/// Text sampled per document; longer documents are sampled from the start.
pub const MAX_SAMPLE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default)]
pub struct LanguageTally {
    /// ISO 639-3 code -> (words, segments)
    pub by_code: BTreeMap<&'static str, (u64, u64)>,
    pub undetermined_words: u64,
    pub undetermined_segments: u64,
    /// Text that isn't prose: code, logs, data or placeholder text. Left out
    /// of the language shares rather than being misread as a language.
    pub non_prose_words: u64,
    pub non_prose_segments: u64,
    /// Files skipped because their type is code or data (e.g. .js, .csv).
    pub non_prose_files: u64,
    /// Documents whose text was sampled rather than read in full.
    pub sampled_documents: u64,
}

impl LanguageTally {
    /// Classifies one short unit of text, such as a chat message.
    pub fn add_segment(&mut self, text: &str) {
        let words = text.split_whitespace().count() as u64;
        if words == 0 {
            return;
        }
        let letters = text.chars().filter(|c| c.is_alphabetic()).count();
        if letters >= MIN_LETTERS && (!looks_like_prose(text) || is_placeholder(text)) {
            self.non_prose_words += words;
            self.non_prose_segments += 1;
            return;
        }
        let detected = if letters >= SENTENCE_LETTERS {
            whatlang::detect(text)
        } else if letters >= MIN_LETTERS {
            whatlang::detect(text).filter(|i| i.is_reliable() || i.confidence() >= 0.6)
        } else {
            None
        };
        match detected {
            Some(info) => {
                let e = self.by_code.entry(info.lang().code()).or_default();
                e.0 += words;
                e.1 += 1;
            }
            None => {
                self.undetermined_words += words;
                self.undetermined_segments += 1;
            }
        }
    }

    /// Classifies a document paragraph by paragraph. Paragraphs long enough
    /// to judge get their own verdict, so a French email in an English
    /// mailbox is counted as French; short ones are grouped before judging.
    pub fn add_document(&mut self, text: &str, sampled: bool) {
        if sampled {
            self.sampled_documents += 1;
        }
        let mut buf = String::new();
        for para in text.split("\n\n") {
            let letters = para.chars().filter(|c| c.is_alphabetic()).count();
            if letters >= SENTENCE_LETTERS {
                // Short leftovers (headings, greetings) travel with the next
                // real paragraph; a substantial buffer is judged on its own.
                let mut joined = String::new();
                let buf_letters = buf.chars().filter(|c| c.is_alphabetic()).count();
                if buf_letters >= SENTENCE_LETTERS {
                    self.add_segment(&buf);
                } else {
                    joined.push_str(&buf);
                }
                buf.clear();
                joined.push_str(para);
                // Very long paragraphs are judged in slices.
                let chars: Vec<char> = joined.chars().collect();
                for chunk in chars.chunks(SEGMENT_CHARS * 2) {
                    self.add_segment(&chunk.iter().collect::<String>());
                }
            } else {
                buf.push_str(para);
                buf.push('\n');
                if buf.len() >= SEGMENT_CHARS {
                    self.add_segment(&buf);
                    buf.clear();
                }
            }
        }
        if !buf.trim().is_empty() {
            self.add_segment(&buf);
        }
    }

    pub fn merge(&mut self, other: &LanguageTally) {
        for (code, (w, s)) in &other.by_code {
            let e = self.by_code.entry(code).or_default();
            e.0 += w;
            e.1 += s;
        }
        self.undetermined_words += other.undetermined_words;
        self.undetermined_segments += other.undetermined_segments;
        self.non_prose_words += other.non_prose_words;
        self.non_prose_segments += other.non_prose_segments;
        self.non_prose_files += other.non_prose_files;
        self.sampled_documents += other.sampled_documents;
    }

    pub fn determined_words(&self) -> u64 {
        self.by_code.values().map(|v| v.0).sum()
    }

    /// Rows sorted by words, largest first.
    pub fn rows(&self) -> Vec<LanguageRow> {
        let total = self.determined_words() + self.undetermined_words;
        let mut rows: Vec<LanguageRow> = self
            .by_code
            .iter()
            .map(|(code, (words, segments))| LanguageRow {
                language: name_of(code),
                code,
                words: *words,
                segments: *segments,
                share_percent: pct(*words, total),
            })
            .collect();
        rows.sort_by(|a, b| b.words.cmp(&a.words).then(a.code.cmp(b.code)));
        rows
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageRow {
    pub language: &'static str,
    pub code: &'static str,
    pub words: u64,
    pub segments: u64,
    /// Share of all words examined for language, undetermined included.
    pub share_percent: f64,
}

pub fn pct(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 * 1000.0 / total as f64).round() / 10.0
    }
}

fn name_of(code: &str) -> &'static str {
    whatlang::Lang::from_code(code)
        .map(|l| l.eng_name())
        .unwrap_or("Unknown")
}

/// Whether text reads like human prose rather than code, logs or data.
/// Code tokens look like Latin, Danish or Catalan to a trigram detector,
/// so anything that fails this is set aside instead of being classified.
pub fn looks_like_prose(text: &str) -> bool {
    let mut visible = 0usize;
    let mut letters = 0usize;
    let mut code_marks = 0usize;
    let mut digits = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        visible += 1;
        if c.is_alphabetic() {
            letters += 1;
        } else if c.is_ascii_digit() {
            digits += 1;
        } else if "{}[]()<>;=_/\\|#*`$@&^~+".contains(c) {
            code_marks += 1;
        }
    }
    if visible == 0 {
        return false;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let wordlike = tokens
        .iter()
        .filter(|t| {
            let t = t.trim_matches(|c: char| ".,!?;:'\"()«»„“”‘’–—-".contains(c));
            !t.is_empty()
                && t.chars()
                    .all(|c| c.is_alphabetic() || c == '\'' || c == '’' || c == '-')
        })
        .count();
    letters * 100 >= visible * 70
        && code_marks * 100 <= visible * 4
        && digits * 100 <= visible * 15
        && wordlike * 100 >= tokens.len().max(1) * 60
}

/// "Lorem ipsum" placeholder text, which the detector would call Latin.
pub fn is_placeholder(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("lorem ipsum") || t.contains("dolor sit amet")
}

/// File types that hold code or data rather than prose.
pub fn is_code_or_data_ext(ext: &str) -> bool {
    matches!(
        ext,
        "js" | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "py"
            | "rb"
            | "php"
            | "java"
            | "kt"
            | "kts"
            | "scala"
            | "go"
            | "rs"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "swift"
            | "m"
            | "mm"
            | "dart"
            | "lua"
            | "r"
            | "pl"
            | "sh"
            | "bash"
            | "zsh"
            | "ps1"
            | "bat"
            | "cmd"
            | "sql"
            | "graphql"
            | "proto"
            | "vue"
            | "svelte"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "json"
            | "jsonl"
            | "ndjson"
            | "yml"
            | "yaml"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "env"
            | "properties"
            | "lock"
            | "gradle"
            | "xml"
            | "svg"
            | "plist"
            | "csv"
            | "tsv"
            | "log"
            | "map"
            | "ipynb"
            | "tf"
            | "hcl"
            | "dockerfile"
            | "makefile"
            | "cmake"
            | "gitignore"
            | "gitattributes"
            | "editorconfig"
            | "pem"
            | "crt"
            | "key"
    )
}

/// Removes code blocks, inline code and :emoji: shortcodes from a chat
/// message so only the prose is classified.
pub fn strip_chat_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // ```fenced``` blocks
    while let Some(start) = rest.find("```") {
        out.push_str(&rest[..start]);
        match rest[start + 3..].find("```") {
            Some(end) => rest = &rest[start + 3 + end + 3..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    // `inline code`
    let mut cleaned = String::with_capacity(out.len());
    let mut in_code = false;
    for c in out.chars() {
        if c == '`' {
            in_code = !in_code;
            cleaned.push(' ');
        } else if !in_code {
            cleaned.push(c);
        }
    }
    // :emoji_shortcodes:
    cleaned
        .split_whitespace()
        .filter(|w| !(w.len() > 2 && w.starts_with(':') && w.ends_with(':')))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Removes markup so tags and attributes don't skew detection.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Drops email header lines ("Name: value") so only bodies are classified.
pub fn strip_mail_headers(s: &str) -> String {
    s.lines()
        .filter(|l| {
            let key = l.split(':').next().unwrap_or("");
            !(l.starts_with("From ")
                || (l.contains(':') && !key.is_empty() && !key.contains(' ') && key.len() < 40))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_major_languages() {
        let mut t = LanguageTally::default();
        t.add_segment("The deployment failed because the database migration ran before the new schema was ready.");
        t.add_segment("Die Bereitstellung ist fehlgeschlagen, weil die Datenbankmigration zu früh gestartet wurde.");
        t.add_segment("Le déploiement a échoué parce que la migration de la base de données a démarré trop tôt.");
        t.add_segment(
            "El despliegue falló porque la migración de la base de datos comenzó demasiado pronto.",
        );
        // Clearly English, but whatlang's confidence margin is low for these.
        t.add_segment("Looks like the staging database ran out of disk space during the overnight import job.");
        t.add_segment("Thanks for the quick turnaround on the dashboard changes, the sales team loves the new filters.");
        t.add_segment("ok");
        let rows = t.rows();
        let codes: Vec<&str> = rows.iter().map(|r| r.code).collect();
        for c in ["eng", "deu", "fra", "spa"] {
            assert!(codes.contains(&c), "{codes:?}");
        }
        assert_eq!(t.undetermined_segments, 1, "short text is not guessed");
        assert_eq!(t.by_code["eng"].1, 3, "sentence-length English is counted");
        assert_eq!(
            rows.iter().find(|r| r.code == "eng").unwrap().language,
            "English"
        );
    }

    #[test]
    fn code_logs_data_and_placeholders_are_not_languages() {
        let mut t = LanguageTally::default();
        t.add_segment(
            "function updateUser(id, data) { return db.users.update({ where: { id }, data }); }",
        );
        t.add_segment("const result = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' } });");
        t.add_segment("2024-03-01 12:00:01 INFO server started on port 8080 pid=1234 env=production region=eu-west-1");
        t.add_segment("Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore");
        assert!(t.by_code.is_empty(), "{:?}", t.by_code);
        assert_eq!(t.non_prose_segments, 4);
    }

    #[test]
    fn hungarian_is_recognised() {
        let mut t = LanguageTally::default();
        t.add_segment("Szia, meg tudnád nézni, hogy miért nem működik a fizetési rendszer a tegnapi frissítés óta?");
        t.add_segment("A megrendelő azt kérte, hogy a jövő héten küldjük el a végleges ajánlatot és a szerződést.");
        t.add_segment(&strip_chat_code("Köszönöm, holnap reggel átnézem a dokumentumot :thumbsup: ```let x = 1;``` és visszajelzek a csapatnak."));
        assert_eq!(
            t.by_code.get("hun").map(|v| v.1),
            Some(3),
            "{:?}",
            t.by_code
        );
    }

    #[test]
    fn chat_code_is_stripped() {
        assert_eq!(
            strip_chat_code("fix: ```\nfn main() {}\n``` done :tada: see `foo()` now"),
            "fix: done see now"
        );
    }

    #[test]
    fn documents_are_segmented_and_markup_removed() {
        let mut t = LanguageTally::default();
        let html = "<html><body><p>This quarterly report summarises revenue, churn and the main operational risks for the board.</p></body></html>";
        t.add_document(&strip_markup(html), false);
        assert_eq!(t.rows()[0].code, "eng");
        let mail = "From a@b Mon\nSubject: Rechnung\nDate: x\n\nVielen Dank für Ihre Bestellung, die Rechnung finden Sie im Anhang dieser Nachricht.";
        let mut m = LanguageTally::default();
        m.add_document(&strip_mail_headers(mail), false);
        assert_eq!(m.rows()[0].code, "deu");
        // A mailbox: each message body is judged on its own.
        let mut mbox = String::new();
        for i in 0..9 {
            mbox.push_str(if i == 0 {
                "Bonjour, je n'arrive plus à télécharger mes factures depuis votre portail client.\n\n"
            } else {
                "Hello, I can no longer download my invoices from the customer portal since this morning.\n\n"
            });
        }
        let mut t2 = LanguageTally::default();
        t2.add_document(&mbox, false);
        assert_eq!(t2.by_code["fra"].1, 1);
        assert_eq!(t2.by_code["eng"].1, 8);
    }
}
