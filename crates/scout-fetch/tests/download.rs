//! End-to-end download test against a local HTTP server. The test policy
//! allows plain HTTP to 127.0.0.1 only; the shipped app uses
//! `HostPolicy::slack()`.

use scout_fetch::{
    download, plan_from_export, FailReason, HostPolicy, Options, Outcome, MANIFEST_NAME,
};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;

fn serve() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) <= 2 {
                    break;
                }
            }
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap()
                .to_string();
            let (status, ctype, body, extra): (&str, &str, Vec<u8>, String) = match path.as_str() {
                "/ok/a.png" => ("200 OK", "image/png", vec![7u8; 1000], String::new()),
                "/ok/b.pdf" => (
                    "200 OK",
                    "application/pdf",
                    b"%PDF-1.7 hello".to_vec(),
                    String::new(),
                ),
                "/redir" => (
                    "302 Found",
                    "text/plain",
                    vec![],
                    "Location: /ok/b.pdf\r\n".into(),
                ),
                "/evil" => (
                    "302 Found",
                    "text/plain",
                    vec![],
                    "Location: https://evil.example/steal\r\n".into(),
                ),
                "/html" => (
                    "200 OK",
                    "text/html; charset=utf-8",
                    b"<html>Sign in</html>".to_vec(),
                    String::new(),
                ),
                "/forbidden" => ("403 Forbidden", "text/plain", b"no".to_vec(), String::new()),
                _ => (
                    "404 Not Found",
                    "text/plain",
                    b"gone".to_vec(),
                    String::new(),
                ),
            };
            let mut s = s;
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
            let _ = s.write_all(&body);
        }
    });
    port
}

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dps-fetch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn export(port: u16) -> Vec<u8> {
    let u = |p: &str| format!("http://127.0.0.1:{port}{p}?t=xoxe-CANARY-TOKEN");
    let day = json!([
        {"ts": "1.1", "user": "U1", "files": [
            {"id": "F0000000001", "name": "diagram.png", "mimetype": "image/png", "size": 1000, "url_private_download": u("/ok/a.png")},
            {"id": "F0000000002", "name": "../../spec.pdf", "mimetype": "application/pdf", "url_private_download": u("/redir")},
            {"id": "F0000000003", "name": "x.png", "mimetype": "image/png", "url_private_download": u("/evil")},
            {"id": "F0000000004", "name": "y.png", "mimetype": "image/png", "url_private_download": u("/html")},
            {"id": "F0000000005", "name": "z.png", "mimetype": "image/png", "url_private_download": u("/forbidden")},
            {"id": "F0000000006", "name": "w.png", "mimetype": "image/png", "url_private_download": u("/missing")},
            {"id": "F0000000007", "name": "d.png", "mimetype": "image/png", "url_private_download": "https://drive.google.com/file/d/1"},
            {"id": "F0000000008", "mode": "hidden_by_limit"},
            {"id": "F0000000009", "mode": "external", "url_private": "https://docs.google.com/x"},
            {"id": "../etc", "url_private_download": u("/ok/a.png")}
        ]},
        {"ts": "1.2", "user": "U2", "files": [{"id": "F0000000001", "name": "diagram.png", "url_private_download": u("/ok/a.png")}]}
    ]);
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("channels.json", o).unwrap();
        z.write_all(json!([{"name": "eng"}]).to_string().as_bytes())
            .unwrap();
        z.start_file("users.json", o).unwrap();
        z.write_all(b"[]").unwrap();
        z.start_file("eng/2021-01-01.json", o).unwrap();
        z.write_all(day.to_string().as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn downloads_only_what_slack_serves_and_matches_back() {
    let port = serve();
    let d = tempdir("e2e");
    let exp = d.join("Acme export.zip");
    std::fs::write(&exp, export(port)).unwrap();

    let plan = plan_from_export(&exp).unwrap();
    assert_eq!(
        plan.files.len(),
        7,
        "deduplicated, hidden/external/invalid skipped"
    );
    assert_eq!(
        (
            plan.skipped_hidden,
            plan.skipped_external,
            plan.skipped_unusable
        ),
        (1, 1, 1)
    );
    assert_eq!(plan.images(), 6);

    let out = d.join("Acme export attachments");
    let mut opts = Options::new(out.clone());
    opts.policy = HostPolicy::local_test("127.0.0.1");
    let mut last = None;
    let summary = download(&plan, &opts, &mut |p| last = Some(p.clone()));
    let p = summary.progress;
    assert_eq!(
        (p.total, p.finished, p.downloaded, p.failed),
        (7, 7, 2, 5),
        "{:?}",
        summary.failures
    );
    assert_eq!(p.bytes, 1000 + 14);
    assert_eq!(last.unwrap().finished, 7);
    let count = |r: FailReason| {
        summary
            .failures
            .iter()
            .find(|f| f.reason == r)
            .map_or(0, |f| f.files)
    };
    assert_eq!(
        count(FailReason::NotSlack),
        2,
        "redirect off-host and non-Slack link are refused"
    );
    assert_eq!(count(FailReason::NeedsSignIn), 2);
    assert_eq!(count(FailReason::Deleted), 1);

    assert_eq!(
        std::fs::read(out.join("attachments/F0000000001/diagram.png"))
            .unwrap()
            .len(),
        1000
    );
    assert!(
        out.join("attachments/F0000000002/spec.pdf").exists(),
        "traversal stripped from name"
    );
    assert!(!out.join("attachments/F0000000004").join("y.png").exists());
    let manifest = std::fs::read_to_string(out.join(MANIFEST_NAME)).unwrap();
    assert!(
        !manifest.contains("CANARY")
            && !manifest.contains("127.0.0.1")
            && !manifest.contains("http"),
        "{manifest}"
    );
    let parts: Vec<_> = walk(&out)
        .into_iter()
        .filter(|p| p.ends_with(".part"))
        .collect();
    assert!(parts.is_empty(), "{parts:?}");

    // Running again skips what is already there.
    let again = download(&plan, &opts, &mut |_| {});
    assert_eq!(again.progress.already_present, 2);
    assert_eq!(again.progress.downloaded, 0);

    // Working Minds credits the downloaded files to the export's references.
    let inv = scout_core::scan(
        &[exp.clone(), out.clone()],
        scout_core::ScanOptions::default(),
    );
    let map = scout_core::build_data_map(&inv);
    let img = map
        .relationships
        .iter()
        .find(|r| r.kind == "slack_images")
        .unwrap();
    assert_eq!((img.count, img.resolved), (6, 1));
    // The core also counts hidden, external and malformed records as references.
    let doc = map
        .relationships
        .iter()
        .find(|r| r.kind == "slack_files")
        .unwrap();
    assert_eq!(
        (doc.count, doc.resolved, doc.available_locally),
        (4, 1, false)
    );
    let _ = Outcome::AlreadyPresent;
}

#[test]
fn stop_is_prompt_and_leaves_no_partial_files() {
    let port = serve();
    let d = tempdir("stop");
    let exp = d.join("e.zip");
    std::fs::write(&exp, export(port)).unwrap();
    let plan = plan_from_export(&exp).unwrap();
    let out = d.join("out");
    let mut opts = Options::new(out.clone());
    opts.policy = HostPolicy::local_test("127.0.0.1");
    opts.cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let s = download(&plan, &opts, &mut |_| {});
    assert!(s.stopped);
    assert_eq!(s.progress.downloaded, 0);
    assert!(walk(&out).iter().all(|p| !p.ends_with(".part")));
}

#[test]
fn rejects_non_slack_exports() {
    let d = tempdir("notslack");
    std::fs::write(d.join("x.zip"), {
        let mut b = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut b);
            z.start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"hi").unwrap();
            z.finish().unwrap();
        }
        b.into_inner()
    })
    .unwrap();
    assert!(plan_from_export(&d.join("x.zip"))
        .unwrap_err()
        .contains("Slack workspace export"));
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p.display().to_string());
            }
        }
    }
    out
}
