//! Near-duplicate detection with one-permutation MinHash over word
//! 5-shingles, and banded locality-sensitive hashing. In memory only.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub const BINS: usize = 64;
const BANDS: usize = 16;
const ROWS: usize = BINS / BANDS;

pub type Signature = [u32; BINS];

fn h(s: &[&str]) -> u64 {
    let mut x = DefaultHasher::new();
    for w in s {
        w.to_lowercase().hash(&mut x);
    }
    x.finish()
}

/// One hash per shingle; each lands in one of 64 bins and the bin keeps its
/// minimum. Empty bins borrow from the next filled one (densification).
pub fn signature(text: &str) -> Option<Signature> {
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 20 {
        return None;
    }
    let mut sig = [u32::MAX; BINS];
    for sh in words.windows(5) {
        let v = h(sh);
        let bin = (v % BINS as u64) as usize;
        let val = (v >> 32) as u32;
        if val < sig[bin] {
            sig[bin] = val;
        }
    }
    for i in 0..BINS {
        if sig[i] == u32::MAX {
            for k in 1..BINS {
                let j = (i + k) % BINS;
                if sig[j] != u32::MAX {
                    sig[i] = sig[j].wrapping_add(k as u32);
                    break;
                }
            }
        }
    }
    Some(sig)
}

pub fn similarity(a: &Signature, b: &Signature) -> f32 {
    a.iter().zip(b).filter(|(x, y)| x == y).count() as f32 / BINS as f32
}

/// Marks documents that nearly duplicate an earlier one (similarity at least
/// `threshold`). Returns the indices of the duplicates.
pub fn near_duplicates(sigs: &[(usize, Signature)], threshold: f32) -> Vec<usize> {
    let mut buckets: HashMap<(usize, u64), Vec<usize>> = HashMap::new();
    let mut dup = Vec::new();
    for (pos, (doc, sig)) in sigs.iter().enumerate() {
        let mut is_dup = false;
        let mut keys = Vec::with_capacity(BANDS);
        for b in 0..BANDS {
            let mut x = DefaultHasher::new();
            sig[b * ROWS..(b + 1) * ROWS].hash(&mut x);
            keys.push((b, x.finish()));
        }
        'outer: for k in &keys {
            if let Some(cands) = buckets.get(k) {
                for &c in cands {
                    if similarity(&sigs[c].1, sig) >= threshold {
                        is_dup = true;
                        break 'outer;
                    }
                }
            }
        }
        if is_dup {
            dup.push(*doc);
        } else {
            for k in keys {
                let v = buckets.entry(k).or_default();
                if v.len() < 64 {
                    v.push(pos);
                }
            }
        }
    }
    dup
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_near_duplicates() {
        let base = "The quarterly planning meeting covered the roadmap for the mobile app, the hiring plan for the platform team, and the budget for the new office in Budapest next spring.";
        let near = format!("{base} Thanks all.");
        let other = "Our support queue doubled after the outage, so we added a second on-call rotation and wrote a runbook for restoring the billing service from the nightly backup.";
        let sigs: Vec<(usize, Signature)> = [base, near.as_str(), other]
            .iter()
            .enumerate()
            .map(|(i, t)| (i, signature(t).unwrap()))
            .collect();
        assert!(similarity(&sigs[0].1, &sigs[1].1) > 0.7);
        assert_eq!(near_duplicates(&sigs, 0.7), vec![1]);
    }
}
