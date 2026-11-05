//! Bounded readers. Every decompressor output is wrapped in a
//! [`RatioGuard`] and every leaf read goes through a [`MeteredReader`], so
//! actual expanded bytes (never header claims) drive the limits (spec §5).

use crate::model::Reason;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::fmt;
use std::io::{self, Read};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A limit was reached while reading. Carried inside `io::Error`.
#[derive(Debug, Clone)]
pub struct LimitHit(pub Reason);

impl fmt::Display for LimitHit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.explain())
    }
}
impl std::error::Error for LimitHit {}

pub fn limit_error(reason: Reason) -> io::Error {
    io::Error::other(LimitHit(reason))
}

/// Extracts a [`Reason`] from an io::Error produced by a guard, if any.
pub fn limit_reason(e: &io::Error) -> Option<Reason> {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<LimitHit>())
        .map(|l| l.0.clone())
}

/// Stops a decompression stream whose output grows suspiciously relative to
/// its compressed size, or beyond a hard per-stream cap.
pub struct RatioGuard<R> {
    inner: R,
    compressed: u64,
    max_ratio: u64,
    floor: u64,
    hard_cap: u64,
    produced: u64,
    tripped: Option<Reason>,
}

impl<R: Read> RatioGuard<R> {
    pub fn new(inner: R, compressed: u64, max_ratio: u64, floor: u64, hard_cap: u64) -> Self {
        RatioGuard {
            inner,
            compressed: compressed.max(1),
            max_ratio,
            floor,
            hard_cap,
            produced: 0,
            tripped: None,
        }
    }
}

impl<R: Read> Read for RatioGuard<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Once tripped, the stream stays dead so callers cannot resume a bomb.
        if let Some(r) = &self.tripped {
            return Err(limit_error(r.clone()));
        }
        let n = self.inner.read(buf)?;
        self.produced += n as u64;
        let trip = if self.produced > self.hard_cap {
            Some(Reason::ExpansionLimit)
        } else if self.produced > self.floor && self.produced / self.compressed > self.max_ratio {
            Some(Reason::CompressionRatio)
        } else {
            None
        };
        if let Some(r) = trip {
            self.tripped = Some(r.clone());
            return Err(limit_error(r));
        }
        Ok(n)
    }
}

/// Shared expanded-byte budget for the whole assessment.
#[derive(Clone)]
pub struct Budget {
    used: Rc<Cell<u64>>,
    max: u64,
    cancel: Option<Arc<AtomicBool>>,
}

impl Budget {
    pub fn new(max: u64) -> Self {
        Budget {
            used: Rc::new(Cell::new(0)),
            max,
            cancel: None,
        }
    }
    pub fn with_cancel(mut self, cancel: Option<Arc<AtomicBool>>) -> Self {
        self.cancel = cancel;
        self
    }
    pub fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
    pub fn used(&self) -> u64 {
        self.used.get()
    }
    pub fn exhausted(&self) -> bool {
        self.used.get() >= self.max
    }
}

/// Counts and hashes every byte read, enforcing the global budget and a
/// per-object cap.
pub struct MeteredReader<'a> {
    inner: &'a mut dyn Read,
    budget: Budget,
    object_cap: u64,
    read: u64,
    hasher: Sha256,
}

impl<'a> MeteredReader<'a> {
    pub fn new(inner: &'a mut dyn Read, budget: Budget, object_cap: u64) -> Self {
        MeteredReader {
            inner,
            budget,
            object_cap,
            read: 0,
            hasher: Sha256::new(),
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.read
    }

    /// Reads to the end so the hash covers the full object.
    pub fn drain(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 64 * 1024];
        loop {
            if self.read(&mut buf)? == 0 {
                return Ok(());
            }
        }
    }

    pub fn finish_hash(self) -> String {
        let digest = self.hasher.finalize();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Read for MeteredReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Checked per read so Stop is prompt even inside a very large file.
        if self.budget.cancelled() {
            return Err(limit_error(Reason::StoppedByUser));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        self.budget.used.set(self.budget.used.get() + n as u64);
        if self.read > self.object_cap {
            return Err(limit_error(Reason::ObjectTooLarge));
        }
        if self.budget.used.get() > self.budget.max {
            return Err(limit_error(Reason::ExpansionLimit));
        }
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// Reads up to `n` bytes, tolerating short reads.
pub fn read_head(r: &mut dyn Read, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut filled = 0;
    while filled < n {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(k) => filled += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_guard_trips() {
        let data = vec![0u8; 10_000];
        let mut g = RatioGuard::new(&data[..], 10, 100, 100, u64::MAX);
        let mut out = Vec::new();
        let err = g.read_to_end(&mut out).unwrap_err();
        assert_eq!(limit_reason(&err), Some(Reason::CompressionRatio));
    }

    #[test]
    fn budget_trips() {
        let data = vec![1u8; 5000];
        let mut src: &[u8] = &data;
        let budget = Budget::new(1000);
        let mut m = MeteredReader::new(&mut src, budget, u64::MAX);
        let err = m.drain().unwrap_err();
        assert_eq!(limit_reason(&err), Some(Reason::ExpansionLimit));
    }
}
