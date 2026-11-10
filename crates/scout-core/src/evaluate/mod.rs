//! Training-value evaluation: a second, slower pass that reads every
//! readable document in full and measures how much of it is useful as
//! LLM training data. Everything runs locally; only measurements leave
//! this module (never text).
//!
//! A document is one chat conversation (a Slack channel-day, a Teams chat,
//! a Discord channel, chunked at 200 messages), one email, one text file or
//! one code file. For each: tokens, kind, written by people or machines,
//! year, a quality score and a near-duplicate signature.

pub mod mail;
pub mod minhash;
pub mod quality;

use crate::model::{Format, Status};
use crate::scan::{Inventory, ScanOptions};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const METHOD: &str = "Every readable document is read in full on this computer. Tokens are estimated from words (about 1.33 tokens per word; characters for Chinese, Japanese and similar scripts). Usable tokens are written by people, score at least 0.5 for text quality (word shape, symbols, repetition, function words) and are not exact or near duplicates (MinHash, 70% similarity). Text written before 2023 is counted separately because it is unlikely to be AI-generated. PDF, Office, image, audio and video files are counted but not evaluated yet.";

/// What a document is, for training purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Conversation,
    Email,
    #[default]
    Document,
    Code,
    Structured,
    Machine,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Conversation => "Conversations",
            Kind::Email => "Email",
            Kind::Document => "Documents",
            Kind::Code => "Code",
            Kind::Structured => "Structured data",
            Kind::Machine => "Bot and automated messages",
        }
    }
    /// How much a usable token of this kind is typically worth for training,
    /// relative to human conversation.
    pub fn weight(self) -> f32 {
        match self {
            Kind::Conversation => 1.0,
            Kind::Email => 0.8,
            Kind::Code => 0.8,
            Kind::Document => 0.7,
            Kind::Structured => 0.2,
            Kind::Machine => 0.05,
        }
    }
}

#[derive(Debug, Clone)]
struct Doc {
    col: u32,
    kind: Kind,
    tokens: u64,
    quality: f32,
    human: bool,
    year: Option<i32>,
    turns: u32,
    duplicate: bool,
}

/// Collects documents during the evaluation pass.
#[derive(Default)]
pub struct Evaluator {
    docs: Vec<Doc>,
    sigs: Vec<(usize, minhash::Signature)>,
    exact: HashSet<u64>,
    pub bytes_read: u64,
}

impl std::fmt::Debug for Evaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Evaluator({} documents)", self.docs.len())
    }
}

/// Tokens in a piece of text (words for spaced scripts, characters for
/// unspaced ones, via the language module's word count).
fn tokens_of(text: &str) -> u64 {
    (crate::analyze::language::word_count(text) as f64 * 1.33).round() as u64
}

fn year_of_unix(t: i64) -> i32 {
    crate::analyze::slack::civil_from_days(t.div_euclid(86_400)).0 as i32
}

impl Evaluator {
    pub fn documents(&self) -> usize {
        self.docs.len()
    }

    /// Adds one document's text.
    pub fn submit(
        &mut self,
        col: u32,
        kind: Kind,
        text: &str,
        human: bool,
        year: Option<i32>,
        turns: u32,
    ) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let tokens = if kind == Kind::Code {
            (text.chars().count() as f64 / 3.5).round() as u64
        } else {
            tokens_of(text)
        };
        if tokens == 0 {
            return;
        }
        let quality = match kind {
            Kind::Code => 1.0,
            Kind::Structured => 0.6,
            _ => quality::score(text).score,
        };
        // Exact duplicates by normalised text.
        let mut hsh = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(
            &text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            &mut hsh,
        );
        let duplicate = !self.exact.insert(std::hash::Hasher::finish(&hsh));
        let idx = self.docs.len();
        if !duplicate && !matches!(kind, Kind::Structured | Kind::Machine) {
            if let Some(sig) = minhash::signature(text) {
                if self.sigs.len() < 3_000_000 {
                    self.sigs.push((idx, sig));
                }
            }
        }
        self.docs.push(Doc {
            col,
            kind,
            tokens,
            quality,
            human,
            year,
            turns,
            duplicate,
        });
    }

    /// Structured data without prose (CSV, JSON records): counted by size.
    pub fn add_structured(&mut self, col: u32, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.docs.push(Doc {
            col,
            kind: Kind::Structured,
            tokens: bytes / 4,
            quality: 0.6,
            human: true,
            year: None,
            turns: 0,
            duplicate: false,
        });
    }

    /// Chat exports and other JSON documents.
    pub fn add_json(&mut self, col: u32, path: &str, doc: &Value) {
        let file = path.rsplit('/').next().unwrap_or("");
        // Slack day file: [{ts, user, text, subtype, bot_id}, ...]
        if is_day_file(file) {
            if let Some(msgs) = doc.as_array() {
                self.chat(
                    col,
                    msgs.iter().filter_map(|m| {
                        let ts = m.get("ts").and_then(Value::as_str)?;
                        let t = ts.split('.').next()?.parse::<i64>().ok();
                        let sub = m.get("subtype").and_then(Value::as_str).unwrap_or("");
                        if sub.starts_with("channel_")
                            || sub.starts_with("group_")
                            || sub == "tombstone"
                        {
                            return None;
                        }
                        let bot = sub == "bot_message" || m.get("bot_id").is_some();
                        let text = m.get("text").and_then(Value::as_str).unwrap_or("");
                        let text = crate::analyze::language::strip_chat_code(text);
                        let who = m.get("user").and_then(Value::as_str).map(String::from);
                        Some(ChatMsg {
                            text,
                            human: !bot,
                            who,
                            time: t,
                        })
                    }),
                );
                return;
            }
        }
        // Microsoft Graph (Teams): {"value": [chatMessage...]}, by conversation.
        if let Some(items) = doc.get("value").and_then(Value::as_array) {
            let is_chat = items
                .iter()
                .any(|m| m.get("messageType").is_some() && m.get("createdDateTime").is_some());
            if is_chat {
                let mut convs: BTreeMap<String, Vec<ChatMsg>> = BTreeMap::new();
                for m in items {
                    let conv = m
                        .get("channelIdentity")
                        .and_then(|c| c.get("channelId"))
                        .or_else(|| m.get("chatId"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let user = m
                        .get("from")
                        .and_then(|f| f.get("user"))
                        .filter(|u| !u.is_null());
                    let human = user.is_some()
                        && m.get("messageType").and_then(Value::as_str) == Some("message");
                    let body = m
                        .get("body")
                        .and_then(|b| b.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let time = m
                        .get("createdDateTime")
                        .and_then(Value::as_str)
                        .and_then(crate::analyze::teams::parse_time);
                    convs.entry(conv).or_default().push(ChatMsg {
                        text: crate::analyze::language::strip_markup(body),
                        human,
                        who: user
                            .and_then(|u| u.get("id"))
                            .and_then(Value::as_str)
                            .map(String::from),
                        time,
                    });
                }
                for (_, mut msgs) in convs {
                    msgs.sort_by_key(|m| m.time);
                    self.chat(col, msgs.into_iter());
                }
                return;
            }
            self.structured_json(col, doc);
            return;
        }
        // Discord package (messages.json / .csv rows) or DiscordChatExporter.
        let discord_rows = if file.eq_ignore_ascii_case("messages.json")
            || file.eq_ignore_ascii_case("messages.csv")
        {
            doc.as_array()
        } else {
            doc.get("messages")
                .and_then(Value::as_array)
                .filter(|_| doc.get("channel").is_some())
        };
        if let Some(rows) = discord_rows.filter(|r| {
            r.iter()
                .any(|m| m.get("Contents").is_some() || m.get("content").is_some())
        }) {
            let mut msgs: Vec<ChatMsg> = rows
                .iter()
                .map(|m| {
                    let text = m
                        .get("Contents")
                        .or_else(|| m.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let time = m
                        .get("Timestamp")
                        .or_else(|| m.get("timestamp"))
                        .and_then(Value::as_str)
                        .and_then(crate::analyze::teams::parse_time);
                    let who = m
                        .get("author")
                        .and_then(|a| a.get("id"))
                        .and_then(Value::as_str)
                        .map(String::from);
                    ChatMsg {
                        text: crate::analyze::language::strip_chat_code(text),
                        human: true,
                        who,
                        time,
                    }
                })
                .collect();
            msgs.sort_by_key(|m| m.time);
            self.chat(col, msgs.into_iter());
            return;
        }
        self.structured_json(col, doc);
    }

    /// Prose inside other JSON (long string values) counts as structured.
    fn structured_json(&mut self, col: u32, doc: &Value) {
        let mut words = 0u64;
        walk_strings(doc, &mut |s| {
            words += crate::analyze::language::word_count(s)
        });
        if words > 0 {
            self.docs.push(Doc {
                col,
                kind: Kind::Structured,
                tokens: (words as f64 * 1.33) as u64,
                quality: 0.6,
                human: true,
                year: None,
                turns: 0,
                duplicate: false,
            });
        }
    }

    /// A conversation: people's messages form one document per 200 messages;
    /// bot and system messages go to a separate machine document.
    fn chat(&mut self, col: u32, msgs: impl Iterator<Item = ChatMsg>) {
        let mut human = String::new();
        let mut machine = String::new();
        let mut turns = 0u32;
        let mut year = None;
        let flush = |ev: &mut Evaluator, human: &mut String, turns: &mut u32, year: Option<i32>| {
            if !human.is_empty() {
                ev.submit(col, Kind::Conversation, human, true, year, *turns);
            }
            human.clear();
            *turns = 0;
        };
        for m in msgs {
            if year.is_none() {
                year = m.time.map(year_of_unix);
            }
            if m.text.trim().is_empty() {
                continue;
            }
            if m.human {
                human.push_str(m.text.trim());
                human.push('\n');
                turns += 1;
                if turns >= 200 {
                    flush(self, &mut human, &mut turns, year);
                    year = None;
                }
            } else {
                machine.push_str(m.text.trim());
                machine.push('\n');
            }
        }
        flush(self, &mut human, &mut turns, year);
        if !machine.is_empty() {
            self.submit(col, Kind::Machine, &machine, false, year, 0);
        }
    }

    /// Finishes: removes near duplicates and totals everything.
    pub fn finish(
        mut self,
        inv: &Inventory,
        labels: &[String],
        incomplete: bool,
        seconds: u64,
    ) -> Evaluation {
        for d in minhash::near_duplicates(&self.sigs, 0.7) {
            self.docs[d].duplicate = true;
        }
        let mut rows: BTreeMap<(u32, Kind), EvalRow> = BTreeMap::new();
        for d in &self.docs {
            let r = rows.entry((d.col, d.kind)).or_insert_with(|| EvalRow {
                collection: labels.get(d.col as usize).cloned().unwrap_or_default(),
                kind: d.kind,
                kind_label: d.kind.label(),
                ..Default::default()
            });
            r.documents += 1;
            r.tokens += d.tokens;
            r.turns += d.turns as u64;
            r.quality_sum += d.quality as f64 * d.tokens as f64;
            if d.human {
                r.human_tokens += d.tokens;
            }
            if d.year.is_some_and(|y| y < 2023) {
                r.pre_llm_tokens += d.tokens;
            }
            if let Some(y) = d.year {
                r.first_year = Some(r.first_year.map_or(y, |f: i32| f.min(y)));
                r.last_year = Some(r.last_year.map_or(y, |l: i32| l.max(y)));
            }
            if d.duplicate {
                r.duplicate_tokens += d.tokens;
            } else if d.quality < 0.5 {
                r.low_quality_tokens += d.tokens;
            } else if d.human && d.kind != Kind::Machine {
                r.usable_tokens += d.tokens;
            }
        }
        let mut rows: Vec<EvalRow> = rows.into_values().collect();
        for r in &mut rows {
            r.average_quality = if r.tokens > 0 {
                (r.quality_sum / r.tokens as f64 * 100.0).round() / 100.0
            } else {
                0.0
            };
            let human_share = if r.tokens > 0 {
                r.human_tokens as f32 / r.tokens as f32
            } else {
                0.0
            };
            let usable_share = if r.tokens > 0 {
                r.usable_tokens as f32 / r.tokens as f32
            } else {
                0.0
            };
            let v = r.kind.weight()
                * r.average_quality as f32
                * human_share.max(0.05)
                * (0.5 + 0.5 * usable_share);
            r.value_band = if v >= 0.45 && r.usable_tokens >= 1_000 {
                3
            } else if v >= 0.2 && r.usable_tokens > 0 {
                2
            } else {
                1
            };
            r.reason = reason(r);
        }
        rows.sort_by(|a, b| {
            b.usable_tokens
                .cmp(&a.usable_tokens)
                .then(a.collection.cmp(&b.collection))
        });
        let sum = |f: fn(&EvalRow) -> u64| rows.iter().map(f).sum::<u64>();
        let tokens = sum(|r| r.tokens);
        let usable = sum(|r| r.usable_tokens);
        let weighted: f64 = rows
            .iter()
            .map(|r| r.usable_tokens as f64 * r.kind.weight() as f64)
            .sum();
        let totals = EvalTotals {
            documents: sum(|r| r.documents),
            tokens,
            usable_tokens: usable,
            conversation_tokens: rows
                .iter()
                .filter(|r| r.kind == Kind::Conversation)
                .map(|r| r.usable_tokens)
                .sum(),
            human_tokens: sum(|r| r.human_tokens),
            pre_llm_tokens: sum(|r| r.pre_llm_tokens),
            duplicate_tokens: sum(|r| r.duplicate_tokens),
            low_quality_tokens: sum(|r| r.low_quality_tokens),
            machine_tokens: rows
                .iter()
                .filter(|r| r.kind == Kind::Machine)
                .map(|r| r.tokens)
                .sum(),
        };
        let mean_weight = if usable > 0 {
            weighted / usable as f64
        } else {
            0.0
        };
        let (rating, rating_reason) = if usable >= 10_000_000 && mean_weight >= 0.6 {
            ("high", "A large amount of human-written, good-quality text, much of it conversation or email.")
        } else if usable >= 1_000_000 {
            ("medium", "A useful amount of human-written text; more conversation, history or volume would raise it.")
        } else if usable > 0 {
            (
                "low",
                "Only a small amount of usable human-written text so far.",
            )
        } else {
            ("low", "No usable human-written text was found.")
        };
        let mut not = NotEvaluated::default();
        for it in &inv.items {
            if it.status == Status::Excluded {
                continue;
            }
            match it.format {
                Format::Pdf | Format::OfficeDocument | Format::OpenDocument => {
                    not.pdf_and_office_files += 1
                }
                Format::Image | Format::Audio | Format::Video => not.media_files += 1,
                _ => {}
            }
        }
        Evaluation {
            method: METHOD,
            rows,
            totals,
            rating,
            rating_reason,
            not_evaluated: not,
            incomplete,
            seconds,
        }
    }
}

fn reason(r: &EvalRow) -> String {
    let mut parts = Vec::new();
    match r.kind {
        Kind::Conversation if r.documents > 0 => parts.push(format!(
            "multi-turn conversation (~{} messages per conversation)",
            (r.turns / r.documents).max(1)
        )),
        Kind::Email => parts.push("human email".to_string()),
        Kind::Document => parts.push("long-form documents".to_string()),
        Kind::Code => parts.push("source code".to_string()),
        Kind::Structured => parts.push("records and tables, little prose".to_string()),
        Kind::Machine => parts.push("written by bots and automated systems".to_string()),
        _ => {}
    }
    if r.tokens > 0 && r.pre_llm_tokens * 2 > r.tokens {
        parts.push("mostly written before 2023".to_string());
    }
    if r.tokens > 0 && r.duplicate_tokens * 5 > r.tokens {
        parts.push(format!(
            "{}% duplicated elsewhere",
            r.duplicate_tokens * 100 / r.tokens
        ));
    }
    if r.tokens > 0 && r.low_quality_tokens * 4 > r.tokens {
        parts.push("much low-quality text".to_string());
    }
    parts.join(", ")
}

struct ChatMsg {
    text: String,
    human: bool,
    #[allow(dead_code)]
    who: Option<String>,
    time: Option<i64>,
}

fn walk_strings(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::String(s) if s.split_whitespace().count() >= 4 => f(s),
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, f)),
        Value::Object(o) => o.values().for_each(|x| walk_strings(x, f)),
        _ => {}
    }
}

fn is_day_file(name: &str) -> bool {
    let b = name.as_bytes();
    name.len() == 15
        && name.ends_with(".json")
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
}

/// The result shown to the owner (and the basis of the Working Minds
/// assessment). Measurements only.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Evaluation {
    pub method: &'static str,
    pub rows: Vec<EvalRow>,
    pub totals: EvalTotals,
    /// "high", "medium" or "low".
    pub rating: &'static str,
    pub rating_reason: &'static str,
    pub not_evaluated: NotEvaluated,
    pub incomplete: bool,
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EvalRow {
    pub collection: String,
    pub kind: Kind,
    pub kind_label: &'static str,
    pub documents: u64,
    pub tokens: u64,
    /// Written by people, good quality, not duplicated.
    pub usable_tokens: u64,
    pub human_tokens: u64,
    pub pre_llm_tokens: u64,
    pub duplicate_tokens: u64,
    pub low_quality_tokens: u64,
    pub turns: u64,
    pub average_quality: f64,
    pub first_year: Option<i32>,
    pub last_year: Option<i32>,
    /// 3 high, 2 medium, 1 low.
    pub value_band: u8,
    pub reason: String,
    #[serde(skip)]
    quality_sum: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EvalTotals {
    pub documents: u64,
    pub tokens: u64,
    pub usable_tokens: u64,
    pub conversation_tokens: u64,
    pub human_tokens: u64,
    pub pre_llm_tokens: u64,
    pub duplicate_tokens: u64,
    pub low_quality_tokens: u64,
    pub machine_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NotEvaluated {
    pub pdf_and_office_files: u64,
    pub media_files: u64,
}

/// Live counters for the evaluation progress screen.
#[derive(Debug, Clone, Serialize)]
pub struct EvalProgress {
    pub documents: u64,
    pub bytes_read: u64,
    pub files_seen: u64,
    pub collection: String,
}

/// Reads every item of the assessment again, in full, and evaluates it.
pub fn evaluate(
    inv: &Inventory,
    cancel: Option<Arc<AtomicBool>>,
    progress: &mut dyn FnMut(&EvalProgress),
) -> Evaluation {
    evaluate_inputs(
        inv.collections.iter().map(|c| c.source.clone()).collect(),
        inv.collections.iter().map(|c| c.label.clone()).collect(),
        inv.exclude.clone(),
        cancel,
        progress,
    )
}

/// As [`evaluate`], from the items' paths, labels and exclusions.
pub fn evaluate_inputs(
    inputs: Vec<PathBuf>,
    labels: Vec<String>,
    exclude: Vec<String>,
    cancel: Option<Arc<AtomicBool>>,
    progress: &mut dyn FnMut(&EvalProgress),
) -> Evaluation {
    let started = Instant::now();
    let ev = Arc::new(Mutex::new(Evaluator::default()));
    let opts = ScanOptions {
        exclude,
        cancel: cancel.clone(),
        evaluator: Some(ev.clone()),
        ..ScanOptions::default()
    };
    let probe = ev.clone();
    let pass = crate::scan::scan_with_progress(&inputs, opts, &mut |p| {
        let docs = probe.lock().map(|e| e.documents() as u64).unwrap_or(0);
        progress(&EvalProgress {
            documents: docs,
            bytes_read: p.bytes_read,
            files_seen: p.files_seen,
            collection: p.collection.clone(),
        });
    });
    // Take the results out (the progress closure still holds a handle).
    let evaluator = std::mem::take(&mut *ev.lock().unwrap_or_else(|e| e.into_inner()));
    evaluator.finish(&pass, &labels, pass.cancelled, started.elapsed().as_secs())
}

/// Copies what a content parser reads into a document collector.
pub struct Tee<'a> {
    pub inner: &'a mut dyn Read,
    pub sink: &'a mut Sink,
}

impl Read for Tee<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.sink.feed(&buf[..n]);
        Ok(n)
    }
}

/// Collects one file's text while it is read, then hands documents over.
pub struct Sink {
    ev: Arc<Mutex<Evaluator>>,
    col: u32,
    kind: SinkKind,
}

enum SinkKind {
    Mail(Box<mail::MailSplitter>),
    Text {
        buf: Vec<u8>,
        kind: Kind,
        html: bool,
    },
}

const MAX_TEXT: usize = 8 * 1024 * 1024;

impl Sink {
    /// A collector for a readable text file, mailbox or email; None for
    /// formats evaluated another way (or not at all).
    pub fn new(ev: &Arc<Mutex<Evaluator>>, format: Format, col: u32, path: &str) -> Option<Sink> {
        if path.contains("/.git/") || path.contains("/node_modules/") {
            return None;
        }
        let ext = crate::detect::extension(path);
        let kind = match format {
            Format::Mbox | Format::Eml => {
                return Some(Sink {
                    ev: ev.clone(),
                    col,
                    kind: SinkKind::Mail(Box::new(mail::MailSplitter::new(format == Format::Mbox))),
                })
            }
            Format::Xml => Kind::Structured,
            Format::PlainText | Format::Markdown | Format::Html => {
                if crate::analyze::language::is_code_or_data_ext(&ext)
                    && !matches!(
                        ext.as_str(),
                        "txt" | "md" | "markdown" | "html" | "htm" | "rst"
                    )
                {
                    Kind::Code
                } else {
                    Kind::Document
                }
            }
            _ => return None,
        };
        Some(Sink {
            ev: ev.clone(),
            col,
            kind: SinkKind::Text {
                buf: Vec::new(),
                kind,
                html: format == Format::Html,
            },
        })
    }

    fn feed(&mut self, bytes: &[u8]) {
        match &mut self.kind {
            SinkKind::Mail(m) => {
                m.feed(bytes);
                if m.done.len() >= 200 {
                    let done = std::mem::take(&mut m.done);
                    submit_mail(&self.ev, self.col, done);
                }
            }
            SinkKind::Text { buf, .. } => {
                if buf.len() < MAX_TEXT {
                    buf.extend_from_slice(&bytes[..bytes.len().min(MAX_TEXT - buf.len())]);
                }
            }
        }
    }

    pub fn finish(self, bytes_read: u64) {
        let col = self.col;
        match self.kind {
            SinkKind::Mail(mut m) => {
                m.finish();
                submit_mail(&self.ev, col, std::mem::take(&mut m.done));
            }
            SinkKind::Text { buf, kind, html } => {
                let text = String::from_utf8_lossy(&buf);
                let text = if html {
                    crate::analyze::language::strip_markup(&text)
                } else {
                    text.into_owned()
                };
                if let Ok(mut e) = self.ev.lock() {
                    if kind == Kind::Structured {
                        e.add_structured(col, bytes_read);
                    } else {
                        e.submit(col, kind, &text, true, None, 0);
                    }
                }
            }
        }
    }
}

fn submit_mail(ev: &Arc<Mutex<Evaluator>>, col: u32, emails: Vec<mail::Email>) {
    if let Ok(mut e) = ev.lock() {
        for m in emails {
            let kind = if m.automated {
                Kind::Machine
            } else {
                Kind::Email
            };
            e.submit(col, kind, &m.text, !m.automated, m.year, 1);
        }
    }
}
