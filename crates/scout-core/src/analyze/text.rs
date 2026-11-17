//! Plain-text family: TXT, Markdown, HTML, XML, EML and MBOX. Statistics
//! only; markup is not rendered and no referenced resource is resolved.

use super::Outcome;
use std::io::{self, BufRead, BufReader, Read};

#[derive(Debug, Default, Clone)]
pub struct TextStats {
    pub bytes: u64,
    pub lines: u64,
    pub words: u64,
    pub invalid_utf8_lines: u64,
    /// Lines starting with "From " at a message boundary (MBOX only).
    pub mbox_messages: u64,
    /// Hashes of the Message-ID headers seen (MBOX and EML), to recognise
    /// the same email in another mailbox. Local only.
    pub message_ids: Vec<u64>,
    /// Addresses in From/To/Cc headers (MBOX and EML), capped. Local only.
    pub addresses: Vec<String>,
    /// Leading text, up to the language sample limit.
    pub sample: String,
    pub sample_truncated: bool,
    /// Emails per month, from their Date headers (or the mbox From line).
    pub months: std::collections::BTreeMap<String, u64>,
}

/// Hash of a Message-ID value, ignoring case and surrounding space and <>.
fn message_id_hash(v: &[u8]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let v = String::from_utf8_lossy(v);
    let v = v
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    Some(h.finish())
}

pub fn stats(r: &mut dyn Read) -> io::Result<TextStats> {
    stats_with(r, false, false)
}

/// As [`stats`]; `mbox`/`eml` also read each email's Message-ID header.
pub fn stats_with(r: &mut dyn Read, mbox: bool, eml: bool) -> io::Result<TextStats> {
    let mut s = TextStats::default();
    let mut reader = BufReader::with_capacity(64 * 1024, r);
    let mut line = Vec::with_capacity(256);
    let mut prev_blank = true;
    // Inside an email's header block, and a Message-ID folded onto the next line.
    let mut in_headers = eml;
    let mut id_pending = false;
    // The current email's month from its From line, and whether a Date
    // header has dated it.
    let mut from_month: Option<(i32, u32)> = None;
    let mut dated = false;
    let mut seen_email = eml;
    let add_month = |months: &mut std::collections::BTreeMap<String, u64>, m: (i32, u32)| {
        *months.entry(super::month_key(m.0, m.1)).or_default() += 1;
    };
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        s.bytes += n as u64;
        s.lines += 1;
        if s.sample.len() + line.len() <= super::language::MAX_SAMPLE_BYTES {
            s.sample.push_str(&String::from_utf8_lossy(&line));
        } else {
            s.sample_truncated = true;
        }
        if std::str::from_utf8(&line).is_err() {
            s.invalid_utf8_lines += 1;
        }
        let blank = line.iter().all(|b| b.is_ascii_whitespace());
        if prev_blank && line.starts_with(b"From ") {
            if mbox {
                if seen_email && !dated {
                    if let Some(m) = from_month {
                        add_month(&mut s.months, m);
                    }
                }
                from_month = super::month_of_date(&String::from_utf8_lossy(&line));
                dated = false;
                seen_email = true;
            }
            s.mbox_messages += 1;
            in_headers = mbox;
            id_pending = false;
        } else if in_headers && !dated && line.len() > 5 && line[..5].eq_ignore_ascii_case(b"date:")
        {
            if let Some(m) = super::month_of_date(&String::from_utf8_lossy(&line[5..])) {
                add_month(&mut s.months, m);
                dated = true;
            }
        } else if in_headers {
            if blank {
                in_headers = false;
                id_pending = false;
            } else if id_pending && line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
                if let Some(h) = message_id_hash(&line) {
                    s.message_ids.push(h);
                }
                id_pending = false;
            } else if s.addresses.len() < 20_000
                && [&b"from:"[..], b"to:", b"cc:", b"reply-to:"]
                    .iter()
                    .any(|h| line.len() > h.len() && line[..h.len()].eq_ignore_ascii_case(h))
            {
                let l = String::from_utf8_lossy(&line);
                for tok in l.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
                    let t =
                        tok.trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\'' | '(' | ')'));
                    if t.contains('@') && t.contains('.') {
                        s.addresses.push(t.to_ascii_lowercase());
                    }
                }
            } else if line.len() > 11 && line[..11].eq_ignore_ascii_case(b"message-id:") {
                match message_id_hash(&line[11..]) {
                    Some(h) => s.message_ids.push(h),
                    None => id_pending = true,
                }
            }
        }
        prev_blank = blank;
        s.words += line
            .split(|b| b.is_ascii_whitespace())
            .filter(|w| !w.is_empty())
            .count() as u64;
    }
    if seen_email && !dated {
        if let Some(m) = from_month {
            add_month(&mut s.months, m);
        }
    }
    Ok(s)
}

pub fn analyze(r: &mut dyn Read, mbox: bool, eml: bool) -> io::Result<Outcome> {
    let mut s = stats_with(r, mbox, eml)?;
    let records = if mbox {
        Some(s.mbox_messages)
    } else if eml {
        Some(1)
    } else {
        None
    };
    let mut o = Outcome::assessed(records);
    o.text_bytes = s.bytes;
    o.words = s.words;
    o.sample = Some((std::mem::take(&mut s.sample), s.sample_truncated));
    o.message_ids = std::mem::take(&mut s.message_ids);
    o.addresses = std::mem::take(&mut s.addresses);
    o.months = std::mem::take(&mut s.months);
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_message_ids() {
        let data = b"From a Mon\nMessage-ID: <One@x>\nSubject: x\n\nMessage-ID: <not-a-header>\n\nFrom b Tue\nMessage-Id:\n <two@x>\n\nbody\n\nFrom c Wed\nSubject: none\n\nbody\n";
        let mut r = &data[..];
        let s = stats_with(&mut r, true, false).unwrap();
        assert_eq!(s.mbox_messages, 3);
        assert_eq!(s.message_ids.len(), 2);
        assert_eq!(s.message_ids[0], message_id_hash(b" <one@X> ").unwrap());
    }

    #[test]
    fn counts_mbox_messages() {
        let data = b"From a Mon\nSubject: x\n\nbody From here\n\nFrom b Tue\nSubject: y\n\nbody\n";
        let mut r: &[u8] = data;
        let s = stats(&mut r).unwrap();
        assert_eq!(s.mbox_messages, 2);
        assert!(s.words > 5);
    }
}
