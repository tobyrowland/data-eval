//! `working-minds` — command-line driver for the local assessment core.
//!
//! Usage:
//!   working-minds scan <path>... [--exclude <path>]... [--json <file>]
//!
//! Prints the owner's data map to the terminal and optionally writes it as
//! JSON. Nothing is sent anywhere; the binary has no network capability.

use scout_core::report::render_text;
use scout_core::{build_data_map, scan, ScanOptions};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Working Minds — understand your company data, privately.

Usage:
  working-minds scan <file-or-folder>... [--exclude <path>]... [--json <out.json>]
  working-minds pack <file-or-folder>... --out <folder> [--exclude <path>]...

  pack copies everything into one new folder with standard names (Slack/,
  Google Drive/, Google Mail/, Email/, Code/<repo>/, Documents/, Other/),
  unpacking and merging Google Takeout parts. Originals are not changed.

Options:
  --exclude <path>   Leave out a path, relative to a selection (e.g. export.zip/random)
  --json <file>      Also save the data map as JSON
  --out <folder>     Where pack creates the data pack folder
  -h, --help         Show this help

Assessment happens on this computer. No data is sent to us or to an AI service.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        // Double-clicked: guide the user instead of flashing a help screen.
        return interactive();
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let packing = args[0] == "pack";
    if args[0] != "scan" && !packing {
        eprintln!("Unknown command '{}'.\n\n{USAGE}", args[0]);
        return ExitCode::from(2);
    }
    let mut inputs = Vec::new();
    let mut opts = ScanOptions::default();
    let mut json_out: Option<PathBuf> = None;
    let mut pack_out: Option<PathBuf> = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--exclude" => match it.next() {
                Some(v) => opts.exclude.push(v.clone()),
                None => return usage_error("--exclude needs a path"),
            },
            "--out" => match it.next() {
                Some(v) => pack_out = Some(PathBuf::from(v)),
                None => return usage_error("--out needs a folder"),
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
    if packing {
        let Some(out) = pack_out else {
            return usage_error("pack needs --out <folder>");
        };
        if let Err(e) = std::fs::create_dir_all(&out) {
            eprintln!("Could not create {}: {e}", out.display());
            return ExitCode::FAILURE;
        }
        let plan = scout_core::package::plan(&inventory);
        for e in &plan.entries {
            println!("  {:<28} → {}", e.collection, e.folders.join(", "));
        }
        let s = scout_core::package::build(
            &inventory,
            &plan,
            &out,
            "Working Minds data pack",
            None,
            &mut |_| {},
        );
        match &s.error {
            None => println!("Created {} ({} files).", s.folder, s.files_copied),
            Some(e) => {
                eprintln!("Copying stopped: {e}. Partial pack at {}", s.folder);
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }
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

/// Guided mode for people who open the app by double-clicking it.
fn interactive() -> ExitCode {
    println!("Working Minds — understand your company data, privately.");
    println!("Assessment happens on this computer. No data is sent to us or to an AI service.\n");
    println!("Drag a file, folder or ZIP export into this window and press Enter.");
    println!("Add as many as you like. Press Enter on an empty line to start.\n");
    let stdin = io::stdin();
    let mut inputs: Vec<PathBuf> = Vec::new();
    loop {
        print!(
            "{} ",
            if inputs.is_empty() {
                "Add >"
            } else {
                "Add more, or press Enter to start >"
            }
        );
        let _ = io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            if inputs.is_empty() {
                continue;
            }
            break;
        }
        for p in parse_dropped(line) {
            if p.exists() {
                println!("  Added {}", p.display());
                inputs.push(p);
            } else {
                println!("  Could not find {}", p.display());
            }
        }
    }
    if inputs.is_empty() {
        return ExitCode::SUCCESS;
    }
    println!("\nWorking on your computer. Nothing is being uploaded.\n");
    let inventory = scan(&inputs, ScanOptions::default());
    let map = build_data_map(&inventory);
    let text = render_text(&map);
    println!("{text}");

    let dir = output_dir();
    let stem = format!("Your-Data-Map-{}", today());
    let saved = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::write(dir.join(format!("{stem}.txt")), &text)?;
        let json = serde_json::to_string_pretty(&map).expect("data map serialises");
        std::fs::write(dir.join(format!("{stem}.json")), json)
    });
    match saved {
        Ok(()) => println!("Saved your data map in {}", dir.display()),
        Err(e) => println!("Could not save the data map in {}: {e}", dir.display()),
    }
    print!("\nPress Enter to close.");
    let _ = io::stdout().flush();
    let _ = stdin.lock().read_line(&mut String::new());
    ExitCode::SUCCESS
}

/// Splits a dropped line into paths. Terminals quote paths with spaces
/// ('...' or "...") or escape them with backslashes (macOS).
fn parse_dropped(line: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '\\') if !cfg!(windows) && chars.peek().is_some() => {
                cur.push(chars.next().unwrap())
            }
            (None, ' ') => {
                if !cur.is_empty() {
                    out.push(PathBuf::from(std::mem::take(&mut cur)));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(PathBuf::from(cur));
    }
    // A single unquoted Windows path containing spaces arrives unsplit only if
    // it existed as a whole; prefer the whole line when it is a real path.
    let whole = PathBuf::from(line.trim_matches(|c| c == '"' || c == '\''));
    if out.len() > 1 && whole.exists() {
        return vec![whole];
    }
    out
}

fn output_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let docs = home.join("Documents");
    if docs.is_dir() { docs } else { home }.join("Working Minds")
}

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = scout_core::analyze::slack::civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("{msg}\n\n{USAGE}");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::parse_dropped;
    use std::path::PathBuf;

    #[test]
    fn parses_quoted_and_escaped_paths() {
        assert_eq!(
            parse_dropped("'/a b/c.zip' /d"),
            vec![PathBuf::from("/a b/c.zip"), PathBuf::from("/d")]
        );
        assert_eq!(
            parse_dropped("\"C:/My Files/x.zip\""),
            vec![PathBuf::from("C:/My Files/x.zip")]
        );
        if !cfg!(windows) {
            assert_eq!(parse_dropped("/a\\ b/c"), vec![PathBuf::from("/a b/c")]);
        }
    }
}
