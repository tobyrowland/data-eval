# Working Minds

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

- Windows 10/11 (64-bit): `Working-Minds-Setup-windows-x64.exe`
- Optional: `Working-Minds-Attachment-Downloader-Setup-windows-x64.exe`

Only Windows x64 is built for now, and CI tests on Windows only. The macOS
and Linux builds, the `.msi` and the command-line tool download are paused
and will return once Windows is done (see `.github/workflows/`). The command-line
tool still builds from source with `cargo build --release --bin working-minds`.
The preview is not code-signed yet, so Windows shows a first-run warning.

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
   `Documents/Working Minds`.

### Page layout

1. **The data map** comes first: a one-line summary and four key figures
   above it.
2. **Your platforms** shows what has been collected and what is still
   missing, one card per platform: Slack, Email, Google Drive, GitHub,
   GitLab, Bitbucket, and a final "Other files" card. Each card has:
   - a status (Collected / Partly collected) and totals: items, files, size,
     dates and messages (each message counted once)
   - the next action where one applies, for example "6,123 shared files
     still missing", "2 other linked repositories not added", or "added as a
     download, no history"
   - a table of the **items in the collection**: name, files, size, dates,
     messages, records or emails, and **overlap** with other items, such as
     "Duplicate of …", "88% also in …" or "40 emails also in …"
   - for each row, *Remove* (a highlighted *Remove duplicate* when another
     item already holds everything in it), and the item's file tree, where
     single files and folders can be removed and put back
   - "Linked from N other items", showing which files link to the platform

   Platforms your data links to or mentions but you haven't collected are
   listed under **Not collected yet**, with the evidence ("507 links to 297
   tickets · app installed in Slack"), *How to export*, *Mark as done* and
   *Remove*. Removing never touches files on disk, can be undone, and is
   remembered with the session. File names and paths appear only on this
   screen and never go into the saved map.

   **Duplicates are never counted twice.** The headline figures count
   files, records, messages and emails once:
   - identical copies of files are matched by content
   - a Slack message in two exports is matched by channel and timestamp
   - an email in two mailboxes (for example a Takeout "All mail" and a
     separate .mbox) is matched by Message-ID

   An item counts as a duplicate only when everything in it, emails and
   other files alike, is also in another item.

   **Data that doesn't seem to belong is flagged.** Each item is compared
   with the rest of the map:
   - people, email addresses, email and web domains, code-host owners and
     Microsoft tenants it has in common with other items
   - distinctive names in common: capitalised project, product and client
     names and long acronyms; common words, months and tool names don't
     count
   - direct ties: shared messages or files, matched attachments, a
     repository the other data links to
   - whether its dates overlap the other items' dates

   Public services such as gmail.com, github.com and teams.microsoft.com
   never count as a connection. An item that names its own people,
   addresses or organisation, none of which appear in the rest of the data,
   is flagged even if a few names or its dates happen to coincide. Names
   only count when they are names: words also used in lower case ("Canvas",
   "release") and common first names are ignored. A Discord server or Teams
   tenant that the rest of the data links to (`discord.com/channels/<id>`,
   `tenantId=` in a Teams link) counts as a direct connection. An item with nothing in common gets an amber
   warning at the top of the platforms section and a "Not linked to your
   other data" badge on its row, for example "Shares no people, email
   addresses, domains or names with your other data, and its dates (2026)
   don't overlap theirs". The warning offers *Show*, *Keep it* (remembered)
   and *Remove*. The data-pack panel repeats it. Only counts of what is
   shared reach the data map; the fingerprints are hashed and stay on the
   computer.

   **Microsoft Teams** exports get their own platform card:
   - **read in full:** Microsoft Graph JSON (Teams Export API, eDiscovery
     tools, tenant takeouts with `teams/<id>/messages_*.json` and
     `users/<id>/messages_*.json`), and the personal "Export your data"
     `messages.json`
   - **counted:** messages, teams, channels, chats, people and meetings
   - **counted once:** a chat stored in both participants' files, or in two
     exports, counts once
   - **PST files** from a Purview eDiscovery export are counted but not read
     yet

   **Discord** exports get their own card too:
   - Discord's own **"Request your data" package**: `Account/user.json`,
     `Messages/c<id>/channel.json` with `messages.json` (or `messages.csv`
     in older packages), and `Servers/index.json`
   - **DiscordChatExporter** channel exports (JSON) for whole servers
   - counts messages (each once, even if a package is added twice),
     servers, channels, DMs, group chats and linked attachments
   - a personal package holds only the messages written by the account that
     requested it, and the card says so

   **Groups of unrelated items are flagged too.** Items are grouped by what
   connects them, pair by pair, and the largest group is taken as the main
   data. Two exports from another company that only connect to each other
   (for example a Teams and a Discord export from the same team) are both
   flagged "Only connected to …". A group is only flagged when it shows
   whose data it is (people, addresses, domains or distinctive names), not
   when it is merely thin.
3. **Analysis:** collections, coverage, file formats and languages tables,
   plus activity over time.

### Create data pack

*Create data pack* copies everything you added into one new folder with
standard names, ready to put on a drive:

| Source | Folder in the pack |
|---|---|
| Slack exports | `Slack/` |
| Downloaded Slack attachments | `Slack/Attachments/` |
| Google Takeout (all parts unpacked and merged by product) | `Google Drive/`, `Google Mail/`, … |
| Mailboxes | `Email/` |
| Git repositories, with history | `Code/<owner-repo>/` |
| Everything else | `Documents/` or `Other/` |

- **Originals are never changed.** Anything you excluded is left out.
- **Safe copying:** symbolic links aren't followed, and archive entries that
  try to escape their folder are skipped and counted.
- **What's included:** a `README.txt` describing the layout (and that the
  folder holds real data) and a JSON manifest (counts only).
- **The CLI does the same:**

  ```bash
  working-minds pack <inputs> --out <folder>
  ```

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

- **The only Working Minds tool that connects to the internet.** It is a
  separate app, so Working Minds itself keeps no network capability.
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
  a local server. It checks that Working Minds credits the downloaded files
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
cargo run --release --bin working-minds -- scan ~/Downloads/slack-export.zip ~/Exports/Drive --json data-map.json
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
  - **Prose only:** code and data files (`.js`, `.json`, `.csv`, `.log` and
    so on) are skipped. Passages that read like code, logs or "lorem ipsum"
    placeholder text are set aside and reported separately, so they are never
    misread as Latin, Danish or Catalan. Slack code blocks and `:emoji:` codes
    are removed first.
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
