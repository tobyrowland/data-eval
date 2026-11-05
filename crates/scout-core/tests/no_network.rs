//! Release guard: the assessment binaries must have no network capability.
//! This is a build-level check; OS-level isolation (App Sandbox,
//! AppContainer) and packet-capture tests are required on packaged builds.

use std::path::Path;

const FORBIDDEN_CRATES: &[&str] = &[
    "reqwest",
    "hyper",
    "ureq",
    "curl",
    "isahc",
    "surf",
    "h2",
    "h3",
    "quinn",
    "tokio",
    "mio",
    "socket2",
    "native-tls",
    "rustls",
    "openssl",
    "tungstenite",
    "trust-dns-resolver",
    "hickory-resolver",
    "attohttpc",
];

#[test]
fn lockfile_has_no_networking_crates() {
    let lock =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .expect("Cargo.lock is committed");
    for line in lock.lines() {
        if let Some(name) = line
            .strip_prefix("name = \"")
            .and_then(|l| l.strip_suffix('"'))
        {
            assert!(
                !FORBIDDEN_CRATES.contains(&name),
                "networking crate in dependency graph: {name}"
            );
        }
    }
}

#[test]
fn sources_use_no_network_apis() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut stack = vec![root.join("crates")];
    let needles = [
        "std::net",
        "TcpStream",
        "UdpSocket",
        "TcpListener",
        "ToSocketAddrs",
        "std::process::Command",
    ];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "target") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs")
                && !p.ends_with("tests/no_network.rs")
            {
                let src = std::fs::read_to_string(&p).unwrap();
                for n in needles {
                    assert!(!src.contains(n), "{} uses {n}", p.display());
                }
            }
        }
    }
}
