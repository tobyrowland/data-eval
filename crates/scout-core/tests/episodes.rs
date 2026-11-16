mod common;

use common::*;
use scout_core::evaluate::evaluate;
use scout_core::{scan, ScanOptions};
use serde_json::json;
use std::path::Path;

/// A small Bitbucket repository (tests/fixtures/platform-repo.zip, made
/// with git): three commits packed by `git gc` (two stored as deltas) and a
/// loose merge commit "Merged in bug-812 (pull request #7)". The fix commit
/// is "BUG-812: handle a missing region in the export".
const FIX_SHA: &str = "6682e15a4c7421e6e9677cba8e0f950599d5c764";

fn repo(d: &Path) -> String {
    let f = std::fs::File::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/platform-repo.zip"),
    )
    .unwrap();
    zip::ZipArchive::new(f).unwrap().extract(d).unwrap();
    FIX_SHA.to_string()
}

fn slack(thread_text: &[&str], extra: &[serde_json::Value]) -> Vec<u8> {
    let base = 1_614_679_200i64; // 2021-03-02
    let mut msgs: Vec<serde_json::Value> = thread_text
        .iter()
        .enumerate()
        .map(|(i, t)| {
            json!({"ts": format!("{}.00010{i}", base + i as i64 * 60), "thread_ts": format!("{base}.000100"), "user": format!("U{i}"), "text": t})
        })
        .collect();
    msgs.extend(extra.iter().cloned());
    zip_bytes(&[
        (
            "channels.json",
            json!([{"name": "support"}]).to_string().as_bytes(),
        ),
        ("users.json", b"[]"),
        (
            "support/2021-03-02.json",
            json!(msgs).to_string().as_bytes(),
        ),
    ])
}

fn fixture(name: &str, with_jira: bool, with_ci: bool) -> scout_core::Inventory {
    let d = tempdir(name);
    let fix = repo(&d);
    // Other tickets mentioned in passing (so BUG is a real project even
    // without the tracker's export).
    let mut extra = vec![
        json!({"ts": "1614790000.000001", "user": "U3", "text": "BUG-901 looks like a duplicate of an older report"}),
        json!({"ts": "1614790600.000001", "user": "U4", "text": "Can someone triage BUG-902 today?"}),
    ];
    // Sixty separate messages citing the same pinned page: a hub, which
    // must not join them into one episode.
    let hub: Vec<serde_json::Value> = (0..60)
        .map(|i| json!({"ts": format!("{}.000{i:03}", 1_614_800_000 + i * 600), "user": "U9", "text": format!("See the runbook https://acme.atlassian.net/wiki/spaces/OPS/pages/42 item {i}")}))
        .collect();
    extra.extend(hub);
    write(
        &d,
        "slack.zip",
        &slack(
            &[
                "Customers say the CSV export fails when the region is empty. Tracking it in BUG-812.",
                "I have a fix up for review: https://bitbucket.org/acme/platform/pull-requests/7",
                "Merged, thanks. The nightly build is green again.",
            ],
            &extra,
        ),
    );
    write(&d, "mail.mbox", b"From a Tue\nFrom: Ann <ann@customer.example>\nMessage-ID: <root-1@customer.example>\nDate: Mon, 1 Mar 2021 09:00:00 +0000\nSubject: Export broken\n\nHello, our export has failed every day this week. Can you look into it please?\n\nFrom b Tue\nFrom: Bo <bo@acme.com>\nMessage-ID: <reply-1@acme.com>\nIn-Reply-To: <root-1@customer.example>\nReferences: <root-1@customer.example>\nDate: Mon, 1 Mar 2021 11:00:00 +0000\nSubject: Re: Export broken\n\nThanks Ann, we are tracking this as BUG-812 and will let you know when the fix ships.\n\n");
    if with_jira {
        write(&d, "jira.csv", b"Summary,Issue key,Issue Type,Status,Created,Resolved,Description\n\"Export fails on empty region\",BUG-812,Bug,Done,2021-03-01 10:00,2021-03-03 16:00,\"The CSV export throws when a customer has no region set.\"\nOther bug,BUG-813,Bug,Open,2021-03-04 10:00,,Something else entirely\nThird bug,BUG-814,Bug,Open,2021-03-05 10:00,,Yet another unrelated problem\n");
    }
    std::fs::create_dir_all(d.join("ci")).unwrap();
    if with_ci {
        write(
        &d,
        "ci/build_1.txt",
        format!("2021-03-02T10:05:00.0000000Z ##[group]Run actions/checkout@v4\n2021-03-02T10:05:02.0000000Z HEAD is now at {} BUG-812: handle a missing region\n2021-03-02T10:06:00.0000000Z ##[endgroup]\n2021-03-02T10:09:00.0000000Z Cleaning up orphan processes\n", &fix[..8]).as_bytes(),
    );
    }
    // Unrelated data: no shared references.
    write(&d, "discord.zip", &zip_bytes(&[("messages/c1/messages.json", json!([{"ID": "1", "Timestamp": "2021-05-01 10:00:00", "Contents": "anyone up for a game tonight?"}]).to_string().as_bytes())]));
    let mut inputs = vec![
        d.join("slack.zip"),
        d.join("platform"),
        d.join("mail.mbox"),
        d.join("ci"),
        d.join("discord.zip"),
    ];
    if with_jira {
        inputs.push(d.join("jira.csv"));
    }
    scan(&inputs, ScanOptions::default())
}

#[test]
fn one_piece_of_work_across_five_systems_is_one_complete_episode() {
    let inv = fixture("episodes", true, true);
    let ev = evaluate(&inv, None, &mut |_| {});
    let e = &ev.episodes;
    assert_eq!(e.commits, 4, "{e:#?}");
    assert_eq!(e.tickets, 3);
    assert_eq!((e.ci_logs, e.ci_passed), (1, 1));
    assert_eq!(e.episodes, 1, "{e:#?}");
    assert_eq!(e.complete, 1);
    assert_eq!(e.multi_system, 1);
    assert_eq!(e.pre_llm, 1);
    // Slack thread, email thread, two commits, the ticket and the CI log.
    assert_eq!(e.records_linked, 6, "{e:#?}");
    let linked: Vec<&str> = e
        .systems
        .iter()
        .filter(|s| s.linked > 0)
        .map(|s| s.id)
        .collect();
    assert_eq!(linked, vec!["bitbucket", "ci", "email", "jira", "slack"]);
    // Jira, the repository and CI logs are all here; only the wiki page
    // cited sixty times points to a system not uploaded.
    let gaps: Vec<&str> = e.gaps.iter().map(|g| g.id).collect();
    assert_eq!(gaps, vec!["confluence"], "{:?}", e.gaps);
    assert_eq!(e.gaps[0].records, 60);
    // Measurements only: no keys, hashes or links in the result or report.
    let map = scout_core::build_data_map(&inv);
    let md = scout_core::evaluate::wm_report::wm_report(&map, &ev, None, &json!({}));
    let out = serde_json::to_string(&ev).unwrap() + &md;
    for leak in [
        "BUG-812",
        "pull-requests",
        "bitbucket.org",
        "root-1@",
        "OPS/pages",
    ] {
        assert!(!out.contains(leak), "leaked {leak}");
    }
    assert!(md.contains("## Work trajectories"));
}

#[test]
fn without_the_ticket_export_jira_is_the_gap_to_fill() {
    let inv = fixture("episodes-gap", false, true);
    let ev = evaluate(&inv, None, &mut |_| {});
    let e = &ev.episodes;
    // Still one episode (Slack, email, commits, CI), now without an intent
    // record from the tracker but with discussion, action and outcome.
    assert_eq!(e.episodes, 1, "{e:#?}");
    assert_eq!(e.complete, 1);
    let jira = e.gaps.iter().find(|g| g.id == "jira").expect("Jira gap");
    // The Slack thread, the email thread and the fix commit cite BUG-812;
    // two more messages cite other tickets.
    assert_eq!(jira.records, 5, "{:?}", e.gaps);
    assert_eq!(jira.distinct_targets, 3);
    assert!(jira.advice.contains("CSV"));
}

#[test]
fn without_results_nothing_is_complete_and_the_gaps_say_what_would_complete_it() {
    // Like many real exports: chat, email and code, but no tracker and no
    // build logs, so no piece of work records how it turned out.
    let inv = fixture("episodes-noresult", false, false);
    let e = evaluate(&inv, None, &mut |_| {}).episodes;
    assert_eq!(e.episodes, 1, "{e:#?}");
    assert_eq!(e.complete, 0);
    assert_eq!((e.with_why, e.with_action, e.with_outcome), (1, 1, 0));
    let gap = |id: &str| {
        e.gaps
            .iter()
            .find(|g| g.id == id)
            .unwrap_or_else(|| panic!("{id}: {:?}", e.gaps))
    };
    // A Jira export adds the request and the result; CI logs add the result.
    assert_eq!(gap("jira").would_complete, 1);
    assert_eq!(gap("jira").adds, vec!["why", "how it turned out"]);
    assert_eq!(gap("ci").would_complete, 1);
    // The tickets mentioned only in passing would become new linked work.
    assert_eq!(gap("jira").would_add, 2);
    // Completing gaps come first.
    assert!(e.gaps[0].would_complete >= e.gaps.last().unwrap().would_complete);
    assert_eq!(e.shapes[0].episodes, 1);
    assert!(e.shapes[0].why && e.shapes[0].action && !e.shapes[0].outcome);
}

#[test]
fn test_jira_data_from_the_owners_own_mentions_completes_linked_work_and_is_flagged() {
    let inv = fixture("episodes-testjira", false, false);
    let inputs: Vec<_> = inv.collections.iter().map(|c| c.source.clone()).collect();
    let mentions = scout_core::evaluate::ticket_mentions(inputs.clone(), vec![], None, &mut |_| {});
    let keys: Vec<&str> = mentions.iter().map(|m| m.key.as_str()).collect();
    // Most mentioned first: BUG-812 (Slack thread, email, commit), then the others.
    assert_eq!(keys, vec!["BUG-812", "BUG-901", "BUG-902"], "{mentions:?}");
    let bug = &mentions[0];
    assert!(
        bug.mentions >= 3 && bug.systems.contains(&"slack") && bug.systems.contains(&"email"),
        "{bug:?}"
    );
    assert!(bug.first.is_some() && bug.last >= bug.first);
    let csv = scout_core::evaluate::testdata::test_jira_csv(&mentions);
    let out = tempdir("episodes-testjira-out");
    let file = write(&out, "test-jira.csv", csv.as_bytes());
    let mut with = inputs;
    with.push(file);
    let inv2 = scan(&with, ScanOptions::default());
    let ev = evaluate(&inv2, None, &mut |_| {});
    let e = &ev.episodes;
    assert_eq!((e.tickets, e.test_tickets), (3, 3));
    // Jira no longer a gap; BUG-812 joins the work as its request (and its
    // result, if the test ticket came out resolved).
    assert!(e.gaps.iter().all(|g| g.id != "jira"), "{:?}", e.gaps);
    let done = csv
        .lines()
        .any(|l| l.contains(",BUG-812,") && l.contains(",Done,"));
    assert_eq!(e.complete, u64::from(done), "{e:#?}");
    // The report says loudly that it contains test data.
    let md = scout_core::evaluate::wm_report::wm_report(
        &scout_core::build_data_map(&inv2),
        &ev,
        None,
        &json!({}),
    );
    assert!(md.contains("TEST DATA"), "report must flag test data");
    // Normal evaluations keep no ticket text.
    assert!(!serde_json::to_string(&ev).unwrap().contains("BUG-812"));
}

#[test]
fn an_evaluation_also_returns_the_ticket_mentions_without_putting_them_in_its_output() {
    let inv = fixture("episodes-mentions", false, false);
    let inputs: Vec<_> = inv.collections.iter().map(|c| c.source.clone()).collect();
    let labels: Vec<_> = inv.collections.iter().map(|c| c.label.clone()).collect();
    let (ev, mentions) = scout_core::evaluate::evaluate_inputs_with_mentions(
        inputs.clone(),
        labels,
        vec![],
        None,
        &mut |_| {},
    );
    let keys: Vec<&str> = mentions.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, vec!["BUG-812", "BUG-901", "BUG-902"]);
    let alone = scout_core::evaluate::ticket_mentions(inputs, vec![], None, &mut |_| {});
    assert_eq!(alone.len(), mentions.len());
    assert!(!serde_json::to_string(&ev).unwrap().contains("BUG-"));
}
