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
