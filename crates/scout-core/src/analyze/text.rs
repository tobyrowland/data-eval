//! Plain-text family: TXT, Markdown, HTML, XML, EML and MBOX. Statistics
//! only; markup is not rendered and no referenced resource is resolved.

use super::Outcome;
use std::io::{self, BufRead, BufReader, Read};

#[derive(Debug, Default, Clone, Copy)]
pub struct TextStats {
    pub bytes: u64,
    pub lines: u64,
    pub words: u64,
    pub invalid_utf8_lines: u64,
    /// Lines starting with "From " at a message boundary (MBOX only).
    pub mbox_messages: u64,
}

pub fn stats(r: &mut dyn Read) -> io::Result<TextStats> {
    let mut s = TextStats::default();
    let mut reader = BufReader::with_capacity(64 * 1024, r);
    let mut line = Vec::with_capacity(256);
    let mut prev_blank = true;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        s.bytes += n as u64;
        s.lines += 1;
        if std::str::from_utf8(&line).is_err() {
            s.invalid_utf8_lines += 1;
        }
        if prev_blank && line.starts_with(b"From ") {
            s.mbox_messages += 1;
        }
        prev_blank = line.iter().all(|b| b.is_ascii_whitespace());
        s.words += line
            .split(|b| b.is_ascii_whitespace())
            .filter(|w| !w.is_empty())
            .count() as u64;
    }
    Ok(s)
}

pub fn analyze(r: &mut dyn Read, mbox: bool, eml: bool) -> io::Result<Outcome> {
    let s = stats(r)?;
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
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_mbox_messages() {
        let data = b"From a Mon\nSubject: x\n\nbody From here\n\nFrom b Tue\nSubject: y\n\nbody\n";
        let mut r: &[u8] = data;
        let s = stats(&mut r).unwrap();
        assert_eq!(s.mbox_messages, 2);
        assert!(s.words > 5);
    }
}
