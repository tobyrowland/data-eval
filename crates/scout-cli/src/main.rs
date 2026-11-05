//! `datapack-scout` — command-line driver for the local assessment core.
//!
//! Usage:
//!   datapack-scout scan <path>... [--exclude <path>]... [--json <file>]
//!
//! Prints the owner's data map to the terminal and optionally writes it as
//! JSON. Nothing is sent anywhere; the binary has no network capability.

use scout_core::report::render_text;
use scout_core::{build_data_map, scan, ScanOptions};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
DataPack Scout — understand your company data, privately.

Usage:
  datapack-scout scan <file-or-folder>... [--exclude <path>]... [--json <out.json>]

Options:
  --exclude <path>   Leave out a path, relative to a selection (e.g. export.zip/random)
  --json <file>      Also save the data map as JSON
  -h, --help         Show this help

Assessment happens on this computer. No data is sent to us or to an AI service.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args[0] != "scan" {
        eprintln!("Unknown command '{}'.\n\n{USAGE}", args[0]);
        return ExitCode::from(2);
    }
    let mut inputs = Vec::new();
    let mut opts = ScanOptions::default();
    let mut json_out: Option<PathBuf> = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--exclude" => match it.next() {
                Some(v) => opts.exclude.push(v.clone()),
                None => return usage_error("--exclude needs a path"),
            },
            "--json" => match it.next() {
                Some(v) => json_out = Some(PathBuf::from(v)),
                None => return usage_error("--json needs a file name"),
            },
            flag if flag.starts_with("--") => {
                return usage_error(&format!("unknown option {flag}"))
            }
            path => inputs.push(PathBuf::from(path)),
        }
    }
    if inputs.is_empty() {
        return usage_error("choose at least one file or folder");
    }
    for p in &inputs {
        if !p.exists() {
            return usage_error(&format!("{} was not found", p.display()));
        }
    }

    eprintln!("Working on your computer. Nothing is being uploaded.");
    let inventory = scan(&inputs, opts);
    let map = build_data_map(&inventory);
    print!("{}", render_text(&map));

    if let Some(out) = json_out {
        let json = serde_json::to_string_pretty(&map).expect("data map serialises");
        if let Err(e) = std::fs::write(&out, json) {
            eprintln!("Could not save {}: {e}", out.display());
            return ExitCode::FAILURE;
        }
        eprintln!("Saved your data map to {}", out.display());
    }
    ExitCode::SUCCESS
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("{msg}\n\n{USAGE}");
    ExitCode::from(2)
}
