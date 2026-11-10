//! The detailed report for Working Minds: Markdown written to be read by an
//! LLM (or a person), with a machine-readable JSON appendix. Measurements
//! only: item names and the company name, never message text, addresses,
//! people's names or file paths.

use super::{Evaluation, Kind};
use crate::report::DataMap;
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt::Write as _;

pub const SCORING_VERSION: &str = "wm-scores-2026.11.1";

/// A 0–100 score with what it rests on.
#[derive(Debug, Clone, Serialize)]
pub struct Score {
    pub id: &'static str,
    pub name: &'static str,
    pub score: u32,
    pub weight: f32,
    pub basis: String,
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        (a as f64 * 1000.0 / b as f64).round() / 10.0
    }
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.2}B", n as f64 / 1e9),
    }
}

/// The dimension scores, with documented formulas.
pub fn scores(map: &DataMap, ev: &Evaluation) -> Vec<Score> {
    let t = &ev.totals;
    let tokens = t.tokens.max(1) as f64;
    let quality = if t.tokens > 0 {
        ev.rows
            .iter()
            .filter(|r| !matches!(r.kind, Kind::Code | Kind::Structured))
            .map(|r| r.average_quality * r.tokens as f64)
            .sum::<f64>()
            / ev.rows
                .iter()
                .filter(|r| !matches!(r.kind, Kind::Code | Kind::Structured))
                .map(|r| r.tokens as f64)
                .sum::<f64>()
                .max(1.0)
    } else {
        0.0
    };
    // Completeness: what the data points to but doesn't contain.
    let mut completeness = 100i64;
    let mut gaps = Vec::new();
    if let Some(sf) = &map.slack_files {
        let total = sf.images + sf.documents;
        let found = sf.images_found + sf.documents_found;
        if total > 0 && found * 5 < total * 4 {
            completeness -= 15;
            gaps.push(format!(
                "{} of {} files shared in Slack found",
                found, total
            ));
        }
    }
    if let Some(tf) = &map.teams_files {
        if tf.referenced > 0 && tf.found * 5 < tf.referenced * 4 {
            completeness -= 15;
            gaps.push(format!(
                "{} of {} Teams linked files found",
                tf.found, tf.referenced
            ));
        }
    }
    if map.sources.iter().any(|s| s.snapshot_only) {
        completeness -= 10;
        gaps.push("code added as downloads, without history".to_string());
    }
    let unlinked = map
        .collections
        .iter()
        .filter(|c| c.linkage.status == "unlinked")
        .count();
    if unlinked > 0 {
        completeness -= 10 * unlinked as i64;
        gaps.push(format!("{unlinked} item(s) not connected to the rest"));
    }
    let not_collected = map
        .sources
        .iter()
        .filter(|s| !s.in_selection && s.links >= 50)
        .count();
    if not_collected > 0 {
        completeness -= 5 * not_collected.min(4) as i64;
        gaps.push(format!(
            "{not_collected} platform(s) linked often but not collected"
        ));
    }
    let volume = ((t.usable_tokens as f64 + 1.0).log10() / 8.0 * 100.0).clamp(0.0, 100.0);
    vec![
        Score {
            id: "volume",
            name: "Volume",
            score: volume as u32,
            weight: 0.25,
            basis: format!(
                "{} usable tokens; 100 = 100M (log scale)",
                compact(t.usable_tokens)
            ),
        },
        Score {
            id: "uniqueness",
            name: "Uniqueness",
            score: (100.0 - t.duplicate_tokens as f64 * 100.0 / tokens).clamp(0.0, 100.0) as u32,
            weight: 0.10,
            basis: format!(
                "{}% of tokens are exact or near duplicates",
                pct(t.duplicate_tokens, t.tokens)
            ),
        },
        Score {
            id: "human_share",
            name: "Human-written share",
            score: (t.human_tokens as f64 * 100.0 / tokens).clamp(0.0, 100.0) as u32,
            weight: 0.15,
            basis: format!(
                "{}% written by people (not bots, notifications or bulk mail)",
                pct(t.human_tokens, t.tokens)
            ),
        },
        Score {
            id: "pre_llm",
            name: "Pre-LLM share",
            score: (t.pre_llm_tokens as f64 * 100.0 / tokens).clamp(0.0, 100.0) as u32,
            weight: 0.15,
            basis: format!("{}% dated before 2023", pct(t.pre_llm_tokens, t.tokens)),
        },
        Score {
            id: "quality",
            name: "Text quality",
            score: (quality * 100.0).clamp(0.0, 100.0) as u32,
            weight: 0.15,
            basis: format!(
                "token-weighted mean quality {:.2} (heuristic, prose only)",
                quality
            ),
        },
        Score {
            id: "conversation",
            name: "Conversation and structure",
            score: (t.conversation_tokens as f64 * 100.0 / t.usable_tokens.max(1) as f64)
                .clamp(0.0, 100.0) as u32,
            weight: 0.10,
            basis: format!(
                "{}% of usable tokens are multi-turn conversation",
                pct(t.conversation_tokens, t.usable_tokens)
            ),
        },
        Score {
            id: "completeness",
            name: "Completeness",
            score: completeness.clamp(0, 100) as u32,
            weight: 0.10,
            basis: if gaps.is_empty() {
                "nothing the data points to is missing".to_string()
            } else {
                gaps.join("; ")
            },
        },
    ]
}

pub fn overall(scores: &[Score]) -> u32 {
    let w: f32 = scores.iter().map(|s| s.weight).sum();
    (scores
        .iter()
        .map(|s| s.score as f32 * s.weight)
        .sum::<f32>()
        / w.max(0.01))
    .round() as u32
}

fn platform_of(map: &DataMap, label: &str, kind: Kind) -> String {
    let ps = map
        .collections
        .iter()
        .find(|c| c.label == label)
        .map(|c| c.platforms.clone())
        .unwrap_or_default();
    let pick = if kind == Kind::Email && ps.contains(&"email") {
        Some("email")
    } else {
        ps.iter()
            .copied()
            .find(|p| *p != "email")
            .or(ps.first().copied())
    };
    pick.map_or_else(|| "Other files".to_string(), platform_name)
}

fn platform_name(p: &str) -> String {
    match p {
        "slack" => "Slack".into(),
        "teams" => "Microsoft Teams".into(),
        "discord" => "Discord".into(),
        "email" => "Email".into(),
        "google_drive" => "Google Drive".into(),
        "google_other" => "Google (other products)".into(),
        p => crate::analyze::sources::service(p).map_or(p.to_string(), |s| s.name.to_string()),
    }
}

/// The report. `company` is the owner's own name for the company if they
/// set one; `plan` is the owner's source plan (removals, kept items).
pub fn wm_report(map: &DataMap, ev: &Evaluation, company: Option<&str>, plan: &Value) -> String {
    let t = &ev.totals;
    let sc = scores(map, ev);
    let total_score = overall(&sc);
    let company_name = company
        .map(String::from)
        .or_else(|| map.company.as_ref().map(|c| c.name.clone()))
        .unwrap_or_else(|| "Unknown company".to_string());
    let company_basis = if company.is_some() {
        "set by the owner".to_string()
    } else {
        map.company
            .as_ref()
            .map(|c| {
                format!(
                    "detected ({} confidence) from {}",
                    c.confidence,
                    c.evidence.join(", ")
                )
            })
            .unwrap_or_else(|| "not detected".to_string())
    };
    let first = map
        .collections
        .iter()
        .filter_map(|c| c.date_coverage.as_ref().map(|d| d.first.clone()))
        .min();
    let last = map
        .collections
        .iter()
        .filter_map(|c| c.date_coverage.as_ref().map(|d| d.last.clone()))
        .max();
    let mut platforms: Vec<String> = map
        .collections
        .iter()
        .flat_map(|c| c.platforms.iter().map(|p| platform_name(p)))
        .collect();
    platforms.sort();
    platforms.dedup();

    let mut md = String::new();
    let _ = writeln!(md, "# Working Minds data assessment: {company_name}\n");
    let _ = writeln!(
        md,
        "- Company: **{company_name}** ({company_basis})\n- Assessment: {} · {} {} · rules {} · scoring {}\n- Contents: **measurements only**. No messages, documents, email addresses, people's names or file paths are included.{}\n",
        map.assessment_id, map.app, map.app_version, map.ruleset_version, SCORING_VERSION,
        if ev.incomplete { "\n- **Incomplete:** the evaluation was stopped before the end; figures cover what was read." } else { "" }
    );
    md.push_str("## How to read this report\n\n");
    md.push_str("You are assessing a company's internal data as potential training data for large language models. The owner ran Working Minds on their own computer; it read every readable document and produced the measurements below. Please assess: (1) the likely value of this data for LLM training (pre-training, instruction tuning, dialogue), (2) its strengths and weaknesses, (3) rights, privacy and contamination risks you can infer, and (4) what the owner should add, remove or clarify before a sale.\n\n");
    let _ = writeln!(md, "Method: {}\n", ev.method);
    md.push_str("Definitions: *tokens* are estimated, not counted with a specific tokenizer. *Usable* tokens are written by people, have a quality score of at least 0.5 and are not duplicates. *Pre-2023* text predates widespread AI-generated text. Duplicates are removed across all items (exact and MinHash near-duplicates; Slack messages by channel and timestamp, emails by Message-ID, Teams and Discord messages by ID). PDF and Office files, images, audio and video are counted but not evaluated yet.\n\n");

    md.push_str("## Summary\n\n");
    let _ = writeln!(md, "| Measure | Value |\n|---|---|");
    let rows: Vec<(&str, String)> = vec![
        (
            "Working Minds rating",
            format!("{} ({})", ev.rating, ev.rating_reason),
        ),
        ("Overall score", format!("{total_score} / 100")),
        (
            "Usable tokens",
            format!(
                "{} ({}% of {})",
                compact(t.usable_tokens),
                pct(t.usable_tokens, t.tokens),
                compact(t.tokens)
            ),
        ),
        (
            "Multi-turn conversation (usable)",
            compact(t.conversation_tokens),
        ),
        (
            "Written by people",
            format!("{}%", pct(t.human_tokens, t.tokens)),
        ),
        (
            "Written before 2023",
            format!("{}%", pct(t.pre_llm_tokens, t.tokens)),
        ),
        ("Duplicated (not counted)", compact(t.duplicate_tokens)),
        ("Low quality", compact(t.low_quality_tokens)),
        (
            "Automated (bots, notifications, bulk mail)",
            compact(t.machine_tokens),
        ),
        ("Documents evaluated", t.documents.to_string()),
        (
            "Date span",
            format!(
                "{} to {}",
                first.clone().unwrap_or_else(|| "?".into()),
                last.clone().unwrap_or_else(|| "?".into())
            ),
        ),
        ("Platforms", platforms.join(", ")),
        (
            "Not evaluated yet",
            format!(
                "{} PDF/Office files, {} media files",
                ev.not_evaluated.pdf_and_office_files, ev.not_evaluated.media_files
            ),
        ),
    ];
    for (k, v) in &rows {
        let _ = writeln!(md, "| {k} | {v} |");
    }

    md.push_str(
        "\n## Dimension scores\n\n| Dimension | Score | Weight | Basis |\n|---|---|---|---|\n",
    );
    for s in &sc {
        let _ = writeln!(
            md,
            "| {} | {} | {:.0}% | {} |",
            s.name,
            s.score,
            s.weight * 100.0,
            s.basis
        );
    }
    let _ = writeln!(md, "| **Overall** | **{total_score}** | | weighted mean |");

    md.push_str("\n## Breakdown by platform and kind\n\n| Platform | Item | Kind | Documents | Tokens | Usable | Human | Pre-2023 | Duplicate | Low quality | Avg quality | Years | Value | Notes |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &ev.rows {
        let years = match (r.first_year, r.last_year) {
            (Some(a), Some(b)) if a == b => a.to_string(),
            (Some(a), Some(b)) => format!("{a}–{b}"),
            _ => "—".into(),
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.2} | {} | {} | {} |",
            platform_of(map, &r.collection, r.kind),
            r.collection.replace('|', "/"),
            r.kind_label,
            r.documents,
            r.tokens,
            r.usable_tokens,
            r.human_tokens,
            r.pre_llm_tokens,
            r.duplicate_tokens,
            r.low_quality_tokens,
            r.average_quality,
            years,
            ["", "low", "medium", "high"][r.value_band as usize],
            r.reason
        );
    }

    let d = &ev.distributions;
    md.push_str(
        "\n## Distributions\n\nUsable tokens by year:\n\n| Year | Usable tokens |\n|---|---|\n",
    );
    for (y, n) in &d.usable_tokens_by_year {
        let _ = writeln!(md, "| {y} | {n} |");
    }
    md.push_str("\nTokens by quality score (prose):\n\n| Quality | Tokens |\n|---|---|\n");
    for (i, n) in d.tokens_by_quality_band.iter().enumerate() {
        let _ = writeln!(
            md,
            "| {:.1}–{:.1} | {n} |",
            i as f32 / 10.0,
            (i + 1) as f32 / 10.0
        );
    }
    md.push_str("\nDocuments by length (tokens):\n\n| Length | Documents |\n|---|---|\n");
    for (label, n) in ["<50", "50–199", "200–999", "1k–5k", "5k–20k", "20k+"]
        .iter()
        .zip(d.documents_by_length)
    {
        let _ = writeln!(md, "| {label} | {n} |");
    }
    md.push_str(
        "\nConversations by number of messages:\n\n| Messages | Conversations |\n|---|---|\n",
    );
    for (label, n) in ["1", "2–3", "4–10", "11–50", "51+"]
        .iter()
        .zip(d.conversations_by_turns)
    {
        let _ = writeln!(md, "| {label} | {n} |");
    }
    md.push_str("\nLanguages (share of words examined):\n\n| Language | Share |\n|---|---|\n");
    for l in map.languages.rows.iter().take(12) {
        let _ = writeln!(md, "| {} | {}% |", l.language, l.share_percent);
    }

    md.push_str("\n## Items in the assessment\n\n| Item | Type | Platforms | Files | Size (bytes) | Dates | Linked to the rest | Overlap |\n|---|---|---|---|---|---|---|---|\n");
    for c in &map.collections {
        let overlap = match (&c.obsolete_given, c.overlaps.first()) {
            (Some(o), _) => format!("duplicate of {}", o.replace('|', "/")),
            (None, Some(o)) if o.share_percent >= 1.0 => {
                format!("{}% also in {}", o.share_percent, o.with.replace('|', "/"))
            }
            _ => "unique".into(),
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            c.display_label.replace('|', "/"),
            c.kind,
            c.platforms.join(", "),
            c.logical_files,
            c.expanded_bytes,
            c.date_coverage
                .as_ref()
                .map(|d| format!("{}–{}", d.first, d.last))
                .unwrap_or_else(|| "—".into()),
            c.linkage.status,
            overlap
        );
    }
    md.push_str("\n## Gaps and owner decisions\n\n");
    if let Some(sf) = &map.slack_files {
        let _ = writeln!(
            md,
            "- Slack shared files: {} of {} found ({} images, {} documents referenced).",
            sf.images_found + sf.documents_found,
            sf.images + sf.documents,
            sf.images,
            sf.documents
        );
    }
    if let Some(tf) = &map.teams_files {
        let _ = writeln!(
            md,
            "- Teams linked files: {} of {} found.",
            tf.found, tf.referenced
        );
    }
    let missing: Vec<String> = map
        .sources
        .iter()
        .filter(|s| !s.in_selection && (s.links > 0 || s.app_installed || s.bot_messages > 0))
        .map(|s| {
            format!(
                "{} ({} links, {} bot messages)",
                s.name, s.links, s.bot_messages
            )
        })
        .collect();
    if !missing.is_empty() {
        let _ = writeln!(
            md,
            "- Platforms the data refers to but that were not collected: {}.",
            missing.join("; ")
        );
    }
    let list = |k: &str| -> Vec<String> {
        plan.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let removed = list("removed");
    if !removed.is_empty() {
        let _ = writeln!(
            md,
            "- Platforms the owner marked as not theirs: {}.",
            removed.join(", ")
        );
    }
    let kept = list("kept");
    if !kept.is_empty() {
        let _ = writeln!(
            md,
            "- Items flagged as unconnected that the owner chose to keep: {}.",
            kept.join(", ")
        );
    }
    let _ = writeln!(md, "- Duplicates removed before counting: {} identical files, {} duplicate messages and emails.", map.totals.exact_duplicate_files, map.totals.duplicate_records);

    let appendix = json!({
        "company": { "name": company_name, "basis": company_basis },
        "assessment_id": map.assessment_id,
        "scoring_version": SCORING_VERSION,
        "rating": ev.rating,
        "overall_score": total_score,
        "scores": sc,
        "totals": ev.totals,
        "rows": ev.rows,
        "distributions": ev.distributions,
        "not_evaluated": ev.not_evaluated,
        "incomplete": ev.incomplete,
        "items": map.collections.iter().map(|c| json!({
            "item": c.display_label, "type": c.kind, "platforms": c.platforms, "files": c.logical_files,
            "bytes": c.expanded_bytes, "dates": c.date_coverage.as_ref().map(|d| [d.first.clone(), d.last.clone()]),
            "linkage": c.linkage.status, "duplicate_of": c.obsolete_given,
        })).collect::<Vec<_>>(),
        "languages": map.languages.rows.iter().take(12).map(|l| json!({"language": l.language, "share_percent": l.share_percent})).collect::<Vec<_>>(),
        "slack_files": map.slack_files.as_ref().map(|s| json!({"referenced": s.images + s.documents, "found": s.images_found + s.documents_found})),
        "teams_files": map.teams_files.as_ref().map(|t| json!({"referenced": t.referenced, "found": t.found})),
        "not_collected": missing,
    });
    md.push_str("\n## Machine-readable data\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&appendix).unwrap_or_default());
    md.push_str("\n```\n");
    md
}
