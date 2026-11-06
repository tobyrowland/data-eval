# DataPack Scout

A downloadable desktop application that helps a company understand its
existing data exports and prepare a private assessment for possible AI
training-data licensing. Everything is assessed on the owner's computer. No
files or contents are sent anywhere.

This repository holds:

- `crates/scout-app`: the **desktop app** (Tauri). You drag in your exports,
  watch the assessment run, and get a visual map of your data.
- `crates/scout-core`: the **assessment core** that does all the reading and
  counting.
- `crates/scout-cli`: a **command-line driver** for the same core.
- `crates/scout-fetch`: the optional **Attachment Downloader**, a separate
  companion app that fetches the files a Slack export links to, directly from
  Slack.

This is the first delivery milestone, *trustworthy local inventory*. The owner
PDF and the encrypted private dossier come in later milestones (see below).

## Download

Preview installers are on the
[Releases page](https://github.com/tobyrowland/data-eval/releases):

- Windows: `DataPack-Scout-Setup-windows-x64.exe` or `.msi`
- macOS: `DataPack-Scout-macos-universal.dmg`
- Linux: `.AppImage` or `.deb`

The command-line tool is in the `DataPack-Scout-CLI-*` archives. The preview is
not code-signed yet, so Windows and macOS show a first-run warning.

## The desktop app

1. **Add data.** Drop files, folders or ZIP exports anywhere on the window,
   or use *Choose files* or *Choose a folder*. Each selection shows its size
   and file count before anything is read.
2. **Assess.** Live counts of files, content examined and records, with the
   stage list and a *Stop* button that keeps the work done so far.
3. **Your data map.** You get:
   - a summary sentence built from measured values
   - a share-examined ring and KPI tiles
   - an interactive map: a "Your data" hub, one circle per collection with
     packed bubbles for formats or message types, and amber satellites for
     material your data refers to that wasn't included (shared images,
     documents, code repositories)
   - an activity timeline, a coverage breakdown, source cards and ranked next
     steps.

   *Save my data map* writes `.txt` and `.json` copies to
   `Documents/DataPack Scout`.

### Making the data more complete

The map's **Make your data more complete** card lists material your data
refers to but you haven't added, such as images and documents shared in Slack
or the repositories behind linked pull requests. Each item has a *How to get
these* guide.

**Adding data keeps your map.** You can drop more files or folders anywhere on
the map, or use *Add data*. Only the new material is read. It is merged into
the existing assessment (`scout_core::extend_scan`), and the map updates in
place with a banner showing what changed, such as "+479 files · 367 images
matched to conversations". If you closed the app, the opening screen offers
*Continue where you left off*. Only the list of selected paths is remembered,
in the app's private data folder.

Added files are matched back to the references. For Slack this is by file ID, using the downloader's
`attachments/<FILE_ID>/<name>` layout. The amber satellites fill teal and the
card shows "367 of 498 added".

## Attachment Downloader

A Slack export contains links to shared files, not the files themselves. The
Attachment Downloader reads the export and downloads those files using the
links already inside it, into `<export> attachments/attachments/<FILE_ID>/`.
It also writes a manifest and a README; neither contains any links.

- **The only DataPack Scout tool that connects to the internet.** It is a
  separate app, so DataPack Scout itself keeps no network capability.
- **Slack only.** Every request and every redirect must be HTTPS to a Slack
  file host (`slack.com`, `*.slack.com`, `*.slack-edge.com`). Anything else is
  refused. Links carry access tokens, so they are never logged, displayed in
  full or written to disk.
- **Handles failures:** expired or revoked links (an HTML sign-in page, 401 or
  403), deleted files (404 or 410), files stored outside Slack, and files
  hidden by a plan limit.
- **Practical:** it resumes by skipping files already saved, has a Stop
  button, caps each file at 4 GB and runs 4 parallel downloads.
- **Tests:** `crates/scout-fetch/tests/download.rs` runs the whole flow against
  a local server. It checks that DataPack Scout credits the downloaded files
  back to the export.

The window can only call the app's own commands. No HTTP, shell or filesystem
plugins are granted, and a strict content security policy blocks every remote
origin. `scripts/check-desktop-deps.sh` fails CI if an HTTP client, a TLS
stack, a socket library or tokio's `net` feature enters the desktop build for
any shipped target.

To develop the app:

```bash
cd crates/scout-app && npx @tauri-apps/cli@2.12.4 dev
```

On Linux, install the webview libraries first: `libwebkit2gtk-4.1-dev` and
`libgtk-3-dev`.

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
- **Language mix.** The apparent language of readable text is measured in
  words. That covers Slack messages written by people (bot and integration
  messages are left out), plus text, Markdown, HTML and email bodies (up to
  256 KB sampled per document). The result is a table in the app and the CLI,
  with a breakdown per collection.
  - **Method:** `whatlang`'s trigram detector, run locally. Paragraphs of 60+
    letters take the top language; 20–59 letters need a confident result;
    anything shorter is reported as "too short to tell", not guessed. Channel
    names are never used as language labels.
  - **Not yet read:** PDF and Office documents, which are listed as not yet
    readable.
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
| Desktop app: privacy review and report-saved screens, a verified "Network access disabled" status backed by an OS isolation self-check | 1 |
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
