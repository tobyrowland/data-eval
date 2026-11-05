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

/// Walks Cargo.lock from the assessment crates. The desktop shell (Tauri)
/// is checked per target in CI by `scripts/check-desktop-deps.sh`, because
/// the lockfile also lists mobile-only dependencies.
#[test]
fn assessment_crates_have_no_networking_dependencies() {
    use std::collections::{HashMap, HashSet};
    let lock =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .expect("Cargo.lock is committed");
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut name = String::new();
        let mut list = Vec::new();
        let mut in_deps = false;
        for line in block.lines() {
            let line = line.trim();
            if let Some(n) = line
                .strip_prefix("name = \"")
                .and_then(|l| l.strip_suffix('"'))
            {
                name = n.to_string();
            } else if line.starts_with("dependencies = [") {
                in_deps = true;
            } else if in_deps && line.starts_with(']') {
                in_deps = false;
            } else if in_deps {
                let d = line.trim_matches(|c| c == '"' || c == ',');
                list.push(d.split(' ').next().unwrap_or(d).to_string());
            }
        }
        deps.entry(name).or_default().extend(list);
    }
    let mut seen = HashSet::new();
    let mut stack = vec!["scout-core".to_string(), "scout-cli".to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        assert!(
            !FORBIDDEN_CRATES.contains(&n.as_str()),
            "networking crate reachable from the assessment core: {n}"
        );
        stack.extend(deps.get(&n).cloned().unwrap_or_default());
    }
    assert!(
        seen.len() > 5,
        "dependency walk found too little; lockfile format changed?"
    );
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
                // The Attachment Downloader (scout-fetch) is the one crate whose
                // purpose is downloading, from Slack only. It is a separate app;
                // scripts/check-desktop-deps.sh keeps it out of DataPack Scout.
                if p.file_name()
                    .is_some_and(|n| n != "target" && n != "scout-fetch")
                {
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
