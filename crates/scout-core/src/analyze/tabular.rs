//! Streaming structural profiling for CSV, TSV, JSON and JSON Lines.

use super::Outcome;
use crate::guard::read_head;
use crate::model::{Reason, Status};
use std::io::{self, BufRead, BufReader, Read};

/// Counts delimited records with RFC 4180 quote handling, so embedded
/// newlines inside quoted fields do not inflate the count.
pub fn delimited(r: &mut dyn Read, delim: u8) -> io::Result<Outcome> {
    let mut reader = BufReader::with_capacity(64 * 1024, r);
    let mut in_quotes = false;
    let mut rows: u64 = 0;
    let mut row_has_content = false;
    let mut fields_in_row: u64 = 1;
    let mut header_fields: Option<u64> = None;
    let mut ragged_rows: u64 = 0;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let len = buf.len();
        for &b in buf {
            if b == b'"' {
                in_quotes = !in_quotes;
                row_has_content = true;
            } else if !in_quotes && b == delim {
                fields_in_row += 1;
                row_has_content = true;
            } else if !in_quotes && b == b'\n' {
                if row_has_content {
                    rows += 1;
                    match header_fields {
                        None => header_fields = Some(fields_in_row),
                        Some(h) if h != fields_in_row => ragged_rows += 1,
                        _ => {}
                    }
                }
                row_has_content = false;
                fields_in_row = 1;
            } else if b != b'\r' {
                row_has_content = true;
            }
        }
        reader.consume(len);
    }
    if row_has_content {
        rows += 1;
        if let Some(h) = header_fields {
            if h != fields_in_row {
                ragged_rows += 1;
            }
        }
    }
    let data_rows = rows.saturating_sub(1);
    let mut o = Outcome::assessed(Some(data_rows));
    if in_quotes || (data_rows > 0 && ragged_rows * 10 > data_rows) {
        o.status = Status::PartlyAssessed;
        o.reason = Some(Reason::CorruptStructure);
    }
    Ok(o)
}

pub fn jsonl(r: &mut dyn Read) -> io::Result<Outcome> {
    let reader = BufReader::with_capacity(64 * 1024, r);
    let mut ok: u64 = 0;
    let mut bad: u64 = 0;
    for line in reader.split(b'\n') {
        let line = line?;
        let t = line.trim_ascii();
        if t.is_empty() {
            continue;
        }
        if serde_json::from_slice::<serde::de::IgnoredAny>(t).is_ok() {
            ok += 1;
        } else {
            bad += 1;
        }
    }
    let mut o = Outcome::assessed(Some(ok));
    if bad > 0 {
        o.status = Status::PartlyAssessed;
        o.reason = Some(Reason::CorruptStructure);
    }
    Ok(o)
}

/// Parses a whole JSON document up to `max_bytes`. Larger documents are
/// marked partly assessed (structure not profiled) rather than dropped.
pub fn json(r: &mut dyn Read, max_bytes: usize) -> io::Result<Outcome> {
    let buf = read_head(r, max_bytes + 1)?;
    if buf.len() > max_bytes {
        return Ok(Outcome::with(
            Status::PartlyAssessed,
            Reason::ObjectTooLarge,
        ));
    }
    match serde_json::from_slice::<serde_json::Value>(&buf) {
        Ok(v) => {
            let records = match &v {
                serde_json::Value::Array(a) => a.len() as u64,
                _ => 1,
            };
            let mut o = Outcome::assessed(Some(records));
            o.json = Some(v);
            Ok(o)
        }
        Err(_) => Ok(Outcome::with(
            Status::StoppedSafely,
            Reason::CorruptStructure,
        )),
    }
}

/// Reads a small CSV with a header row into JSON objects (one per row,
/// keyed by header), for recognisers that need the rows themselves, such
/// as a Discord package's `messages.csv`. Larger files are only counted.
pub fn csv_rows(r: &mut dyn Read, max_bytes: usize) -> io::Result<Outcome> {
    let buf = read_head(r, max_bytes + 1)?;
    if buf.len() > max_bytes {
        let mut o = delimited(&mut &buf[..max_bytes], b',')?;
        o.status = Status::PartlyAssessed;
        o.reason = Some(Reason::ObjectTooLarge);
        return Ok(o);
    }
    let text = String::from_utf8_lossy(&buf);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    let mut it = rows.into_iter();
    let header = it.next().unwrap_or_default();
    let objs: Vec<serde_json::Value> = it
        .map(|r| {
            serde_json::Value::Object(
                header
                    .iter()
                    .cloned()
                    .zip(r.into_iter().map(serde_json::Value::String))
                    .collect(),
            )
        })
        .collect();
    let mut o = Outcome::assessed(Some(objs.len() as u64));
    o.json = Some(serde_json::Value::Array(objs));
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quoted_newlines() {
        let mut r: &[u8] = b"a,b\n1,\"x\ny\"\n2,z\n";
        let o = delimited(&mut r, b',').unwrap();
        assert_eq!(o.records, Some(2));
        assert_eq!(o.status, Status::Assessed);
    }

    #[test]
    fn jsonl_counts_bad_lines() {
        let mut r: &[u8] = b"{\"a\":1}\n{broken\n{\"a\":2}\n";
        let o = jsonl(&mut r).unwrap();
        assert_eq!(o.records, Some(2));
        assert_eq!(o.status, Status::PartlyAssessed);
    }
}
