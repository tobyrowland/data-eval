//! Text quality heuristics for training data, in the spirit of the
//! FineWeb/Gopher filters: word shape, symbols, repetition and stop words.
//! Returns a score from 0 (noise) to 1 (clean prose). Local only.

use std::collections::HashSet;

/// Common function words in the languages seen most; real prose uses them.
const STOP: &[&str] = &[
    "the", "and", "to", "of", "a", "in", "is", "it", "that", "for", "on", "with", "as", "this",
    "be", "are", "was", "we", "you", "i", "have", "not", "but", "at", "or", "if", "so", "can",
    "will", "do", "from", "they", "an", "by", "there", "what", "our", "my", "your", "all", "just",
    // Hungarian
    "a", "az", "és", "hogy", "nem", "is", "meg", "egy", "de", "van", "ez", "azt", "csak", "már",
    // German
    "der", "die", "das", "und", "ist", "nicht", "ich", "es", "zu", "mit", "ein", "eine", "auf",
    // French / Spanish
    "le", "la", "les", "et", "est", "pas", "que", "un", "une", "el", "los", "las", "y", "es", "en",
];

pub struct Quality {
    pub score: f32,
    pub words: u64,
}

pub fn score(text: &str) -> Quality {
    let words: Vec<&str> = text.split_whitespace().collect();
    let n = words.len();
    if n == 0 {
        return Quality {
            score: 0.0,
            words: 0,
        };
    }
    let chars = text.chars().filter(|c| !c.is_whitespace()).count().max(1);
    let symbols = text
        .chars()
        .filter(|c| {
            !c.is_alphanumeric()
                && !c.is_whitespace()
                && !matches!(
                    c,
                    '.' | ',' | '!' | '?' | '\'' | '"' | '-' | ':' | ';' | '(' | ')'
                )
        })
        .count();
    let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
    let mean_len = words.iter().map(|w| w.chars().count()).sum::<usize>() as f32 / n as f32;
    let stop: HashSet<&str> = STOP.iter().copied().collect();
    let stop_hits = words
        .iter()
        .filter(|w| {
            stop.contains(
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
                    .as_str(),
            )
        })
        .count();
    // Repeated lines and repeated word trigrams (templated or spammy text).
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let uniq_lines: HashSet<&str> = lines.iter().copied().collect();
    let dup_lines = if lines.len() > 3 {
        1.0 - uniq_lines.len() as f32 / lines.len() as f32
    } else {
        0.0
    };
    let tri: Vec<String> = words
        .windows(3)
        .map(|w| w.join(" ").to_lowercase())
        .collect();
    let uniq_tri: HashSet<&String> = tri.iter().collect();
    let dup_tri = if tri.len() > 10 {
        1.0 - uniq_tri.len() as f32 / tri.len() as f32
    } else {
        0.0
    };

    let mut s = 1.0f32;
    // Very short texts carry little.
    if n < 5 {
        s *= 0.3;
    } else if n < 20 {
        s *= 0.75;
    }
    if !(2.5..=12.0).contains(&mean_len) {
        s *= 0.4;
    }
    let sym = symbols as f32 / chars as f32;
    if sym > 0.15 {
        s *= 0.3;
    } else if sym > 0.08 {
        s *= 0.7;
    }
    if digits as f32 / chars as f32 > 0.25 {
        s *= 0.4;
    }
    if n >= 20 && (stop_hits as f32 / n as f32) < 0.04 {
        s *= 0.6; // probably not running text (lists, logs, keywords)
    }
    if dup_lines > 0.3 {
        s *= 0.5;
    }
    if dup_tri > 0.4 {
        s *= 0.5;
    }
    Quality {
        score: s.clamp(0.0, 1.0),
        words: n as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_scores_high_and_noise_low() {
        let prose = score("We moved the release to Friday because the migration needs another review, and the team agreed to test it on staging first.");
        assert!(prose.score > 0.7, "{}", prose.score);
        let noise = score("ERR 0x7f3a 0x7f3b 0x7f3c ### ### ### 1234 5678 9012 {{}} [[ ]] <> <> <> ### 0x00 0x01 0x02 0x03 0x04 0x05 ###");
        assert!(noise.score < 0.3, "{}", noise.score);
        let repeated = score(&"Click here to unsubscribe from this list.\n".repeat(20));
        assert!(repeated.score < 0.6, "{}", repeated.score);
    }
}
