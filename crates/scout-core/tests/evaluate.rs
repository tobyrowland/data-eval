mod common;

use common::*;
use scout_core::evaluate::{evaluate, Kind};
use scout_core::{scan, ScanOptions};
use serde_json::json;

const PROSE: &[&str] = &[
    "We moved the release to Friday because the database migration needs another review from the platform team.",
    "Can someone check why the nightly import failed? The logs show a timeout while talking to the billing service.",
    "I restarted the worker and added a retry with backoff, so the import should finish tonight without manual steps.",
    "Thanks, that fixed it. Let's write a short runbook so the on-call person knows what to do next time.",
    "Agreed. I'll draft it this afternoon and share it in the channel before the planning meeting tomorrow.",
];

fn day(msgs: &[(&str, &str, Option<&str>)], base: i64) -> String {
    json!(msgs
        .iter()
        .enumerate()
        .map(|(i, (user, text, bot))| {
            let mut m = json!({"ts": format!("{}.{:06}", base + i as i64 * 60, i), "user": user, "text": text});
            if bot.is_some() {
                m["bot_id"] = json!("B1");
                m["subtype"] = json!("bot_message");
            }
            m
        })
        .collect::<Vec<_>>())
    .to_string()
}

fn fixture(name: &str) -> scout_core::Inventory {
    let d = tempdir(name);
    // 2021 conversation by people, plus a bot; a 2024 conversation.
    let old = day(
        &PROSE
            .iter()
            .enumerate()
            .map(|(i, t)| (if i % 2 == 0 { "U1" } else { "U2" }, *t, None))
            .chain([("B", "Build #42 passed on main in 3m 12s", Some("bot"))])
            .collect::<Vec<_>>(),
        1_609_459_200,
    );
    let new = day(&[("U1", "The new pricing page goes live next week; marketing will announce it on Tuesday morning.", None), ("U2", "Great, I'll update the help centre articles and the onboarding emails before then.", None)], 1_704_067_200);
    let export = zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "eng"}]).to_string().as_bytes(),
        ),
        ("users.json", b"[]"),
        ("eng/2021-01-01.json", old.as_bytes()),
        ("eng/2024-01-01.json", new.as_bytes()),
    ]);
    write(&d, "export.zip", &export);
    // The same export again: everything in it is a duplicate.
    write(&d, "copy/export.zip", &export);
    // A mailbox: a human email, a newsletter, and the human email repeated.
    let human = "From a Mon\nFrom: Ann <ann@acme.com>\nDate: Tue, 5 Jan 2021 10:00:00 +0000\nSubject: Plan\n\nHi Bo, the vendor contract renews in March. Could you check whether the support terms still cover weekend outages before we sign?\n> earlier quoted text\n\n";
    let news = "From b Tue\nFrom: Newsletter <no-reply@news.example>\nList-Unsubscribe: <mailto:u@news.example>\nDate: Tue, 5 Jan 2021 11:00:00 +0000\n\nTop ten productivity tips for your week, plus our latest webinar recordings and offers.\n\n";
    write(&d, "inbox.mbox", format!("{human}{news}{human}").as_bytes());
    write(&d, "docs/handbook.md", "# Handbook\n\nNew starters spend their first week pairing with a buddy. They set up their laptop, meet each team, and ship a small change to production by Friday so they learn the release process early.\n".as_bytes());
    write(&d, "docs/noise.txt", "0x7f3a 0x7f3b ### ### 1234 5678 {{}} [[ ]] <> <> ### 0x00 0x01 0x02 0x03 0x04 0x05 ### 0x06 0x07 0x08 0x09".as_bytes());
    write(
        &d,
        "code/main.rs",
        b"fn main() {\n    println!(\"hello\");\n}\n",
    );
    write(&d, "data/sales.csv", b"region,amount\nnorth,10\nsouth,20\n");

    scan(
        &[
            d.join("export.zip"),
            d.join("copy/export.zip"),
            d.join("inbox.mbox"),
            d.join("docs"),
            d.join("code"),
            d.join("data"),
        ],
        ScanOptions::default(),
    )
}

#[test]
fn evaluation_separates_usable_text_from_bots_duplicates_and_noise() {
    let inv = fixture("evaluate");
    let ev = evaluate(&inv, None, &mut |_| {});
    let row = |label: &str, kind: Kind| {
        ev.rows
            .iter()
            .find(|r| r.collection == label && r.kind == kind)
            .cloned()
    };

    // Conversations: people's messages are usable; 2021 counts as pre-LLM.
    let conv = row("export.zip", Kind::Conversation).expect("conversation row");
    assert_eq!(conv.documents, 2);
    assert!(conv.usable_tokens > 50, "{conv:?}");
    assert!(conv.pre_llm_tokens > 0 && conv.pre_llm_tokens < conv.tokens);
    // High value needs volume (1,000+ usable tokens); a tiny sample is medium.
    assert_eq!(conv.value_band, 2, "{conv:?}");
    // The bot message is machine text, not usable.
    let bot = row("export.zip", Kind::Machine).expect("machine row");
    assert_eq!(bot.usable_tokens, 0);
    // The copied export is all duplicates.
    let copy = row("export.zip (2)", Kind::Conversation).expect("copy row");
    assert_eq!(copy.usable_tokens, 0);
    assert_eq!(copy.duplicate_tokens, copy.tokens);
    // Email: one usable human email (quotes dropped, repeat is a duplicate);
    // the newsletter is automated.
    let mail = row("inbox.mbox", Kind::Email).expect("email row");
    assert_eq!(mail.documents, 2);
    assert!(mail.duplicate_tokens > 0 && mail.usable_tokens > 0);
    assert!(row("inbox.mbox", Kind::Machine).is_some());
    // Documents: the handbook is usable, the noise file is low quality.
    let docs = row("docs", Kind::Document).expect("documents row");
    assert!(
        docs.usable_tokens > 0 && docs.low_quality_tokens > 0,
        "{docs:?}"
    );
    assert!(row("code", Kind::Code).is_some());
    assert!(row("data", Kind::Structured).is_some());

    let t = &ev.totals;
    assert_eq!(
        t.usable_tokens,
        ev.rows.iter().map(|r| r.usable_tokens).sum::<u64>()
    );
    assert!(t.conversation_tokens > 0 && t.machine_tokens > 0 && t.duplicate_tokens > 0);
    assert!(!ev.incomplete);
    // Measurements only: no text in the result.
    let out = serde_json::to_string(&ev).unwrap();
    for leak in [
        "migration",
        "vendor contract",
        "Handbook",
        "pricing page",
        "ann@acme.com",
    ] {
        assert!(!out.contains(leak), "leaked {leak}");
    }
}

#[test]
fn working_minds_report_is_complete_parseable_and_content_free() {
    let inv = fixture("wm-report");
    let map = scout_core::build_data_map(&inv);
    let ev = evaluate(&inv, None, &mut |_| {});
    let plan = json!({"removed": ["jira"], "kept": []});
    let md = scout_core::evaluate::wm_report::wm_report(&map, &ev, Some("Acme Ltd"), &plan);
    for h in [
        "# Working Minds data assessment: Acme Ltd",
        "## How to read this report",
        "## Summary",
        "## Dimension scores",
        "## Breakdown by platform and kind",
        "## Distributions",
        "## Items in the assessment",
        "## Gaps and owner decisions",
        "## Machine-readable data",
    ] {
        assert!(md.contains(h), "missing {h}");
    }
    // The JSON appendix parses and agrees with the evaluation.
    let start = md.find("```json\n").unwrap() + 8;
    let end = md[start..].find("\n```").unwrap() + start;
    let v: serde_json::Value = serde_json::from_str(&md[start..end]).unwrap();
    assert_eq!(v["totals"]["usable_tokens"], json!(ev.totals.usable_tokens));
    assert_eq!(v["company"]["name"], json!("Acme Ltd"));
    for s in v["scores"].as_array().unwrap() {
        let n = s["score"].as_u64().unwrap();
        assert!(n <= 100, "{s}");
    }
    let uniq = v["scores"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "uniqueness")
        .unwrap();
    assert!(
        uniq["score"].as_u64().unwrap() < 100,
        "duplicates lower uniqueness"
    );
    // Distributions add up.
    let d = &ev.distributions;
    assert_eq!(
        d.documents_by_length.iter().sum::<u64>(),
        ev.totals.documents
    );
    assert!(d.usable_tokens_by_year.values().sum::<u64>() <= ev.totals.usable_tokens);
    assert!(md.contains("Platforms the owner marked as not theirs: jira"));
    // Measurements only.
    for leak in [
        "migration",
        "vendor contract",
        "ann@acme.com",
        "Ann <",
        "dps-test-",
        "pricing page",
    ] {
        assert!(!md.contains(leak), "leaked {leak}");
    }
}
