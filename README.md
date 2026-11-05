# DataPack Scout

A downloadable desktop application that helps a company understand its
existing data exports and prepare a private assessment for possible AI
training-data licensing. Everything is assessed on the owner's computer. No
files or contents are sent anywhere.

This repository currently holds the **assessment core** (`crates/scout-core`)
and a **command-line driver** (`crates/scout-cli`) for the first delivery
milestone, *trustworthy local inventory*. The desktop shell, owner PDF and
encrypted private dossier come in later milestones (see below).

## Download

Preview builds for Windows, macOS and Linux are on the
[Releases page](https://github.com/tobyrowland/data-eval/releases). Unzip the
file and double-click `datapack-scout`. `QUICKSTART.txt` inside explains the
first-run security prompts, because the preview is not code-signed yet.

The `release` workflow builds and publishes them on every push to `main` or a
`v*` tag.

## Try it from source

```bash
cargo run --release --bin datapack-scout -- scan ~/Downloads/slack-export.zip ~/Exports/Drive --json data-map.json
```

`--exclude <selection>/<path>` leaves out a sub-folder or archive member. It
is counted as *Excluded* and its contents are not read.

## What milestone 1 does

- **Mixed inputs, nothing vanishes.** Every accessible item gets exactly one
  status: Assessed, Partly assessed, Inventoried, Locked, Unavailable,
  Excluded or Stopped safely. Each non-assessed item records a machine-readable
  reason.
- **Format detection by signature and structure**, with file names only as a
  fallback. Office/OpenDocument packages are one logical document, not a ZIP.
- **Archives expanded in memory, never extracted to disk.** ZIP/ZIP64, TAR,
  GZIP/TGZ and nesting up to three levels. 7z and RAR are inventoried.
  - Decompression-bomb protection uses actual expanded bytes, never header
    claims: a per-stream ratio guard that stays tripped, a global expanded-byte
    budget and a per-object cap.
  - Member-count, logical-file and record limits leave a usable inventory and
    say which limit was reached.
  - Path-traversal and absolute member names are flagged *Stopped safely*.
    Symlinks are never followed. Encrypted ZIP members are *Locked*.
- **Content parsers:** CSV/TSV (quote-aware record counts and ragged-row
  detection), JSON, JSON Lines, plain text, Markdown, HTML, XML, EML and MBOX
  (message counts). Exact duplicates are found with local SHA-256 hashes,
  which never leave the private inventory.
- **Slack export recogniser.** Message records split into participant, bot
  and administrative records, plus posting accounts, threads and replies with
  missing parents. It also counts file references, images, visibility,
  download links (counted, never fetched), Bitbucket/GitHub PR references and
  monthly activity.
- **Online-only placeholders** (Windows recall/offline attributes, macOS
  dataless files) are detected from metadata before any open, so the scan
  never triggers a cloud download.
- **Files changed during a scan** are marked rather than silently mixed in.
- **Your Data Map** (JSON and terminal text): template sentences tied to
  measured values, collections, coverage by status and reason, sizes,
  duplicates, limits reached and ranked preparation priorities. It contains no
  local paths, hashes, message text, channel names, account IDs or URLs. A
  canary test enforces this.

## Privacy guards in the build

`crates/scout-core/tests/no_network.rs` fails the build when:

- a networking crate (`reqwest`, `hyper`, `tokio`, `rustls`, …) enters
  `Cargo.lock`
- any source file uses socket APIs or spawns processes.

This is a build-level guard only. The spec also requires OS-level isolation
(macOS App Sandbox without network entitlements, Windows AppContainer) and
packet-capture tests on the packaged app.

## Tests

```bash
cargo test
```

The tests cover detection, the readers that enforce the limits, CSV and JSONL
edge cases, archive expansion and nesting, zip bombs, path traversal,
encrypted members, corrupt inputs, exclusions, the file and byte budgets, and
symlinks. A synthetic Slack export with seeded canaries stands in for the
private CRANQ reference fixture.

## Not yet built

| Area | Milestone |
|---|---|
| Tauri desktop shell (6 screens), persistent "Network access disabled" status backed by an isolation self-check | 1 |
| Checkpointed pause/resume, worker processes with memory/time limits, encrypted local project store | 1 |
| Signed and notarised Windows/macOS builds, packet-capture release tests | 1 |
| Slack workflow-step decoding (inline step exports, code-bearing steps) needed to reproduce every CRANQ reference value | 1–2 |
| PDF/DOCX/XLSX/PPTX text extraction, SQLite profiling, Git history, Jira/Linear/accounting connectors | 2 |
| Owner PDF (`Your-Data-Map-YYYY-MM-DD.pdf`) | 2 |
| Quality profiler, narrative dossier, disclosure manifest, `age` X25519 encryption to `Assessment-<id>.dpm.age` | 2 |
| Operator report reader | 2 |
| Calibration against analyst decisions | 3 |

Launch inputs still needed: the operator's legal entity, assessment email,
production recipient keys, and name/domain clearance.
