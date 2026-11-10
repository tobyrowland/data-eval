mod common;

use common::*;
use scout_core::browse::{children, linking_files};
use scout_core::model::Status;
use scout_core::{build_data_map, scan, update_scan, Inventory, ScanOptions};
use serde_json::json;
use std::path::{Path, PathBuf};

fn msg(ts: &str, text: &str, file: Option<&str>) -> serde_json::Value {
    let mut m = json!({"ts": ts, "user": "U1", "text": text});
    if let Some(f) = file {
        m["files"] =
            json!([{"id": f, "mimetype": "image/png", "url_private": "https://files.slack.com/x"}]);
    }
    m
}

fn export(channels: &[(&str, Vec<serde_json::Value>)]) -> Vec<u8> {
    let meta = json!(channels
        .iter()
        .map(|c| json!({"name": c.0}))
        .collect::<Vec<_>>())
    .to_string();
    let days: Vec<(String, String)> = channels
        .iter()
        .map(|(c, m)| (format!("{c}/2021-01-01.json"), json!(m).to_string()))
        .collect();
    let mut entries: Vec<(&str, &[u8])> =
        vec![("channels.json", meta.as_bytes()), ("users.json", b"[]")];
    for (n, d) in &days {
        entries.push((n.as_str(), d.as_bytes()));
    }
    zip_bytes(&entries)
}

fn id_of(inv: &Inventory, label: &str) -> u32 {
    inv.collections
        .iter()
        .find(|c| c.label == label)
        .unwrap()
        .id
}

fn update(inv: Inventory, remove: &[u32], reread: &[u32], exclude: Vec<String>) -> Inventory {
    update_scan(
        inv,
        remove,
        reread,
        ScanOptions {
            exclude,
            ..ScanOptions::default()
        },
        &mut |_| {},
    )
}

fn fixture(d: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let a = msg(
        "1609459200.000100",
        "see https://github.com/acme/app/pull/1 and https://github.com/acme/app/pull/3",
        Some("FAAAAAAAA01"),
    );
    let b = msg(
        "1609459300.000100",
        "hello there, a perfectly ordinary sentence in English",
        None,
    );
    let c = msg(
        "1609459400.000100",
        "new https://github.com/acme/app/pull/2",
        Some("FAAAAAAAA02"),
    );
    let r = msg(
        "1609459500.000100",
        "random chatter https://github.com/acme/app/pull/9",
        None,
    );
    write(
        d,
        "old.zip",
        &export(&[("eng", vec![a.clone(), b.clone()])]),
    );
    write(
        d,
        "new.zip",
        &export(&[("eng", vec![a, b, c]), ("random", vec![r])]),
    );
    write(
        d,
        "files/attachments/FAAAAAAAA01/a.png",
        b"\x89PNG\r\n\x1a\n....",
    );
    write(
        d,
        "files/attachments/FAAAAAAAA02/b.png",
        b"\x89PNG\r\n\x1a\n....",
    );
    (d.join("old.zip"), d.join("new.zip"), d.join("files"))
}

#[test]
fn removing_an_overlapping_export_recounts_the_other() {
    let d = tempdir("remove-overlap");
    let (old, new, files) = fixture(&d);
    let inv = scan(&[old, new.clone(), files.clone()], ScanOptions::default());
    let before = build_data_map(&inv);
    assert_eq!(before.activity.values().sum::<u64>(), 4);

    let gone = id_of(&inv, "old.zip");
    let after = update(inv, &[gone], &[], vec![]);
    // Item ids stay equal to their position.
    assert!(after
        .items
        .iter()
        .enumerate()
        .all(|(i, it)| it.id == i as u64));
    assert_eq!(after.collections.len(), 2);
    let map = build_data_map(&after);
    let alone = build_data_map(&scan(&[new, files], ScanOptions::default()));
    assert_eq!(map.activity, alone.activity, "every message counted again");
    assert_eq!(map.languages.words_examined, alone.languages.words_examined);
    let links =
        |m: &scout_core::DataMap| m.sources.iter().find(|s| s.id == "github").unwrap().links;
    assert_eq!(links(&map), links(&alone));
    let sf = map.slack_files.as_ref().unwrap();
    assert_eq!(
        (sf.images, sf.images_found),
        (2, 2),
        "attachments still matched"
    );
    assert_eq!(sf.exports, vec!["new.zip".to_string()]);
}

#[test]
fn removing_attachments_and_excluding_files() {
    let d = tempdir("remove-files");
    let (_, new, files) = fixture(&d);
    let inv = scan(&[new, files], ScanOptions::default());

    // The files tie each item to its step.
    let nid = id_of(&inv, "new.zip");
    let top = children(&inv, nid, None, None);
    let names: Vec<&str> = top.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["eng", "random", "channels.json", "users.json"]);
    let eng = &top[0];
    assert!(eng.folder);
    assert_eq!((eng.files, eng.links), (1, 3));
    let linking = linking_files(&inv, nid, "github", 10);
    assert_eq!(linking[0].path, "new.zip/eng/2021-01-01.json");
    assert_eq!(linking[0].links, 3);
    assert_eq!(linking.len(), 2);

    // Exclude a channel: only that item is read again.
    let ex = vec!["new.zip/random".to_string()];
    let targets = inv.collections_for_path(&ex[0]);
    assert_eq!(targets, vec![nid]);
    let inv = update(inv, &[], &targets, ex.clone());
    let nid = id_of(&inv, "new.zip");
    let random = children(&inv, nid, None, None)
        .into_iter()
        .find(|e| e.name == "random")
        .unwrap();
    assert!(random.excluded_here);
    assert_eq!(random.excluded_files, 1);
    let map = build_data_map(&inv);
    let gh = map.sources.iter().find(|s| s.id == "github").unwrap();
    assert_eq!(gh.links, 3, "links in the excluded channel are left out");
    let plan = scout_core::package::plan(&inv);
    assert!(plan.excluded_files >= 1);

    // Removing the attachments item: files are no longer matched.
    let fid = id_of(&inv, "files");
    let inv = update(inv, &[fid], &[], ex);
    let map = build_data_map(&inv);
    let sf = map.slack_files.as_ref().unwrap();
    assert_eq!(sf.images_found, 0);
    assert!(inv.attachments.is_empty());

    // Restoring the channel brings it back.
    let nid = id_of(&inv, "new.zip");
    let inv = update(inv, &[], &[nid], vec![]);
    let map = build_data_map(&inv);
    assert_eq!(
        map.sources.iter().find(|s| s.id == "github").unwrap().links,
        4
    );
    let nid = id_of(&inv, "new.zip");
    assert!(inv
        .items
        .iter()
        .filter(|it| it.collection == nid)
        .all(|it| it.status != Status::Excluded));
}

#[test]
fn file_names_never_reach_the_data_map() {
    let d = tempdir("browse-leak");
    let (_, new, _) = fixture(&d);
    let inv = scan(&[new], ScanOptions::default());
    let json = serde_json::to_string(&build_data_map(&inv)).unwrap();
    assert!(!json.contains("2021-01-01.json"));
    assert!(!json.contains("random/"));
}
