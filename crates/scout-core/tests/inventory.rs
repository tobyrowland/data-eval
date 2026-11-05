mod common;
use common::*;
use scout_core::model::{Format, Reason, Status};
use scout_core::{build_data_map, scan, Limits, ScanOptions};

fn find<'a>(inv: &'a scout_core::Inventory, suffix: &str) -> &'a scout_core::model::Item {
    inv.items
        .iter()
        .find(|i| i.local_path.ends_with(suffix))
        .unwrap_or_else(|| {
            panic!(
                "no item ending {suffix}: {:#?}",
                inv.items.iter().map(|i| &i.local_path).collect::<Vec<_>>()
            )
        })
}

#[test]
fn mixed_folder_keeps_every_item_in_totals() {
    let d = tempdir("mixed");
    write(&d, "data/table.csv", b"a,b\n1,2\n3,4\n");
    write(&d, "data/notes.md", b"# Notes\nsome words here\n");
    write(&d, "data/blob.7z", b"7z\xBC\xAF\x27\x1C rest of archive");
    write(&d, "data/photo.png", b"\x89PNG\r\n\x1a\nbinary");
    write(&d, "data/unknown.bin", &[0, 159, 146, 150, 0, 1]);
    write(&d, "data/copy.csv", b"a,b\n1,2\n3,4\n");
    let inv = scan(&[d.join("data")], ScanOptions::default());
    assert_eq!(inv.items.len(), 6);
    assert_eq!(find(&inv, "table.csv").records, Some(2));
    assert_eq!(find(&inv, "table.csv").status, Status::Assessed);
    assert_eq!(find(&inv, "blob.7z").status, Status::Inventoried);
    assert_eq!(find(&inv, "photo.png").format, Format::Image);
    assert_eq!(find(&inv, "unknown.bin").status, Status::Inventoried);
    let map = build_data_map(&inv);
    assert_eq!(map.totals.logical_files, 6);
    assert_eq!(map.totals.exact_duplicate_files, 1);
    let sum: u64 = map.coverage.by_status.values().sum();
    assert_eq!(sum, 6, "every item has exactly one status");
    assert_eq!(
        map.coverage.by_status.len(),
        7,
        "all statuses present, zeros included"
    );
}

#[test]
fn archives_expand_into_logical_files_and_office_docs_stay_whole() {
    let d = tempdir("archives");
    let docx = zip_bytes(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("word/document.xml", b"<w/>"),
    ]);
    let inner_tgz = gzip(&tar_bytes(&[
        ("logs/a.jsonl", b"{\"x\":1}\n{\"x\":2}\n"),
        ("logs/b.txt", b"hello world\n"),
    ]));
    let outer = zip_bytes(&[
        ("export/report.docx", &docx),
        ("export/logs.tgz", &inner_tgz),
        ("export/people.csv", b"id,name\n1,a\n"),
    ]);
    write(&d, "export.zip", &outer);
    let inv = scan(&[d.join("export.zip")], ScanOptions::default());
    let map = build_data_map(&inv);
    assert_eq!(find(&inv, "report.docx").format, Format::OfficeDocument);
    assert_eq!(find(&inv, "logs/a.jsonl").records, Some(2));
    assert_eq!(find(&inv, "logs/a.jsonl").status, Status::Assessed);
    assert_eq!(find(&inv, "export.zip").status, Status::Assessed);
    // docx, a.jsonl, b.txt, people.csv
    assert_eq!(map.totals.logical_files, 4);
    assert_eq!(map.totals.archives_expanded, 3); // zip, tgz, inner tar
    assert!(map.totals.expanded_bytes > 0);
}

#[test]
fn unsafe_member_paths_are_flagged_not_trusted() {
    let d = tempdir("traversal");
    let t = tar_bytes(&[("../../etc/evil.txt", b"x"), ("ok.txt", b"fine\n")]);
    write(&d, "bad.tar", &t);
    let inv = scan(&[d.join("bad.tar")], ScanOptions::default());
    let evil = find(&inv, "evil.txt");
    assert_eq!(evil.status, Status::StoppedSafely);
    assert_eq!(evil.reason, Some(Reason::UnsafeMemberPath));
    assert_eq!(find(&inv, "ok.txt").status, Status::Assessed);
    // Nothing was written outside: the scanner never extracts to disk.
    assert!(!d.join("../etc/evil.txt").exists());
}

#[test]
fn compression_bomb_is_stopped_safely() {
    let d = tempdir("bomb");
    let zeros = vec![0u8; 50 * 1024 * 1024];
    let z = zip_bytes(&[("zeros.txt", &zeros), ("small.txt", b"hello\n")]);
    write(&d, "bomb.zip", &z);
    let mut opts = ScanOptions::default();
    opts.limits.ratio_check_floor_bytes = 1024 * 1024;
    opts.limits.max_compression_ratio = 100;
    let inv = scan(&[d.join("bomb.zip")], opts);
    let bomb = find(&inv, "zeros.txt");
    assert_eq!(bomb.status, Status::StoppedSafely);
    assert_eq!(bomb.reason, Some(Reason::CompressionRatio));
    assert!(bomb.expanded_bytes < 10 * 1024 * 1024);
    assert_eq!(find(&inv, "small.txt").status, Status::Assessed);
    assert!(inv.limits_reached.contains(&Reason::CompressionRatio));
}

#[test]
fn nesting_limit_is_enforced() {
    let d = tempdir("nesting");
    let mut z = zip_bytes(&[("leaf.txt", b"deep\n")]);
    for i in 0..5 {
        z = zip_bytes(&[(&format!("level{i}.zip"), &z)]);
    }
    write(&d, "nest.zip", &z);
    let inv = scan(&[d.join("nest.zip")], ScanOptions::default());
    let stopped: Vec<_> = inv
        .items
        .iter()
        .filter(|i| i.reason == Some(Reason::NestingLimit))
        .collect();
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0].archive_depth, 4);
    assert!(!inv.items.iter().any(|i| i.local_path.ends_with("leaf.txt")));
}

#[test]
fn expansion_budget_still_inventories_the_rest() {
    let d = tempdir("budget");
    for i in 0..5 {
        write(&d, &format!("in/file{i}.txt"), &vec![b'a'; 4000]);
    }
    let opts = ScanOptions {
        limits: Limits {
            max_expanded_bytes: 9000,
            ..Limits::default()
        },
        exclude: vec![],
        cancel: None,
    };
    let inv = scan(&[d.join("in")], opts);
    assert_eq!(inv.items.len(), 5, "every file stays in the inventory");
    assert!(inv.items.iter().any(|i| i.status == Status::StoppedSafely));
    assert!(inv.limits_reached.contains(&Reason::ExpansionLimit));
    let map = build_data_map(&inv);
    assert!(map
        .summary
        .iter()
        .any(|s| s.contains("not a full assessment")));
}

#[test]
fn logical_file_limit_stops_enumeration_and_says_so() {
    let d = tempdir("filelimit");
    for i in 0..10 {
        write(&d, &format!("in/{i}.txt"), b"x");
    }
    let opts = ScanOptions {
        limits: Limits {
            max_logical_files: 4,
            ..Limits::default()
        },
        exclude: vec![],
        cancel: None,
    };
    let inv = scan(&[d.join("in")], opts);
    assert_eq!(inv.items.len(), 4);
    assert!(inv.limits_reached.contains(&Reason::LogicalFileLimit));
}

#[test]
fn exclusions_are_counted_and_not_read() {
    let d = tempdir("exclude");
    write(&d, "sel/keep.csv", b"a\n1\n");
    write(&d, "sel/private/secret.csv", b"a\n1\n2\n");
    let z = zip_bytes(&[("hr/pay.csv", b"a\n1\n"), ("eng/code.txt", b"x\n")]);
    write(&d, "sel/bundle.zip", &z);
    let opts = ScanOptions {
        exclude: vec!["sel/private".into(), "sel/bundle.zip/hr".into()],
        ..ScanOptions::default()
    };
    let inv = scan(&[d.join("sel")], opts);
    let secret = find(&inv, "secret.csv");
    assert_eq!(secret.status, Status::Excluded);
    assert_eq!(secret.expanded_bytes, 0);
    assert_eq!(find(&inv, "pay.csv").status, Status::Excluded);
    assert_eq!(find(&inv, "code.txt").status, Status::Assessed);
}

#[test]
fn encrypted_zip_members_are_locked() {
    // Minimal ZIP whose single entry has the "encrypted" general-purpose flag.
    let d = tempdir("locked");
    let mut z = zip_bytes(&[("secret.txt", b"hidden contents")]);
    // Set bit 0 of the flags in the local header (offset 6) and central
    // directory header (offset 8 after its signature).
    z[6] |= 1;
    let cd = z.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    z[cd + 8] |= 1;
    write(&d, "locked.zip", &z);
    let inv = scan(&[d.join("locked.zip")], ScanOptions::default());
    let s = find(&inv, "secret.txt");
    assert_eq!(s.status, Status::Locked);
    assert_eq!(s.reason, Some(Reason::PasswordProtected));
}

#[test]
fn corrupt_inputs_affect_only_themselves() {
    let d = tempdir("corrupt");
    write(&d, "in/broken.zip", b"PK\x03\x04 this is not really a zip");
    write(&d, "in/broken.json", b"{\"a\": [1, 2,");
    write(&d, "in/fine.csv", b"a\n1\n");
    let inv = scan(&[d.join("in")], ScanOptions::default());
    assert_eq!(find(&inv, "broken.zip").status, Status::StoppedSafely);
    assert_eq!(find(&inv, "broken.json").status, Status::StoppedSafely);
    assert_eq!(find(&inv, "fine.csv").status, Status::Assessed);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let d = tempdir("symlink");
    std::fs::create_dir_all(d.join("sel")).unwrap();
    write(&d, "outside.txt", b"should not be read\n");
    std::os::unix::fs::symlink(d.join("outside.txt"), d.join("sel/link.txt")).unwrap();
    let inv = scan(&[d.join("sel")], ScanOptions::default());
    let l = find(&inv, "link.txt");
    assert_eq!(l.reason, Some(Reason::SymlinkNotFollowed));
    assert_eq!(l.expanded_bytes, 0);
}

#[test]
fn stop_keeps_work_done_and_says_it_is_incomplete() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let d = tempdir("cancel");
    for i in 0..20 {
        write(&d, &format!("in/{i:02}.txt"), b"hello\n");
    }
    let flag = Arc::new(AtomicBool::new(false));
    let opts = ScanOptions {
        cancel: Some(flag.clone()),
        ..ScanOptions::default()
    };
    let mut seen = 0;
    let inv = scout_core::scan_with_progress(&[d.join("in")], opts, &mut |p| {
        seen = p.files_seen;
        if p.files_seen == 5 {
            flag.store(true, Ordering::Relaxed);
        }
    });
    assert!(inv.cancelled);
    assert!(
        inv.items.len() < 20 && inv.items.len() >= 4,
        "{}",
        inv.items.len()
    );
    assert!(seen >= 5);
    assert!(inv.limits_reached.contains(&Reason::StoppedByUser));
}
