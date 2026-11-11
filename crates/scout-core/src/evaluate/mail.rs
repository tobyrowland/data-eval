//! Splits a mailbox stream into emails and extracts the readable text of
//! each: text/plain (or HTML) parts only, attachments skipped, base64 and
//! quoted-printable decoded, quoted replies and signatures removed.

/// One email's readable text and what its headers say about it.
#[derive(Debug, Default, Clone)]
pub struct Email {
    pub text: String,
    /// Bulk or automated mail (newsletters, notifications).
    pub automated: bool,
    pub year: Option<i32>,
    /// The thread's first message ID (References, else In-Reply-To, else
    /// its own Message-ID); empty if none.
    pub thread: String,
    pub subject: String,
}

#[derive(Default)]
struct Part {
    content_type: String,
    encoding: String,
    attachment: bool,
    body: Vec<u8>,
}

/// Collects one email at a time from a stream of lines.
#[derive(Default)]
pub struct MailSplitter {
    mbox: bool,
    line: Vec<u8>,
    prev_blank: bool,
    in_headers: bool,
    started: bool,
    header_block: Vec<String>,
    boundaries: Vec<String>,
    // top-level message
    automated: bool,
    year: Option<i32>,
    message_id: String,
    in_reply_to: String,
    references: String,
    subject: String,
    top_type: String,
    top_encoding: String,
    // current MIME part
    part: Option<Part>,
    part_in_headers: bool,
    parts: Vec<Part>,
    body: Vec<u8>,
    pub done: Vec<Email>,
}

const MAX_PART: usize = 512 * 1024;

fn header_value<'a>(h: &'a str, name: &str) -> Option<&'a str> {
    let (k, v) = h.split_once(':')?;
    k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
}

fn param(v: &str, name: &str) -> Option<String> {
    v.split(';').skip(1).find_map(|p| {
        let (k, val) = p.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| val.trim().trim_matches('"').to_string())
    })
}

pub fn year_of(date: &str) -> Option<i32> {
    date.split(|c: char| !c.is_ascii_digit())
        .filter(|t| t.len() == 4)
        .filter_map(|t| t.parse::<i32>().ok())
        .find(|y| (1980..=2100).contains(y))
}

pub fn decode_base64(s: &[u8]) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s {
        if let Some(v) = val(c) {
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
    }
    out
}

pub fn decode_qp(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'=' {
            if i + 1 < s.len() && (s[i + 1] == b'\n' || s[i + 1] == b'\r') {
                i += if s.get(i + 2) == Some(&b'\n') && s[i + 1] == b'\r' {
                    3
                } else {
                    2
                };
                continue;
            }
            if i + 2 < s.len() {
                if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&s[i + 1..i + 3]), 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// The first `<id>` in a header (lower case), or the trimmed value.
fn first_id(v: &str) -> String {
    match (v.find('<'), v.find('>')) {
        (Some(s), Some(e)) if e > s => v[s + 1..e].trim().to_ascii_lowercase(),
        _ => v
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase(),
    }
}

/// Readable text without quoted replies, forwarded history or signatures.
pub fn strip_quotes(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t == "--"
            || t == "-- "
            || t.starts_with("-----Original Message")
            || t.starts_with("________________")
        {
            break;
        }
        if t.starts_with("On ") && t.ends_with("wrote:") {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

impl MailSplitter {
    pub fn new(mbox: bool) -> Self {
        MailSplitter {
            mbox,
            prev_blank: true,
            in_headers: !mbox,
            started: !mbox,
            ..Default::default()
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.line.push(b);
            if b == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.on_line(&line);
            }
        }
    }

    pub fn finish(&mut self) {
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.on_line(&line);
        }
        self.end_email();
    }

    fn on_line(&mut self, line: &[u8]) {
        let blank = line.iter().all(|b| b.is_ascii_whitespace());
        if self.mbox && self.prev_blank && line.starts_with(b"From ") {
            self.end_email();
            self.started = true;
            self.in_headers = true;
            self.prev_blank = false;
            return;
        }
        self.prev_blank = blank;
        if !self.started {
            return;
        }
        if self.in_headers {
            if blank {
                self.in_headers = false;
                self.apply_headers();
            } else if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
                if let Some(last) = self.header_block.last_mut() {
                    last.push(' ');
                    last.push_str(String::from_utf8_lossy(line).trim());
                }
            } else {
                self.header_block
                    .push(String::from_utf8_lossy(line).trim_end().to_string());
            }
            return;
        }
        // Body.
        if !self.boundaries.is_empty() {
            let l = String::from_utf8_lossy(line);
            let t = l.trim_end();
            let hit = self
                .boundaries
                .iter()
                .rev()
                .find(|b| t == format!("--{b}") || t == format!("--{b}--"))
                .cloned();
            if let Some(b) = hit {
                let closing = t.ends_with("--") && t == format!("--{b}--");
                self.close_part();
                if closing {
                    while let Some(x) = self.boundaries.pop() {
                        if x == b {
                            break;
                        }
                    }
                } else {
                    self.part = Some(Part::default());
                    self.part_in_headers = true;
                    self.header_block.clear();
                }
                return;
            }
            if let Some(p) = self.part.as_mut() {
                if self.part_in_headers {
                    if blank {
                        self.part_in_headers = false;
                        let headers = std::mem::take(&mut self.header_block);
                        for h in &headers {
                            if let Some(v) = header_value(h, "content-type") {
                                p.content_type = v.to_ascii_lowercase();
                                if p.content_type.starts_with("multipart/") {
                                    if let Some(b) = param(v, "boundary") {
                                        self.boundaries.push(b);
                                    }
                                }
                            } else if let Some(v) = header_value(h, "content-transfer-encoding") {
                                p.encoding = v.to_ascii_lowercase();
                            } else if let Some(v) = header_value(h, "content-disposition") {
                                p.attachment = v.to_ascii_lowercase().starts_with("attachment");
                            }
                        }
                    } else if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
                        if let Some(last) = self.header_block.last_mut() {
                            last.push(' ');
                            last.push_str(String::from_utf8_lossy(line).trim());
                        }
                    } else {
                        self.header_block
                            .push(String::from_utf8_lossy(line).trim_end().to_string());
                    }
                } else if p.body.len() < MAX_PART
                    && !p.attachment
                    && (p.content_type.is_empty() || p.content_type.starts_with("text/"))
                {
                    p.body.extend_from_slice(line);
                }
            }
        } else if self.body.len() < MAX_PART {
            self.body.extend_from_slice(line);
        }
    }

    fn apply_headers(&mut self) {
        let headers = std::mem::take(&mut self.header_block);
        for h in &headers {
            if let Some(v) = header_value(h, "date") {
                self.year = year_of(v);
            } else if let Some(v) = header_value(h, "message-id") {
                self.message_id = first_id(v);
            } else if let Some(v) = header_value(h, "in-reply-to") {
                self.in_reply_to = first_id(v);
            } else if let Some(v) = header_value(h, "references") {
                self.references = first_id(v);
            } else if let Some(v) = header_value(h, "subject") {
                self.subject = v.chars().take(500).collect();
            } else if header_value(h, "list-unsubscribe").is_some()
                || header_value(h, "list-id").is_some()
            {
                self.automated = true;
            } else if let Some(v) = header_value(h, "precedence") {
                let v = v.to_ascii_lowercase();
                if v.contains("bulk") || v.contains("list") || v.contains("junk") {
                    self.automated = true;
                }
            } else if let Some(v) = header_value(h, "auto-submitted") {
                if !v.eq_ignore_ascii_case("no") {
                    self.automated = true;
                }
            } else if let Some(v) = header_value(h, "from") {
                let v = v.to_ascii_lowercase();
                if v.contains("no-reply")
                    || v.contains("noreply")
                    || v.contains("donotreply")
                    || v.contains("notifications@")
                    || v.contains("mailer-daemon")
                {
                    self.automated = true;
                }
            } else if let Some(v) = header_value(h, "content-type") {
                self.top_type = v.to_ascii_lowercase();
                if self.top_type.starts_with("multipart/") {
                    if let Some(b) = param(v, "boundary") {
                        self.boundaries.push(b);
                    }
                }
            } else if let Some(v) = header_value(h, "content-transfer-encoding") {
                self.top_encoding = v.to_ascii_lowercase();
            }
        }
    }

    fn close_part(&mut self) {
        if let Some(p) = self.part.take() {
            if !p.attachment && !p.content_type.starts_with("multipart/") {
                self.parts.push(p);
            }
        }
    }

    fn decode(body: &[u8], encoding: &str) -> String {
        let bytes = if encoding.contains("base64") {
            decode_base64(body)
        } else if encoding.contains("quoted-printable") {
            decode_qp(body)
        } else {
            body.to_vec()
        };
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn end_email(&mut self) {
        if !self.started {
            return;
        }
        self.close_part();
        let text = if !self.parts.is_empty() {
            let plain = self
                .parts
                .iter()
                .find(|p| p.content_type.is_empty() || p.content_type.starts_with("text/plain"));
            match plain {
                Some(p) => Self::decode(&p.body, &p.encoding),
                None => match self
                    .parts
                    .iter()
                    .find(|p| p.content_type.starts_with("text/html"))
                {
                    Some(p) => {
                        crate::analyze::language::strip_markup(&Self::decode(&p.body, &p.encoding))
                    }
                    None => String::new(),
                },
            }
        } else if self.top_type.is_empty() || self.top_type.starts_with("text/") {
            let t = Self::decode(&self.body, &self.top_encoding);
            if self.top_type.starts_with("text/html") {
                crate::analyze::language::strip_markup(&t)
            } else {
                t
            }
        } else {
            String::new()
        };
        let thread = [&self.references, &self.in_reply_to, &self.message_id]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default();
        self.done.push(Email {
            text: strip_quotes(&text),
            automated: self.automated,
            year: self.year,
            thread,
            subject: std::mem::take(&mut self.subject),
        });
        // Reset for the next email.
        let mbox = self.mbox;
        let done = std::mem::take(&mut self.done);
        *self = MailSplitter::new(mbox);
        self.done = done;
        self.started = false;
        self.in_headers = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_mime_and_drops_attachments_and_quotes() {
        let mbox = "From a Mon\nFrom: Ann <ann@acme.com>\nDate: Tue, 3 Jan 2021 10:00:00 +0000\nContent-Type: multipart/mixed; boundary=\"XX\"\n\n--XX\nContent-Type: text/plain\nContent-Transfer-Encoding: quoted-printable\n\nThe launch moved to Fri=\nday.\n> old quoted text\nOn Mon, Bo wrote:\nolder history\n--XX\nContent-Type: application/pdf\nContent-Disposition: attachment\nContent-Transfer-Encoding: base64\n\nJVBERi0xLjQKJcfsj6IKNSAwIG9iago=\n--XX--\n\nFrom b Tue\nFrom: news@list.example\nList-Unsubscribe: <mailto:x>\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: base64\n\nV2Vla2x5IG5ld3M=\n\n";
        let mut s = MailSplitter::new(true);
        s.feed(mbox.as_bytes());
        s.finish();
        assert_eq!(s.done.len(), 2);
        assert_eq!(s.done[0].text.trim(), "The launch moved to Friday.");
        assert_eq!(s.done[0].year, Some(2021));
        assert!(!s.done[0].automated);
        assert_eq!(s.done[1].text.trim(), "Weekly news");
        assert!(s.done[1].automated);
    }
}
