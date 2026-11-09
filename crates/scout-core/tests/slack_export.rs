//! Synthetic Slack export, a distributable stand-in for the private CRANQ
//! reference fixture (spec §13). Seeded canaries must never appear in the
//! owner's data map.

mod common;
use common::*;
use scout_core::model::{SourceKind, Status};
use scout_core::{build_data_map, scan, ScanOptions};
use serde_json::json;

const CANARY_TEXT: &str = "CANARY-MESSAGE-7f3a";
const CANARY_TOKEN: &str = "xoxe-CANARY-TOKEN-91b2";
const CANARY_NAME: &str = "Zelda Canaryperson";
const CANARY_CHANNEL: &str = "canary-secret-channel";
const CANARY_EMAIL: &str = "zelda@canary.example";

fn export() -> Vec<u8> {
    let channels = json!([{"id": "C1", "name": CANARY_CHANNEL}, {"id": "C2", "name": "general"}, {"id": "C3", "name": "empty"}]);
    let users = json!([
        {"id": "U1", "real_name": CANARY_NAME, "profile": {"email": CANARY_EMAIL}},
        {"id": "U2", "real_name": "Other"},
        {"id": "B1", "is_bot": true, "real_name": "ci-bot"}
    ]);
    let day1 = json!([
        {"ts": "1609459200.000100", "user": "U1", "text": CANARY_TEXT, "thread_ts": "1609459200.000100", "reply_count": 2},
        {"ts": "1609459300.000100", "user": "U2", "thread_ts": "1609459200.000100",
         "text": "fixed in https://bitbucket.org/acme/core/pull-requests/42",
         "files": [{"id": "F1", "name": "screenshot.png", "mimetype": "image/png", "mode": "hosted",
                    "url_private_download": format!("https://files.slack.com/files-pri/T1-F1/download/s.png?t={CANARY_TOKEN}")}]},
        {"ts": "1609459400.000100", "user": "U1", "thread_ts": "1609459200.000100", "text": "thanks"},
        {"ts": "1609459500.000100", "bot_id": "B1", "subtype": "bot_message", "text": "Build #9 passed"},
        {"ts": "1609459600.000100", "user": "U2", "subtype": "channel_join", "text": "<@U2> has joined"}
    ]);
    let day2 = json!([
        {"ts": "1640995200.000100", "user": "U2", "text": "see https://bitbucket.org/acme/core/pull-requests/42 and https://bitbucket.org/acme/core/pull-requests/43",
         "files": [{"id": "F2", "mimetype": "application/pdf", "mode": "hidden_by_limit"},
                   {"id": "F1", "mimetype": "image/png", "url_private_download": "https://files.slack.com/x"}]}
    ]);
    zip_bytes(&[
        ("channels.json", channels.to_string().as_bytes()),
        ("users.json", users.to_string().as_bytes()),
        (
            "integration_logs.json",
            json!({"logs": [{"service_id": "1"}]})
                .to_string()
                .as_bytes(),
        ),
        (
            &format!("{CANARY_CHANNEL}/2021-01-01.json"),
            day1.to_string().as_bytes(),
        ),
        ("general/2022-01-01.json", day2.to_string().as_bytes()),
    ])
}

#[test]
fn reproduces_structural_inventory() {
    let d = tempdir("slack");
    write(&d, "Acme Slack export.zip", &export());
    let inv = scan(&[d.join("Acme Slack export.zip")], ScanOptions::default());
    assert_eq!(inv.collections[0].source_kind, SourceKind::SlackExport);
    assert!(inv.items.iter().all(|i| i.status == Status::Assessed));
    let s = &inv.slack[0].inventories()[0];
    assert_eq!(s.message_records, 6);
    assert_eq!(s.channels_in_metadata, 3);
    assert_eq!(s.channel_folders_with_messages, 2);
    assert_eq!(s.participant_message_records, 4);
    assert_eq!(s.bot_integration_message_records, 1);
    assert_eq!(s.administrative_records, 1);
    assert_eq!(s.accounts_posting_participant_content, 2);
    assert_eq!(s.user_accounts_in_metadata, 3);
    assert_eq!(s.bot_accounts_in_metadata, 1);
    assert_eq!(s.integration_log_records, 1);
    assert_eq!(s.explicit_threads, 1);
    assert_eq!(s.thread_replies, 2);
    assert_eq!(s.replies_with_missing_parent, 0);
    assert_eq!(s.distinct_file_references, 2);
    assert_eq!(s.referenced_image_files, 1);
    assert_eq!(s.file_records_marked_visible, 1);
    assert_eq!(s.file_download_links_present, 1);
    assert_eq!(s.file_binaries_retrieved, 0);
    assert_eq!(s.distinct_bitbucket_pr_references, 2);
    assert_eq!(s.monthly_activity.len(), 2);
}

#[test]
fn data_map_contains_no_canaries_or_paths() {
    let d = tempdir("slack-canary");
    write(&d, "Acme Slack export.zip", &export());
    let inv = scan(&[d.join("Acme Slack export.zip")], ScanOptions::default());
    let map = build_data_map(&inv);
    let json = serde_json::to_string(&map).unwrap();
    let text = scout_core::report::render_text(&map);
    for out in [&json, &text] {
        for canary in [
            CANARY_TEXT,
            CANARY_TOKEN,
            CANARY_NAME,
            CANARY_CHANNEL,
            CANARY_EMAIL,
            "files.slack.com",
            "bitbucket.org",
            "U1",
            "screenshot.png",
            "2021-01-01.json",
        ] {
            assert!(!out.contains(canary), "data map leaked {canary}");
        }
        assert!(
            !out.contains(d.to_string_lossy().as_ref()),
            "data map leaked a local path"
        );
    }
    assert!(
        map.summary[0].contains("covering approximately 1 year with"),
        "{}",
        map.summary[0]
    );
    assert_eq!(
        map.collections[0].date_coverage.as_ref().unwrap().first,
        "2021-01-01"
    );
    assert!(map
        .preparation_priorities
        .iter()
        .any(|p| p.contains("1 image")));
}

#[test]
fn data_map_has_relationships_formats_and_activity() {
    let d = tempdir("slack-rel");
    write(&d, "Acme Slack export.zip", &export());
    let inv = scan(&[d.join("Acme Slack export.zip")], ScanOptions::default());
    let map = build_data_map(&inv);
    let to: Vec<_> = map
        .relationships
        .iter()
        .map(|r| (r.to, r.count, r.available_locally))
        .collect();
    assert!(
        to.contains(&("Images shared in conversations", 1, false)),
        "{to:?}"
    );
    assert!(
        to.contains(&("Documents shared in conversations", 1, false)),
        "{to:?}"
    );
    assert!(
        to.contains(&("Code reviews and repositories", 2, false)),
        "{to:?}"
    );
    assert_eq!(map.activity.values().sum::<u64>(), 6);
    let json = &map.collections[0].formats;
    assert_eq!(json[0].format, "JSON");
    assert_eq!(json[0].files, json[0].examined);
    assert!(!map.stopped_by_user);
}

#[test]
fn downloaded_attachments_are_matched_back_to_references() {
    let d = tempdir("slack-attach");
    let day = json!([
        {"ts": "1609459200.000100", "user": "U1", "text": "a",
         "files": [{"id": "F04AAAAAAA1", "mimetype": "image/png", "url_private_download": "https://files.slack.com/a"},
                   {"id": "F04AAAAAAA2", "mimetype": "image/jpeg", "url_private_download": "https://files.slack.com/b"},
                   {"id": "F04BBBBBBB1", "mimetype": "application/pdf", "url_private_download": "https://files.slack.com/c"}]}
    ]);
    let z = zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "eng"}]).to_string().as_bytes(),
        ),
        ("users.json", json!([{"id": "U1"}]).to_string().as_bytes()),
        ("eng/2021-01-01.json", day.to_string().as_bytes()),
    ]);
    write(&d, "export.zip", &z);
    write(
        &d,
        "Export attachments/attachments/F04AAAAAAA1/diagram.png",
        b"\x89PNG\r\n\x1a\nxx",
    );
    write(
        &d,
        "Export attachments/attachments/F04BBBBBBB1/spec.pdf",
        b"%PDF-1.7\n",
    );
    write(
        &d,
        "Export attachments/attachments/F9NOTREFD99/other.txt",
        b"unrelated\n",
    );

    let inv = scan(
        &[d.join("export.zip"), d.join("Export attachments")],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let img = map
        .relationships
        .iter()
        .find(|r| r.kind == "slack_images")
        .unwrap();
    assert_eq!(
        (img.count, img.resolved, img.available_locally),
        (2, 1, false)
    );
    assert_eq!(img.found_in, vec!["Export attachments".to_string()]);
    let doc = map
        .relationships
        .iter()
        .find(|r| r.kind == "slack_files")
        .unwrap();
    assert_eq!(
        (doc.count, doc.resolved, doc.available_locally),
        (1, 1, true)
    );
    assert!(
        map.summary
            .iter()
            .any(|s| s.contains("2 are included in your selection and 1 still missing")),
        "{:?}",
        map.summary
    );
    assert!(map
        .preparation_priorities
        .iter()
        .any(|p| p.starts_with("Add the original attachments: 1 image")));
}

#[test]
fn adding_data_later_matches_adding_it_at_once() {
    let d = tempdir("slack-extend");
    let day = json!([{"ts": "1609459200.000100", "user": "U1", "text": "a",
        "files": [{"id": "F04AAAAAAA1", "mimetype": "image/png", "url_private_download": "https://files.slack.com/a"},
                  {"id": "F04AAAAAAA2", "mimetype": "image/png", "url_private_download": "https://files.slack.com/b"}]}]);
    let z = zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "eng"}]).to_string().as_bytes(),
        ),
        ("users.json", b"[]"),
        ("eng/2021-01-01.json", day.to_string().as_bytes()),
    ]);
    write(&d, "export.zip", &z);
    write(
        &d,
        "export attachments/attachments/F04AAAAAAA1/a.png",
        b"\x89PNG\r\n\x1a\nx",
    );
    let export = d.join("export.zip");
    let attach = d.join("export attachments");

    let first = scan(std::slice::from_ref(&export), ScanOptions::default());
    let before = build_data_map(&first);
    let img = |m: &scout_core::DataMap| {
        let r = m
            .relationships
            .iter()
            .find(|r| r.kind == "slack_images")
            .unwrap();
        (r.count, r.resolved)
    };
    assert_eq!(img(&before), (2, 0));

    let mut seen = 0;
    let extended = scout_core::extend_scan(
        Some(first),
        &[export.clone(), attach.clone()],
        ScanOptions::default(),
        &mut |p| seen = p.files_seen,
    );
    assert_eq!(
        extended.collections.len(),
        2,
        "re-adding the export is ignored"
    );
    let after = build_data_map(&extended);
    assert_eq!(img(&after), (2, 1));
    assert_eq!(
        after.assessment_id, before.assessment_id,
        "the assessment keeps its ID"
    );

    let at_once = build_data_map(&scan(&[export, attach], ScanOptions::default()));
    assert_eq!(after.totals.logical_files, at_once.totals.logical_files);
    assert_eq!(
        after.totals.structured_records,
        at_once.totals.structured_records
    );
    assert_eq!(after.totals.expanded_bytes, at_once.totals.expanded_bytes);
    assert_eq!(img(&after), img(&at_once));
    assert_eq!(after.summary, at_once.summary);
    assert!(seen > 0);
}

#[test]
fn language_mix_covers_messages_and_documents() {
    let d = tempdir("lang");
    let en =
        "Can you check why the nightly build failed again after the dependency upgrade yesterday?";
    let de = "Kannst du bitte prüfen, warum der nächtliche Build nach dem Update gestern wieder fehlgeschlagen ist?";
    let day = json!([
        {"ts": "1.1", "user": "U1", "text": en},
        {"ts": "1.2", "user": "U1", "text": format!("<@U2> {en}")},
        {"ts": "1.3", "user": "U2", "text": de},
        {"ts": "1.4", "user": "U2", "text": "ok 👍"},
        {"ts": "1.5", "bot_id": "B1", "subtype": "bot_message", "text": "Ce message automatique ne devrait pas être compté dans les langues des personnes."}
    ]);
    let z = zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "eng"}]).to_string().as_bytes(),
        ),
        ("users.json", b"[]"),
        ("eng/2021-01-01.json", day.to_string().as_bytes()),
    ]);
    write(&d, "export.zip", &z);
    write(&d, "docs/handbook.md", b"# Handbook\n\nOur support team answers customer questions within one working day and escalates outages to engineering immediately.\n");
    write(&d, "docs/accueil.html", "<html><body><p>Bienvenue dans l'équipe. Ce document explique comment nous travaillons avec nos clients et nos partenaires.</p></body></html>".as_bytes());
    write(&d, "docs/report.pdf", b"%PDF-1.7\n");

    let inv = scan(
        &[d.join("export.zip"), d.join("docs")],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let l = &map.languages;
    let code = |c: &str| {
        l.rows
            .iter()
            .find(|r| r.code == c)
            .map(|r| r.words)
            .unwrap_or(0)
    };
    assert!(
        code("eng") > code("deu") && code("deu") > 0 && code("fra") > 0,
        "{:?}",
        l.rows
    );
    assert_eq!(l.undetermined_words, 2, "\"ok 👍\" is too short to tell");
    assert_eq!(l.unreadable_documents, 1);
    assert_eq!(l.by_collection.len(), 2);
    assert_eq!(l.by_collection[0].basis, "messages");
    // the bot's French message is not counted
    let slack_fr = l.by_collection[0].rows.iter().any(|r| r.code == "fra");
    assert!(!slack_fr);
    assert!(l.english_percent.is_some());
    let text = scout_core::report::render_text(&map);
    assert!(
        text.contains("Languages")
            && text.contains("English")
            && text.contains("Too short to tell"),
        "{text}"
    );
}

#[test]
fn data_sources_are_built_from_links_apps_and_bots() {
    let d = tempdir("sources");
    let day = json!([
        {"ts": "1.1", "user": "U1", "text": "fix in <https://github.com/acme/app/pull/7|PR 7> and https://github.com/acme/app/pull/8"},
        {"ts": "1.2", "user": "U1", "text": "spec: https://docs.google.com/document/d/SECRETDOC/edit?usp=sharing"},
        {"ts": "1.3", "bot_id": "B1", "subtype": "bot_message", "bot_profile": {"name": "Jira Cloud"},
         "attachments": [{"title_link": "https://acme.atlassian.net/browse/SUP-1"}]},
        {"ts": "1.4", "bot_id": "B1", "subtype": "bot_message", "username": "Jira Cloud", "text": "SUP-2 resolved"}
    ]);
    let z = zip_bytes(&[
        ("channels.json", json!([{"name": "eng"}]).to_string().as_bytes()),
        ("users.json", b"[]"),
        ("integration_logs.json", json!({"logs": [{"service_type": "GitHub", "change_type": "added"}, {"app_type": "Jira Cloud"}]}).to_string().as_bytes()),
        ("eng/2021-01-01.json", day.to_string().as_bytes()),
    ]);
    write(&d, "export.zip", &z);
    write(
        &d,
        "docs/notes.html",
        b"<p>Board: <a href=\"https://www.notion.so/acme/Roadmap-123\">roadmap</a></p>",
    );
    write(&d, "mail.mbox", b"From a@b Mon Jan 1\nFrom: a\nSubject: x\n\nSee https://docs.google.com/document/d/OTHER/edit\n");

    let inv = scan(
        &[d.join("export.zip"), d.join("docs"), d.join("mail.mbox")],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let get = |id: &str| {
        map.sources.iter().find(|s| s.id == id).unwrap_or_else(|| {
            panic!(
                "{id} missing: {:?}",
                map.sources.iter().map(|s| s.id).collect::<Vec<_>>()
            )
        })
    };
    assert!(get("slack").in_selection && get("email").in_selection);
    assert!(
        map.sources[0].in_selection,
        "sources already in the map come first"
    );
    let gh = get("github");
    assert_eq!(
        (gh.links, gh.distinct_targets, gh.app_installed),
        (2, 2, true)
    );
    let jira = get("jira");
    assert_eq!(
        (jira.links, jira.bot_messages, jira.app_installed),
        (1, 2, true)
    );
    let gd = get("google_drive");
    assert_eq!((gd.links, gd.distinct_targets), (2, 2));
    assert_eq!(
        gd.referenced_from,
        vec!["export.zip".to_string(), "mail.mbox".to_string()]
    );
    assert_eq!(get("notion").referenced_from, vec!["docs".to_string()]);
    assert!(
        gh.evidence.starts_with("2 links to 2 distinct pages"),
        "{}",
        gh.evidence
    );
    assert!(!map.source_catalog.is_empty());
    let json = serde_json::to_string(&map).unwrap();
    for leak in [
        "SECRETDOC",
        "acme/app/pull",
        "atlassian.net",
        "notion.so/acme",
        "OTHER",
    ] {
        assert!(!json.contains(leak), "data map leaked {leak}");
    }
}

#[test]
fn added_sources_are_recognised_and_linked_pages_resolved() {
    let d = tempdir("detect");
    let day = json!([
        {"ts": "1.1", "user": "U1", "text": "see https://github.com/acme/app/pull/1 and https://github.com/other/lib/pull/2"},
        {"ts": "1.2", "user": "U1", "text": "spec https://docs.google.com/document/d/abc/edit"}
    ]);
    let z = zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "eng"}]).to_string().as_bytes(),
        ),
        ("users.json", b"[]"),
        ("eng/2021-01-01.json", day.to_string().as_bytes()),
    ]);
    write(&d, "export.zip", &z);
    let takeout = zip_bytes(&[
        ("Takeout/archive_browser.html", b"<html></html>"),
        ("Takeout/Drive/Plans/Roadmap.md", b"# Roadmap\n"),
        (
            "Takeout/Mail/All mail.mbox",
            b"From a@b Mon\nFrom: a\nSubject: x\n\nhello\n",
        ),
    ]);
    write(&d, "takeout-20261106T153241Z-3-001.zip", &takeout);
    write(
        &d,
        "app/.git/config",
        b"[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:acme/app.git\n",
    );
    write(&d, "app/.git/HEAD", b"ref: refs/heads/main\n");
    write(&d, "app/src/main.rs", b"fn main() {}\n");

    let inv = scan(
        &[
            d.join("export.zip"),
            d.join("takeout-20261106T153241Z-3-001.zip"),
            d.join("app"),
        ],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let col = |l: &str| map.collections.iter().find(|c| c.label == l).unwrap();
    assert_eq!(
        col("takeout-20261106T153241Z-3-001.zip").display_label,
        "Google Takeout · Drive, Mail · part 3-001"
    );
    assert_eq!(col("app").display_label, "acme/app (Git repository)");
    let src = |id: &str| map.sources.iter().find(|s| s.id == id).unwrap();
    let gd = src("google_drive");
    assert!(gd.in_selection);
    assert_eq!(gd.detected_as, Some("Google Takeout (Drive)"));
    assert_eq!(gd.links, 1, "the step stays evidenced after it is done");
    let gh = src("github");
    assert!(gh.in_selection);
    assert_eq!(gh.detected_as, Some("a Git repository"));
    assert_eq!((gh.distinct_targets, gh.resolved_targets), (2, Some(1)));
    assert_eq!(src("email").detected_as, Some("Google Takeout (Mail)"));
}
