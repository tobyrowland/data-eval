mod common;
use common::*;
use scout_core::{package, scan, ScanOptions};
use serde_json::json;

#[test]
fn builds_a_pack_with_canonical_folders() {
    let d = tempdir("pack");
    let src = d.join("src");
    let day = json!([{"ts": "1.1", "user": "U1", "text": "x",
        "files": [{"id": "F04AAAAAAA1", "mimetype": "image/png", "url_private_download": "https://files.slack.com/a"}]}]);
    write(
        &src,
        "export.zip",
        &zip_bytes(&[
            ("channels.json", b"[{\"name\":\"eng\"}]"),
            ("users.json", b"[]"),
            ("eng/2021-01-01.json", day.to_string().as_bytes()),
        ]),
    );
    write(
        &src,
        "export attachments/attachments/F04AAAAAAA1/a.png",
        b"\x89PNG\r\n\x1a\nx",
    );
    write(
        &src,
        "takeout-20261106T153241Z-1-001.zip",
        &zip_bytes(&[
            ("Takeout/archive_browser.html", b"<html></html>"),
            ("Takeout/Drive/Plans/Roadmap.md", b"# Roadmap\n"),
            ("Takeout/Drive/../../evil.txt", b"should never be written"),
        ]),
    );
    write(
        &src,
        "takeout-20261106T153241Z-2-001.zip",
        &zip_bytes(&[
            ("Takeout/Drive/Finance/budget.csv", b"m,a\n1,2\n"),
            (
                "Takeout/Mail/All mail.mbox",
                b"From a@b Mon\nFrom: a\nSubject: x\n\nhi\n",
            ),
        ]),
    );
    write(
        &src,
        "app/.git/config",
        b"[remote \"origin\"]\n\turl = git@github.com:acme/app.git\n",
    );
    write(&src, "app/src/main.rs", b"fn main() {}\n");
    write(
        &src,
        "mail.mbox",
        b"From a@b Mon\nFrom: a\nSubject: y\n\nhello\n",
    );
    write(&src, "docs/notes.md", b"# Notes\n");
    write(&src, "docs/private/secret.txt", b"keep out\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(src.join("mail.mbox"), src.join("docs/link.mbox")).unwrap();

    let inputs: Vec<_> = [
        "export.zip",
        "export attachments",
        "takeout-20261106T153241Z-1-001.zip",
        "takeout-20261106T153241Z-2-001.zip",
        "app",
        "mail.mbox",
        "docs",
    ]
    .iter()
    .map(|n| src.join(n))
    .collect();
    let opts = ScanOptions {
        exclude: vec!["docs/private".into()],
        ..ScanOptions::default()
    };
    let inv = scan(&inputs, opts);
    let plan = package::plan(&inv);
    let kinds: Vec<_> = plan
        .entries
        .iter()
        .map(|e| (e.kind, e.folders.join("+")))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("Slack export", "Slack".to_string()),
            ("Slack attachments", "Slack/Attachments".to_string()),
            ("Google Takeout", "Google Drive".to_string()),
            ("Google Takeout", "Google Drive+Google Mail".to_string()),
            ("Git repository", "Code/acme-app".to_string()),
            ("Email", "Email".to_string()),
            ("Folder", "Documents".to_string()),
        ]
    );
    assert_eq!(plan.excluded_files, 1);

    let out = d.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut ticks = 0;
    let s = package::build(
        &inv,
        &plan,
        &out,
        "Working Minds data pack",
        None,
        &mut |_| ticks += 1,
    );
    assert!(s.error.is_none(), "{:?}", s.error);
    assert!(!s.stopped && ticks > 0);
    assert_eq!(s.skipped_unsafe, 1);
    let pack = std::path::PathBuf::from(&s.folder);
    for p in [
        "Slack/export.zip",
        "Slack/Attachments/attachments/F04AAAAAAA1/a.png",
        "Google Drive/Plans/Roadmap.md",
        "Google Drive/Finance/budget.csv",
        "Google Mail/All mail.mbox",
        "Code/acme-app/.git/config",
        "Code/acme-app/src/main.rs",
        "Email/mail.mbox",
        "Documents/docs/notes.md",
        package::README_NAME,
        package::MANIFEST_NAME,
    ] {
        assert!(pack.join(p).is_file(), "missing {p}");
    }
    assert!(
        !pack.join("Documents/docs/private/secret.txt").exists(),
        "excluded file was copied"
    );
    assert!(
        !pack.join("Documents/docs/link.mbox").exists(),
        "symlink was followed"
    );
    assert!(!pack.join("Google Drive/archive_browser.html").exists());
    assert!(
        !out.join("evil.txt").exists()
            && !d.join("evil.txt").exists()
            && !pack.join("evil.txt").exists()
    );
    let readme = std::fs::read_to_string(pack.join(package::README_NAME)).unwrap();
    assert!(readme.contains("Google Drive/") && readme.contains("actual data"));
    // A second pack does not overwrite the first.
    let s2 = package::build(
        &inv,
        &plan,
        &out,
        "Working Minds data pack",
        None,
        &mut |_| {},
    );
    assert_ne!(s2.folder, s.folder);
}

#[test]
fn stop_leaves_an_incomplete_pack_that_says_so() {
    let d = tempdir("pack-stop");
    for i in 0..20 {
        write(&d, &format!("docs/{i}.txt"), b"hello\n");
    }
    let inv = scan(&[d.join("docs")], ScanOptions::default());
    let plan = package::plan(&inv);
    let flag = std::sync::atomic::AtomicBool::new(true);
    let s = package::build(&inv, &plan, &d, "pack", Some(&flag), &mut |_| {});
    assert!(s.stopped && s.files_copied == 0);
    let readme =
        std::fs::read_to_string(std::path::Path::new(&s.folder).join(package::README_NAME))
            .unwrap();
    assert!(readme.contains("incomplete"));
}
