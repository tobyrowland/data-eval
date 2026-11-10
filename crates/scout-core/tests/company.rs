mod common;

use common::*;
use scout_core::{build_data_map, scan, ScanOptions};
use serde_json::json;
use std::path::{Path, PathBuf};

fn acme_slack(d: &Path) -> PathBuf {
    let users = json!([
        {"id": "U1", "real_name": "Ann Lee", "profile": {"real_name": "Ann Lee", "email": "ann@acme.com"}},
        {"id": "U2", "real_name": "Bo Carter", "profile": {"real_name": "Bo Carter", "email": "bo@acme.com"}}
    ]);
    let day = json!([
        {"ts": "1609459200.000100", "user": "U1", "text": "The ACME roadmap is in https://bitbucket.org/acme/app/pull-requests/3 and ACME docs need a pass."}
    ]);
    write(
        d,
        "ACME LTD Slack export Jan 1 2020 - Jan 1 2021.zip",
        &zip_bytes(&[
            (
                "channels.json",
                json!([{"name": "eng"}]).to_string().as_bytes(),
            ),
            ("users.json", users.to_string().as_bytes()),
            ("eng/2021-01-01.json", day.to_string().as_bytes()),
        ]),
    )
}

fn pixpaint_teams(d: &Path, with_attachments: bool) -> PathBuf {
    let attach = if with_attachments {
        json!([
            {"id": "a1", "contentType": "reference", "contentUrl": "https://pixpaint.example/files/PP-2062%20review%20notes.pdf", "name": "PP-2062 review notes.pdf"},
            {"id": "a2", "contentType": "reference", "contentUrl": "https://pixpaint.example/files/Brush%20spec.docx", "name": null},
            {"id": "a3", "contentType": "application/vnd.microsoft.card.adaptive", "content": "{}"}
        ])
    } else {
        json!([])
    };
    let msgs = json!({"@odata.context": "x", "value": [
        {"@odata.type": "#microsoft.graph.chatMessage", "id": "1", "messageType": "message", "createdDateTime": "2026-03-02T10:00:00Z",
         "chatId": "c1", "from": {"user": {"id": "u1", "displayName": "Dev Shah"}},
         "body": {"contentType": "text", "content": "Notes attached for PixPaint."}, "attachments": attach}
    ]});
    let list = json!({"@odata.context": "x", "value": [
        {"id": "T1", "displayName": "PixPaint Engineering", "resourceProvisioningOptions": ["Team"]},
        {"id": "T2", "displayName": "PixPaint Product & Design", "resourceProvisioningOptions": ["Team"]}
    ]});
    let members = json!({"@odata.context": "x", "value": [
        {"displayName": "Dev Shah", "userId": "u1", "email": "dev@pixpaint.example", "tenantId": "t-pix"}
    ]});
    write(
        d,
        "pixpaint-teams.zip",
        &zip_bytes(&[
            ("teams/teamsList", list.to_string().as_bytes()),
            (
                "teams/T1/teamMembers_T1.json",
                members.to_string().as_bytes(),
            ),
            ("users/u1/messages_u1.json", msgs.to_string().as_bytes()),
        ]),
    )
}

#[test]
fn company_name_from_slack_email_and_code() {
    let d = tempdir("company");
    let slack = acme_slack(&d);
    write(&d, "inbox.mbox", b"From x Mon\nFrom: Ann Lee <ann@acme.com>\nTo: cy@acme.com, dee@acme.com\nSubject: ACME\n\nHi.\n\n");
    let teams = pixpaint_teams(&d, false);
    let inv = scan(
        &[slack, d.join("inbox.mbox"), teams],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let c = map.company.as_ref().unwrap();
    assert_eq!(c.name, "ACME LTD", "{c:?}");
    assert_eq!(c.confidence, "high");
    assert!(c.evidence.iter().any(|e| e == "the Slack export name"));
    assert!(c
        .evidence
        .iter()
        .any(|e| e.contains("email domain acme.com")));
    assert!(c
        .evidence
        .iter()
        .any(|e| e.contains("Bitbucket workspace acme")));
}

#[test]
fn company_name_from_teams_alone_and_none_without_clues() {
    let d = tempdir("company-teams");
    let teams = pixpaint_teams(&d, false);
    let map = build_data_map(&scan(&[teams], ScanOptions::default()));
    let c = map.company.as_ref().unwrap();
    assert_eq!(c.name, "PixPaint", "{c:?}");

    write(&d, "notes/a.txt", b"buy milk\n");
    let map = build_data_map(&scan(&[d.join("notes")], ScanOptions::default()));
    assert!(map.company.is_none());
}

#[test]
fn teams_linked_files_are_matched_when_provided() {
    let d = tempdir("teams-files");
    let teams = pixpaint_teams(&d, true);
    // A SharePoint download holding one of the two linked files.
    write(
        &d,
        "SharePoint/Shared Documents/PP-2062 review notes.pdf",
        b"%PDF-1.4 review notes",
    );
    let inv = scan(&[teams, d.join("SharePoint")], ScanOptions::default());
    let map = build_data_map(&inv);
    let tf = map.teams_files.as_ref().unwrap();
    assert_eq!((tf.referenced, tf.found), (2, 1), "cards are not files");
    assert_eq!(tf.found_in, vec!["SharePoint".to_string()]);
    let sp = map
        .collections
        .iter()
        .find(|c| c.label == "SharePoint")
        .unwrap();
    assert_eq!(sp.platforms, vec!["teams"]);
    assert_eq!(sp.linkage.status, "linked", "{:?}", sp.linkage);
    let plan = scout_core::package::plan(&inv);
    let e = plan
        .entries
        .iter()
        .find(|e| e.collection == "SharePoint")
        .unwrap();
    assert_eq!(e.folders, vec!["Microsoft Teams/Linked files".to_string()]);
    let t = plan
        .entries
        .iter()
        .find(|e| e.collection == "pixpaint-teams.zip")
        .unwrap();
    assert_eq!(t.folders, vec!["Microsoft Teams".to_string()]);
}
