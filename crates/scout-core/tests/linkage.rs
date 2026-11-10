mod common;

use common::*;
use scout_core::{build_data_map, scan, update_scan, DataMap, ScanOptions};
use serde_json::json;
use std::path::{Path, PathBuf};

fn slack_export(d: &Path) -> PathBuf {
    let users = json!([
        {"id": "U1", "name": "ann", "real_name": "Ann Lee", "profile": {"real_name": "Ann Lee", "email": "ann@acme.com"}},
        {"id": "U2", "name": "bo", "real_name": "Bo Carter", "profile": {"real_name": "Bo Carter", "email": "bo@acme.com"}}
    ]);
    let day = json!([
        {"ts": "1609459200.000100", "user": "U1", "text": "The Hydra launch for Contoso moves to May, and Hydra docs need a pass."},
        {"ts": "1612137600.000100", "user": "U2", "text": "Contoso signed off on Hydra pricing."}
    ]);
    write(
        d,
        "acme-slack.zip",
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

fn chat(id: &str, chat: &str, who: &str, uid: &str, when: &str, text: &str) -> serde_json::Value {
    json!({"@odata.type": "#microsoft.graph.chatMessage", "id": id, "messageType": "message",
           "createdDateTime": when, "chatId": chat, "channelIdentity": null,
           "from": {"application": null, "user": {"id": uid, "displayName": who, "userIdentityType": "aadUser"}},
           "body": {"contentType": "text", "content": text}})
}

/// A Microsoft Graph tenant export for an unrelated company.
fn teams_export(d: &Path) -> PathBuf {
    let members = json!({"@odata.context": "x", "value": [
        {"@odata.type": "#microsoft.graph.aadUserConversationMember", "displayName": "Dev Shah", "userId": "u1", "email": "dev@pixpaint.example", "tenantId": "t-pix"},
        {"@odata.type": "#microsoft.graph.aadUserConversationMember", "displayName": "Noor Vaziri", "userId": "u2", "email": "noor@pixpaint.example", "tenantId": "t-pix"},
        {"@odata.type": "#microsoft.graph.aadUserConversationMember", "displayName": "Lin Calder", "userId": "u3", "email": "lin@pixpaint.example", "tenantId": "t-pix"}
    ]});
    let a = chat(
        "1",
        "c1",
        "Dev Shah",
        "u1",
        "2026-03-02T10:00:00Z",
        "The PixPaint canvas needs the Brushwork fix before Brushwork ships.",
    );
    let b = chat(
        "2",
        "c1",
        "Noor Vaziri",
        "u2",
        "2026-03-02T10:05:00Z",
        "Agreed, PixPaint tile cache first.",
    );
    let both = json!({"@odata.context": "x", "value": [a, b]});
    let list = json!({"@odata.context": "x", "value": [{"id": "T1", "displayName": "PixPaint Engineering", "mail": "eng@pixpaint.example", "resourceProvisioningOptions": ["Team"]}]});
    write(
        d,
        "pixpaint-teams.zip",
        &zip_bytes(&[
            ("teams/teamsList", list.to_string().as_bytes()),
            (
                "teams/T1/teamMembers_T1.json",
                members.to_string().as_bytes(),
            ),
            // The same chat in both participants' files.
            ("users/u1/messages_u1.json", both.to_string().as_bytes()),
            ("users/u2/messages_u2.json", both.to_string().as_bytes()),
        ]),
    )
}

fn col<'a>(m: &'a DataMap, label: &str) -> &'a scout_core::report::CollectionSummary {
    m.collections.iter().find(|c| c.label == label).unwrap()
}

#[test]
fn unrelated_teams_export_is_flagged_and_related_data_is_linked() {
    let d = tempdir("linkage");
    let slack = slack_export(&d);
    write(
        &d,
        "inbox.mbox",
        b"From x Mon\nFrom: Ann Lee <ann@acme.com>\nTo: bo@acme.com\nMessage-ID: <1@mail.acme.com>\nSubject: Hydra\n\nThe Hydra plan for Contoso.\n\n",
    );
    let teams = teams_export(&d);
    let inv = scan(
        &[slack, d.join("inbox.mbox"), teams],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);

    let s = &col(&map, "acme-slack.zip").linkage;
    assert_eq!(s.status, "linked", "{s:?}");
    assert!(
        s.shared_addresses >= 1 && s.shared_organisations >= 1,
        "{s:?}"
    );
    assert_eq!(col(&map, "inbox.mbox").linkage.status, "linked");

    let t = col(&map, "pixpaint-teams.zip");
    assert_eq!(t.platforms, vec!["teams"]);
    let tm = t.teams.as_ref().unwrap();
    assert_eq!(
        tm.messages, 2,
        "a chat in both participants' files counts once"
    );
    assert_eq!((tm.teams, tm.chats), (1, 1));
    assert_eq!(t.linkage.status, "unlinked", "{:?}", t.linkage);
    assert_eq!(t.linkage.dates_overlap, Some(false));
    assert!(
        t.linkage.explanation.contains("2026"),
        "{}",
        t.linkage.explanation
    );
    assert!(t
        .explanation
        .starts_with("A Microsoft Teams export with 2 messages"));
    // Teams messages count once in the headline figures too.
    assert_eq!(map.totals.unique_records, 2 + 1 + 2);

    // Names, addresses and terms never reach the map.
    let json = serde_json::to_string(&map).unwrap();
    for leak in ["Dev Shah", "pixpaint.example", "Hydra", "Contoso"] {
        assert!(!json.contains(leak), "leaked {leak}");
    }
    // The company's own domain appears only as evidence for its name.
    let company = map.company.as_ref().unwrap();
    assert_eq!(json.matches("acme.com").count(), 1, "{:?}", company);
    assert!(company.evidence.iter().any(|e| e.contains("acme.com")));

    // With the other items removed there is nothing to compare against.
    let keep = inv
        .collections
        .iter()
        .find(|c| c.label == "pixpaint-teams.zip")
        .unwrap()
        .id;
    let others: Vec<u32> = inv
        .collections
        .iter()
        .map(|c| c.id)
        .filter(|&i| i != keep)
        .collect();
    let alone = update_scan(inv, &others, &[], ScanOptions::default(), &mut |_| {});
    assert_eq!(
        build_data_map(&alone).collections[0].linkage.status,
        "unknown"
    );
}

#[test]
fn public_domains_alone_do_not_link_and_tiny_items_are_unknown() {
    let d = tempdir("linkage-public");
    write(&d, "a/notes.txt", b"Write to alice.one@gmail.com, see https://github.com/torvalds/linux and https://www.google.com. Notes from Ingrid about Falcon, Falcon again, then Ingrid and Orbit and Orbit.\n");
    write(&d, "b/notes.txt", b"Write to bob.two@gmail.com, see https://www.google.com and https://slack.com/help. Notes from Pavel about Quasar, Quasar again, then Pavel and Nimbus and Nimbus.\n");
    write(&d, "c/README.md", b"# Readme\nhi\n");
    let inv = scan(
        &[d.join("a"), d.join("b"), d.join("c")],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    assert_ne!(
        col(&map, "a").linkage.status,
        "linked",
        "{:?}",
        col(&map, "a").linkage
    );
    assert_ne!(col(&map, "b").linkage.status, "linked");
    assert_eq!(col(&map, "c").linkage.status, "unknown");
}

/// A Discord "Request your data" package for the same unrelated company.
fn discord_package(d: &Path) -> PathBuf {
    let user = json!({"id": "9", "username": "dev.pixpaint", "global_name": "Dev Shah", "email": "dev@pixpaint.example"});
    let index = json!({"11": "general", "12": null});
    let general = json!({"id": "11", "type": "GUILD_TEXT", "name": "general", "guild": {"id": "G1", "name": "PixPaint"}});
    let dm = json!({"id": "12", "type": "DM", "recipients": ["9", "8"]});
    let msgs = json!([
        {"ID": "100", "Timestamp": "2026-04-01 09:00:00", "Contents": "The PixPaint Brushwork build is green.", "Attachments": ""},
        {"ID": "101", "Timestamp": "2026-04-02 09:00:00", "Contents": "Brushwork notes are in the PixPaint wiki.", "Attachments": "https://cdn.discordapp.com/attachments/1/2/a.png"}
    ]);
    let csv = "ID,Timestamp,Contents,Attachments\n200,2026-04-03 10:00:00,\"hi, Noor\",\n";
    write(
        d,
        "pixpaint-discord.zip",
        &zip_bytes(&[
            ("Account/user.json", user.to_string().as_bytes()),
            ("Messages/index.json", index.to_string().as_bytes()),
            ("Messages/c11/channel.json", general.to_string().as_bytes()),
            ("Messages/c11/messages.json", msgs.to_string().as_bytes()),
            ("Messages/c12/channel.json", dm.to_string().as_bytes()),
            ("Messages/c12/messages.csv", csv.as_bytes()),
            ("Servers/index.json", br#"{"G1": "PixPaint"}"#),
        ]),
    )
}

#[test]
fn a_group_of_unrelated_exports_is_flagged_together() {
    let d = tempdir("linkage-group");
    let slack = slack_export(&d);
    write(
        &d,
        "inbox.mbox",
        b"From x Mon\nFrom: Ann Lee <ann@acme.com>\nTo: bo@acme.com\nCc: cy@acme.com\nSubject: Hydra\n\nThe Hydra plan for Contoso.\n\n",
    );
    write(
        &d,
        "docs/plan.md",
        b"# Plan\nAsk ann@acme.com about the Hydra launch for Contoso.\n",
    );
    let teams = teams_export(&d);
    let discord = discord_package(&d);
    let inv = scan(
        &[slack, d.join("inbox.mbox"), d.join("docs"), teams, discord],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);

    let dc = col(&map, "pixpaint-discord.zip");
    assert_eq!(dc.platforms, vec!["discord"]);
    let di = dc.discord.as_ref().unwrap();
    assert_eq!(
        (di.messages, di.servers, di.channels, di.direct_messages),
        (3, 1, 1, 1)
    );
    assert!(di.own_messages_only);
    assert!(dc
        .explanation
        .contains("only the messages written by the account"));

    // Teams and Discord vouch for each other, but not for the main data.
    for label in ["pixpaint-teams.zip", "pixpaint-discord.zip"] {
        let l = &col(&map, label).linkage;
        assert_eq!(l.status, "unlinked", "{label}: {l:?}");
        assert!(
            l.explanation.starts_with("Only connected to"),
            "{}",
            l.explanation
        );
    }
    assert_eq!(col(&map, "acme-slack.zip").linkage.status, "linked");
    assert_eq!(col(&map, "inbox.mbox").linkage.status, "linked");
    assert_eq!(col(&map, "docs").linkage.status, "linked");
    // Slack 2 + email 1 + Teams 2 + Discord 3, each counted once.
    assert_eq!(map.totals.unique_records, 8);
    let json = serde_json::to_string(&map).unwrap();
    assert!(!json.contains("Brushwork") && !json.contains("dev.pixpaint"));
}

/// A busy main company: many messages with common first names, everyday
/// capitalised words (also used in lower case), Discord invite links and
/// recent dates.
fn busy_slack(d: &Path, extra: &str) -> PathBuf {
    let users = json!([
        {"id": "U1", "real_name": "Ann Lee", "profile": {"real_name": "Ann Lee", "email": "ann@acme.com"}},
        {"id": "U2", "real_name": "Bo Carter", "profile": {"real_name": "Bo Carter", "email": "bo@acme.com"}}
    ]);
    let mut msgs = Vec::new();
    for i in 0..60 {
        msgs.push(json!({"ts": format!("{}.000100", 1_767_225_600 + i * 86_400), "user": "U1",
            "text": format!("Ask Alex and Sam about the Canvas Release for Hydra. The canvas release notes for hydra are in the doc. Join us at https://discord.gg/acme{i}. {extra}")}));
    }
    write(
        d,
        "acme-busy.zip",
        &zip_bytes(&[
            (
                "channels.json",
                json!([{"name": "eng"}]).to_string().as_bytes(),
            ),
            ("users.json", users.to_string().as_bytes()),
            ("eng/2026-01-01.json", json!(msgs).to_string().as_bytes()),
        ]),
    )
}

fn other_company_discord(d: &Path, server: &str) -> PathBuf {
    let user = json!({"id": "9", "username": "mira.pixpaint", "global_name": "Mira Halden", "email": "mira@pixpaint.example"});
    let channel = json!({"id": "11", "type": "GUILD_TEXT", "name": "general", "guild": {"id": server, "name": "PixPaint"}});
    let msgs: Vec<serde_json::Value> = (0..40)
        .map(|i| json!({"ID": format!("{}", 100 + i), "Timestamp": "2026-03-01 09:00:00",
            "Contents": "Alex and Sam said the Canvas Release slips; the Brushwork beta for PixPaint ships after the Brushwork review.", "Attachments": ""}))
        .collect();
    write(
        d,
        "other-discord.zip",
        &zip_bytes(&[
            ("Account/user.json", user.to_string().as_bytes()),
            ("Messages/c11/channel.json", channel.to_string().as_bytes()),
            (
                "Messages/c11/messages.json",
                json!(msgs).to_string().as_bytes(),
            ),
            (
                "Servers/index.json",
                format!("{{\"{server}\": \"PixPaint\"}}").as_bytes(),
            ),
        ]),
    )
}

#[test]
fn another_companys_discord_is_flagged_despite_common_names_and_dates() {
    let d = tempdir("linkage-busy");
    let slack = busy_slack(&d, "");
    write(&d, "inbox.mbox", b"From x Mon\nFrom: Ann Lee <ann@acme.com>\nTo: bo@acme.com\nSubject: Hydra\n\nHydra plan.\n\n");
    let discord = other_company_discord(&d, "555");
    let inv = scan(
        &[slack, d.join("inbox.mbox"), discord],
        ScanOptions::default(),
    );
    let map = build_data_map(&inv);
    let l = &col(&map, "other-discord.zip").linkage;
    assert_eq!(l.status, "unlinked", "{l:?}");
    assert_eq!(
        l.shared_people + l.shared_addresses + l.shared_organisations,
        0
    );
    assert_eq!(col(&map, "acme-busy.zip").linkage.status, "linked");
}

#[test]
fn a_linked_discord_server_or_teams_tenant_connects() {
    let d = tempdir("linkage-ties");
    // The Slack data links to a channel on the Discord server.
    let slack = busy_slack(
        &d,
        "Thread: https://discord.com/channels/555/11/100 and https://teams.microsoft.com/l/channel/19%3Aabc?groupId=g&tenantId=t-pix",
    );
    let discord = other_company_discord(&d, "555");
    let teams = teams_export(&d);
    let inv = scan(&[slack, discord, teams], ScanOptions::default());
    let map = build_data_map(&inv);
    let dl = &col(&map, "other-discord.zip").linkage;
    assert_eq!(dl.status, "linked", "{dl:?}");
    assert!(dl.direct_links > 0);
    let tl = &col(&map, "pixpaint-teams.zip").linkage;
    assert_eq!(tl.status, "linked", "{tl:?}");
    assert!(tl.shared_organisations > 0, "tenant from the Teams link");
}
