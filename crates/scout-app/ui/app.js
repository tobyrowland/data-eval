/* Working Minds — screen flow and wiring to the Rust core.
 * Talks only to the app's own commands through Tauri IPC. */
(function () {
  "use strict";
  const { bytes, num, esc } = window.ScoutMap;
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args);
  const listen = (ev, fn) => T.event.listen(ev, fn);
  const $ = (id) => document.getElementById(id);

  let selections = [];
  let assessing = false;
  let timer = null;
  let lastSaved = null;
  let currentMap = null;

  // ---------------------------------------------------------------- screens
  function show(id) {
    document.querySelectorAll(".screen").forEach((s) => s.toggleAttribute("data-active", s.id === id));
    window.scrollTo({ top: 0 });
  }

  // ---------------------------------------------------------------- add
  const ICONS = {
    folder: '<svg class="i" viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/></svg>',
    archive: '<svg class="i" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9M10 13h4"/></svg>',
    file: '<svg class="i" viewBox="0 0 24 24"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z"/><path d="M14 3v5h5"/></svg>',
  };

  const onMap = () => $("screen-map").hasAttribute("data-active");

  async function addPaths(paths) {
    if (!paths || !paths.length || assessing) return;
    if (onMap() && currentMap) return addToMap(paths);
    const known = new Set(selections.map((s) => s.path));
    const fresh = paths.filter((p) => !known.has(p));
    if (!fresh.length) { toast("Already added."); return; }
    const described = await invoke("describe_paths", { paths: fresh });
    selections.push(...described);
    renderSelection();
    if (!$("screen-add").hasAttribute("data-active")) show("screen-add");
  }

  function renderSelection() {
    const box = $("selection");
    box.hidden = selections.length === 0;
    $("selection-list").innerHTML = selections.map((s, i) => `
      <li class="sel-item">
        <div class="sel-icon ${s.kind}">${ICONS[s.kind] || ICONS.file}</div>
        <div style="min-width:0"><div class="sel-name">${esc(s.name)}</div><div class="sel-path" title="${esc(s.path)}">&lrm;${esc(s.path)}</div></div>
        <div class="sel-meta">${s.kind === "folder" ? `${num(s.files)}${s.approximate ? "+" : ""} files · ` : s.kind === "archive" ? "Archive · " : ""}${bytes(s.bytes)}${s.approximate ? "+" : ""}</div>
        <button class="sel-remove" data-i="${i}" aria-label="Remove ${esc(s.name)}">Remove</button>
      </li>`).join("");
    const total = selections.reduce((a, s) => a + s.bytes, 0);
    const files = selections.reduce((a, s) => a + s.files, 0);
    $("selection-total").textContent = `${selections.length} ${selections.length === 1 ? "item" : "items"} · ${num(files)} files · ${bytes(total)}`;
  }

  $("selection-list").addEventListener("click", (e) => {
    const b = e.target.closest(".sel-remove");
    if (!b) return;
    selections.splice(+b.dataset.i, 1);
    renderSelection();
  });
  $("clear-all").onclick = () => { selections = []; renderSelection(); };
  $("pick-files").onclick = async () => addPaths(await invoke("pick_files"));
  $("pick-folder").onclick = async () => addPaths(await invoke("pick_folder"));
  $("dropzone").addEventListener("keydown", async (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); addPaths(await invoke("pick_files")); } });

  // native drag and drop (paths arrive from the Rust side)
  listen("scout://drag", (e) => {
    const over = e.payload === "over" && !assessing;
    document.body.classList.toggle("dragging", over);
    $("veil-title").textContent = onMap() ? "Drop to add to your map" : "Drop to add";
    $("veil-sub").textContent = onMap() ? "Only the new files are read; your map updates in place" : "Files, folders and ZIP exports";
  });
  listen("scout://drop", (e) => { document.body.classList.remove("dragging"); addPaths(e.payload); });
  // keep the webview from navigating to a dropped file
  window.addEventListener("dragover", (e) => e.preventDefault());
  window.addEventListener("drop", (e) => e.preventDefault());

  // ---------------------------------------------------------------- assess
  function setStage(n) {
    document.querySelectorAll("#stages li").forEach((li) => {
      const k = +li.dataset.stage;
      li.classList.toggle("done", k < n);
      li.classList.toggle("active", k === n);
    });
  }

  $("start").onclick = () => startAssess();
  // Exclusions to reopen the last session with; null for a fresh start.
  let resumeExclude = null;
  async function startAssess() {
    if (!selections.length) return;
    assessing = true;
    show("screen-assess");
    $("stop").textContent = "Stop"; $("stop").disabled = false;
    ["c-files", "c-records"].forEach((id) => ($(id).textContent = "0"));
    $("c-bytes").textContent = "0 B";
    $("assess-current").textContent = "Finding files and exports…";
    setStage(0);
    const started = Date.now();
    timer = setInterval(() => {
      const s = Math.floor((Date.now() - started) / 1000);
      $("c-time").textContent = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
    }, 250);
    try {
      const map = await invoke("assess", { paths: selections.map((s) => s.path), exclude: resumeExclude });
      resumeExclude = null;
      setStage(4);
      await new Promise((r) => setTimeout(r, 450));
      setStage(5);
      await new Promise((r) => setTimeout(r, 250));
      showMap(map);
    } catch (err) {
      toast("The assessment could not finish: " + err);
      show("screen-add");
    } finally {
      clearInterval(timer);
      assessing = false;
    }
  }

  // ------------------------------------------------- add data to the map
  function names(paths) {
    const n = paths.map((p) => p.split(/[\\/]/).filter(Boolean).pop());
    return n.length === 1 ? n[0] : `${n.length} items`;
  }

  async function addToMap(paths) {
    if (assessing) return;
    const known = new Set(selections.map((s) => s.path));
    const fresh = paths.filter((p) => !known.has(p));
    if (!fresh.length) { toast("That is already part of your map."); return; }
    assessing = true;
    const el = $("adding");
    $("adding-title").textContent = `Adding ${names(fresh)}`;
    $("adding-detail").textContent = "Reading on this computer…";
    $("adding-stop").disabled = false; $("adding-stop").textContent = "Stop";
    el.hidden = false;
    requestAnimationFrame(() => el.classList.add("on"));
    const before = currentMap;
    try {
      const described = await invoke("describe_paths", { paths: fresh });
      const map = await invoke("assess_more", { paths: fresh });
      selections.push(...described);
      renderSelection();
      showMap(map, diffBanner(before, map, fresh));
    } catch (err) {
      toast("Could not add that: " + err);
    } finally {
      el.classList.remove("on");
      setTimeout(() => (el.hidden = true), 250);
      assessing = false;
    }
  }
  // ------------------------------------- remove or restore items and files
  // Changes only this assessment and the data pack; nothing on disk changes.
  async function applyChange(title, run, after) {
    if (assessing) return;
    assessing = true;
    const el = $("adding");
    $("adding-title").textContent = title;
    $("adding-detail").textContent = "Updating your map on this computer…";
    $("adding-stop").disabled = false; $("adding-stop").textContent = "Stop";
    el.hidden = false;
    requestAnimationFrame(() => el.classList.add("on"));
    try {
      const map = await run();
      if (after) await after();
      showMap(map, `<div class="update-banner reveal"><div class="update-icon" aria-hidden="true">✓</div><div><b>${esc(title.replace(/…$/, ""))}</b><span>Your map, checklist and data pack are up to date. Nothing on disk was changed.</span></div></div>`);
    } catch (err) {
      toast("Could not change that: " + err);
    } finally {
      el.classList.remove("on");
      setTimeout(() => (el.hidden = true), 250);
      assessing = false;
    }
  }
  const fileHandlers = {
    list: (label, prefix, service) => invoke("list_files", { label, prefix: prefix || null, service: service || null }),
    linking: (label, service) => invoke("linking_files", { label, service }),
    removeItem: (label, display) => applyChange(`Removing ${display}…`, () => invoke("remove_items", { labels: [label] }), async () => {
      const removed = await invoke("removed_items").catch(() => []);
      const gone = new Set(removed.map((r) => r.path));
      selections = selections.filter((s) => !gone.has(s.path));
      renderSelection();
    }),
    restoreItem: (path, name) => applyChange(`Putting back ${name}…`, () => invoke("restore_item", { path }), async () => {
      if (!selections.some((s) => s.path === path)) selections.push(...(await invoke("describe_paths", { paths: [path] })));
      renderSelection();
    }),
    exclude: (path, name) => applyChange(`Leaving out ${name}…`, () => invoke("exclude_paths", { paths: [path] })),
    restore: (path, name) => applyChange(`Putting back ${name}…`, () => invoke("restore_paths", { paths: [path] })),
  };

  $("adding-stop").onclick = async () => { $("adding-stop").disabled = true; $("adding-stop").textContent = "Stopping…"; await invoke("stop"); };

  /** A short, measured summary of what an addition changed. */
  function diffBanner(a, b, paths) {
    if (!a) return "";
    const files = b.totals.logical_files - a.totals.logical_files;
    const parts = [];
    if (files > 0) parts.push(`+${num(files)} ${files === 1 ? "file" : "files"}`);
    for (const r of b.relationships || []) {
      const old = (a.relationships || []).find((x) => x.from === r.from && x.kind === r.kind);
      const gained = (r.resolved || 0) - ((old && old.resolved) || 0);
      if (gained > 0) {
        const what = r.kind === "slack_images" ? (gained === 1 ? "image" : "images") : r.kind === "slack_files" ? (gained === 1 ? "document" : "documents") : "items";
        parts.push(`${num(gained)} ${what} matched to conversations`);
      }
    }
    const pa = a.coverage.content_examined_percent, pb = b.coverage.content_examined_percent;
    // Only mention coverage when it improved; media added in this release is
    // counted but not read, which lowers the share without being bad news.
    if (pa != null && pb != null && pb - pa >= 0.1) parts.push(`contents examined ${pa}% → ${pb}%`);
    if (!parts.length) parts.push("nothing new was found in it");
    return `<div class="update-banner reveal"><div class="update-icon" aria-hidden="true">✓</div><div><b>Added ${esc(names(paths))}</b><span>${esc(parts.join(" · "))}</span></div></div>`;
  }

  // add-data menu: files or a folder
  function addDataMenu(anchor) {
    const m = $("add-menu");
    const r = anchor.getBoundingClientRect();
    m.style.top = r.bottom + 6 + "px";
    m.style.left = Math.max(12, Math.min(innerWidth - 240, r.right - 230)) + "px";
    m.hidden = false;
    setTimeout(() => document.addEventListener("click", closeMenu, { once: true }), 0);
  }
  function closeMenu() { $("add-menu").hidden = true; }
  $("menu-files").onclick = async () => { closeMenu(); const p = await invoke("pick_files"); if (p && p.length) addToMap(p); };
  $("menu-folder").onclick = async () => { closeMenu(); const p = await invoke("pick_folder"); if (p && p.length) addToMap(p); };

  // ------------------------------------------- continue where you left off
  async function offerResume() {
    let last = null;
    try { last = await invoke("last_session"); } catch (_) { /* first run */ }
    if (!last || selections.length) return;
    const when = last.saved_unix ? new Date(last.saved_unix * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "";
    const list = last.paths.map((p) => p.split(/[\\/]/).filter(Boolean).pop());
    $("resume").innerHTML = `
      <div><p class="eyebrow">Continue where you left off</p>
        <b>${num(list.length)} ${list.length === 1 ? "source" : "sources"}${when ? " · last assessed " + esc(when) : ""}</b>
        <span>${esc(list.slice(0, 4).join(", "))}${list.length > 4 ? ", …" : ""}${last.missing ? ` · ${num(last.missing)} no longer found` : ""}</span></div>
      <div class="resume-actions"><button class="btn ghost" id="resume-fresh">Start fresh</button><button class="btn primary" id="resume-go">Reopen my map</button></div>`;
    $("resume").hidden = false;
    $("resume-go").onclick = async () => {
      selections = await invoke("describe_paths", { paths: last.paths });
      renderSelection();
      $("resume").hidden = true;
      resumeExclude = last.exclude || [];
      startAssess();
    };
    $("resume-fresh").onclick = async () => { await invoke("forget_session"); $("resume").hidden = true; };
  }

  listen("scout://progress", (e) => {
    const p = e.payload;
    $("c-files").textContent = num(p.files_seen);
    $("c-bytes").textContent = bytes(p.bytes_read);
    $("c-records").textContent = num(p.records);
    $("assess-current").textContent = `Reading ${p.collection}`;
    if (!$("adding").hidden) $("adding-detail").textContent = `Reading ${p.collection} · ${bytes(p.bytes_read)} examined in total`;
    setStage(p.records > 0 ? 2 : p.files_seen > 3 ? 1 : 0);
  });
  listen("scout://stage", () => { setStage(3); $("assess-current").textContent = "Checking duplicates, links and gaps…"; });
  $("stop").onclick = async () => { $("stop").textContent = "Stopping…"; $("stop").disabled = true; await invoke("stop"); };

  // --------------------------------------------------- training value
  // The evaluation re-reads everything; progress updates the section live.
  const evalState = { running: false, progress: null };
  function evalHost() { return document.getElementById("training-value"); }
  async function runEvaluation(map) {
    if (evalState.running || assessing) return;
    evalState.running = true;
    evalState.progress = null;
    window.ScoutMap.setTab && window.ScoutMap.setTab("training");
    await showMap(currentMap || map); // shows the tabs and the progress view
    window.ScoutMap.setTab("training");
    try {
      const ev = await invoke("evaluate");
      evalState.running = false;
      await showMap(currentMap || map);
      window.ScoutMap.setTab("training");
      toast(ev.incomplete ? "Evaluation stopped: the numbers cover what was read." : "Training value evaluated.");
    } catch (err) {
      evalState.running = false;
      await showMap(currentMap || map);
      toast("The evaluation could not finish: " + err);
    }
  }
  listen("scout://evaluate", (e) => {
    if (!evalState.running) return;
    evalState.progress = e.payload;
    const handlers = { onStopEvaluate: () => invoke("stop") };
    const h = evalHost();
    if (h) window.ScoutMap.renderTraining(h, currentMap, null, handlers, evalState);
    const card = document.getElementById("tv-card");
    if (card) window.ScoutMap.renderTvCard(card, currentMap, null, evalState, handlers);
  });

  // ------------------------------------------- report for Working Minds
  // The owner's own email program sends it; this app never connects anywhere.
  let reportText = "";
  async function openReport() {
    let preview;
    try { preview = await invoke("wm_report_preview"); } catch (e) { toast(String(e)); return; }
    reportText = preview.markdown;
    $("report-body").innerHTML = `
      <p class="eyebrow">Send to Working Minds</p>
      <h2>Your detailed assessment report</h2>
      <p>A full report of the measurements, written so Working Minds can review it with an AI model. Sending it opens your own email program addressed to <b>${esc(preview.email)}</b>, with the report saved in Documents › Working Minds for you to attach. You decide whether to send it.</p>
      <div class="guide-callout"><b>What it contains</b><br>Your company name, the names of the items you added, and counts, scores and distributions: tokens, quality, duplicates, years, languages, platforms and gaps.<br><b>What it never contains</b><br>Messages, documents, email text, email addresses, people's names or file paths.</div>
      <div class="report-preview-head"><span class="muted small">Preview · ${num(Math.round(preview.bytes / 1024))} KB</span></div>
      <pre class="report-preview">${esc(preview.markdown)}</pre>
      <div class="actions-row"><button class="btn ghost" id="report-copy">Copy report</button><div><button class="btn secondary" id="report-save">Save file only</button> <button class="btn primary" id="report-send">Open email to Working Minds</button></div></div>`;
    $("report-panel").setAttribute("aria-hidden", "false");
    $("scrim").classList.add("on");
    $("report-copy").onclick = async () => {
      try { await navigator.clipboard.writeText(reportText); toast("Report copied. You can paste it into an email or an AI chat."); }
      catch (_) { toast("Could not copy; use Save file only instead."); }
    };
    const send = async (openEmail) => {
      try {
        const path = await invoke("wm_report_send", { openEmail });
        toast(openEmail ? "Email opened. Attach the report from the folder that just opened." : "Report saved.", { label: "Show in folder", run: () => invoke("reveal", { path }) });
      } catch (e) { toast("Could not save the report: " + e); }
    };
    $("report-send").onclick = () => send(true);
    $("report-save").onclick = () => send(false);
  }
  $("report-close").onclick = () => panel(false);

  // ---------------------------------------------------------------- map
  async function showMap(map, banner) {
    lastSaved = null;
    currentMap = map;
    let plan = null;
    try { plan = await invoke("get_source_plan"); } catch (_) { /* first run */ }
    const [linkCounts, removedItems, evaluation] = await Promise.all([
      invoke("link_file_counts").catch(() => ({})),
      invoke("removed_items").catch(() => []),
      invoke("evaluation_result").catch(() => null),
    ]);
    show("screen-map");
    window.ScoutMap.render($("map-page"), map, {
      onSave: async () => {
        try {
          lastSaved = await invoke("save_map");
          const parts = lastSaved.split(/[\\/]/).filter(Boolean);
          toast("Saved to " + parts.slice(-3, -1).join(" › "), { label: "Show in folder", run: () => invoke("reveal", { path: lastSaved }) });
        } catch (err) { toast("Could not save: " + err); }
      },
      onRestart: async () => { await invoke("forget_session"); selections = []; currentMap = null; renderSelection(); show("screen-add"); },
      onAddData: (btn) => addDataMenu(btn),
      onGuide: (kind) => openGuide(kind),
      onPack: () => openPack(),
      onPlan: (p) => invoke("set_source_plan", { plan: p }).catch((e) => toast("Could not save your list: " + e)),
      files: fileHandlers,
      onEvaluate: () => runEvaluation(map),
      onStopEvaluate: () => invoke("stop"),
      onSendReport: () => openReport(),
    }, { banner, plan, linkCounts, removedItems, evaluation, evaluating: evalState.running ? evalState : null });
  }

  // ---------------------------------------------------------------- guides
  const TOOL_ICON = `<img src="downloader-icon.png" alt="" width="46" height="46">`;
  const SLACK_GUIDE = `
    <p class="eyebrow">How to get these</p>
    <h2>Get the files shared in Slack</h2>
    <p>Slack exports contain <b>links</b> to shared images and documents, not the files themselves. To include them, download them from Slack into a folder on this computer.</p>
    <div class="guide-tool">${TOOL_ICON}<div><b>Working Minds Attachment Downloader</b><span>A separate companion app. It reads your export and downloads the files straight from Slack, using the links already inside it. It connects only to Slack and never sends anything to us.</span></div></div>
    <button class="btn primary" data-open-download>Get the Attachment Downloader</button>
    <p class="guide-note">Opens the download page in your web browser. Working Minds itself stays offline.</p>
    <ol class="guide-steps" style="margin-top:18px">
      <li><b>Install and open</b> the Attachment Downloader.</li>
      <li><b>Drop in the same Slack export</b> you added here and choose <b>Download from Slack</b>. It saves the files to a folder called “<i>your export</i> attachments”.</li>
      <li><b>Drop that folder onto this map</b>, or choose <b>Add data</b>. Only the new files are read and your map updates in place.</li>
      <li>The amber circles on your map <b>turn teal</b> for every file found.</li>
    </ol>
    <div class="guide-callout"><b>Good to know</b><br>
      Exports from Slack's Free plan only include file links from the last 90 days. Files deleted from Slack can't be downloaded, and links stop working if your workspace admin revokes the export's links. On Enterprise plans, an admin can request a Discovery API export, which includes the files themselves.</div>
    <p class="guide-note"><b>Prefer to do it by hand?</b> Each message's <code class="k">files</code> section in the export has a download link you can open in a browser. Save each file into <code class="k">attachments/&lt;file ID&gt;/</code> so Working Minds can match it to its message.</p>`;
  const GUIDES = {
    slack_images: SLACK_GUIDE,
    slack_files: SLACK_GUIDE,
    code_reviews: `
      <p class="eyebrow">How to get these</p>
      <h2>Add the code behind the pull requests</h2>
      <p>Your conversations link to pull requests. Adding the repositories, with their history, connects the discussion to the code changes it was about.</p>
      <ol class="guide-steps" style="margin-top:14px">
        <li><b>Make a full local copy</b> of each repository using <b>Clone</b> in GitHub Desktop, Bitbucket or GitLab, or <code class="k">git clone &lt;repository address&gt;</code>.</li>
        <li><b>Avoid “Download ZIP”.</b> It leaves out the history and pull-request links that make the code valuable.</li>
        <li><b>Drop the cloned folders onto this map</b>, or choose <b>Add data</b>.</li>
      </ol>
      <div class="guide-callout"><b>Good to know</b><br>This release counts and profiles repository files. Matching pull requests to commits and reading history arrive in a later release, so this item stays amber for now even after you add the code.</div>`,
  };
  // How to export each kind of source. Kept short; the owner follows the
  // service's own admin screens. Nothing here is fetched from the internet.
  const EXPORT_GUIDES = {
    discord: ["Each person can request their own data in Discord: User Settings › Data & Privacy › Request your data. The package holds only the messages they wrote.", "For whole servers or channels, a server admin can export channel history with a tool such as DiscordChatExporter (JSON); Working Minds reads those exports too."],
    teams: ["Ask your Microsoft 365 admin for an export of the Teams chats and channels: Microsoft Purview eDiscovery (choose HTML or individual messages rather than PST, which this release can count but not read yet), or a Teams Export API / Graph export, which saves JSON files.", "On free or personal Teams, use Export your data in your Microsoft account; it gives a ZIP with messages.json."],
    github: ["Clone each repository with its full history, using GitHub Desktop or <code class=\"k\">git clone</code>. Avoid “Download ZIP”, which drops the history.", "Pull requests and issues aren't part of a clone. An organisation owner can export them as a migration archive."],
    gitlab: ["Clone each repository with its full history (<code class=\"k\">git clone</code>).", "To include issues and merge requests, export the project from its settings (Export project)."],
    bitbucket: ["Clone each repository with its full history (<code class=\"k\">git clone</code>).", "Pull requests aren't part of a clone; note which repositories they belong to so they can be exported separately."],
    jira: ["From an issue search, export the issues (CSV) including comments.", "For everything, a Jira admin can create a site backup in the backup manager."],
    linear: ["A workspace admin can export issues as CSV from the workspace settings."],
    asana: ["Export a project as CSV or JSON from its menu, or ask an admin for an organisation export."],
    trello: ["Export a board as JSON from the board menu (Print, export and share)."],
    google_drive: ["Use Google Takeout and choose Drive. Docs, Sheets and Slides arrive as Office files.", "Unzip the download or add the ZIP files as they are."],
    notion: ["A workspace owner can export all content from Settings (Export all workspace content), as Markdown and CSV."],
    confluence: ["Export each space (HTML or XML) from its space settings, or ask an admin for a site backup."],
    microsoft365: ["Sync or download the relevant OneDrive or SharePoint folders to this computer, then add the folder."],
    dropbox: ["Download the relevant folders, or let the Dropbox app sync them, then add the folder."],
    box: ["Download the relevant folders from Box, then add the folder."],
    figma: ["Design files are of limited use as training data. If they matter, export them from Figma's File menu."],
    miro: ["Export boards from the board menu. Like design files, they are usually of limited use as training data."],
    zendesk: ["An admin can request a data export (JSON or CSV) from the admin centre. Tickets with comments are the valuable part."],
    intercom: ["An admin can export conversations from the workspace settings (data export)."],
    salesforce: ["An admin can run the Data Export service from Setup and download the files when it finishes."],
    hubspot: ["Export records and their activity from each object view, or ask an admin for an account export."],
    sentry: ["Issue data can be exported from Sentry's organisation settings, or through its API by an admin."],
    datadog: ["Monitoring data is usually of limited use as training data. If incidents and notes matter, export them from Datadog."],
    pagerduty: ["Incident history can be exported as CSV from the incidents and analytics pages."],
    circleci: ["Build history is mostly already visible through bot messages. Exports need an admin and its API."],
    loom: ["Videos can be downloaded one by one from Loom. Transcripts are often more useful than the video itself."],
    zoom: ["Cloud recordings and transcripts can be downloaded from the Zoom web portal (Recordings)."],
    airtable: ["Download each table as CSV from its view menu."],
    email: ["Export mailboxes as MBOX (Google Takeout for Gmail) or EML, then add the files."],
    custom: ["Look for an export, download or backup option in the service's admin settings.", "Then add the exported files or folder to your map."],
  };
  function exportGuide(id) {
    const s = ((currentMap && currentMap.source_catalog) || []).find((x) => x.id === id);
    const steps = EXPORT_GUIDES[id] || EXPORT_GUIDES.custom;
    const src = ((currentMap && currentMap.sources) || []).find((x) => x.id === id);
    return `<p class="eyebrow">How to export</p><h2>${esc(s ? s.name : "Your source")}</h2>
      ${src && src.evidence ? `<div class="guide-callout"><b>Why it's suggested</b><br>${esc(src.evidence)}${src.referenced_from && src.referenced_from.length ? `, in ${esc(src.referenced_from.join(", "))}` : ""}.</div>` : ""}
      <ol class="guide-steps">${steps.map((x) => `<li>${x}</li>`).join("")}<li><b>Drop the export onto your map</b>, or choose <b>Add data</b>. Only the new material is read.</li></ol>
      <p class="guide-note">Exports stay on this computer. Working Minds reads them locally and never connects to these services.</p>`;
  }
  function openGuide(kind) {
    $("guide-body").innerHTML = kind.startsWith("export:") ? exportGuide(kind.slice(7)) : GUIDES[kind] || "<h2>No guide yet</h2>";
    const b = $("guide-body").querySelector("[data-open-download]");
    if (b) b.onclick = () => invoke("open_download_page").catch((e) => toast("Could not open the page: " + e));
    $("guide-panel").setAttribute("aria-hidden", "false");
    $("scrim").classList.add("on");
  }
  $("guide-close").onclick = () => panel(false);

  // ---------------------------------------------------------------- data pack
  let packDest = null, packing = false;
  // Items that don't seem to belong with the rest, for the data-pack panel.
  function unlinkedWarning() {
    const list = ((currentMap && currentMap.collections) || []).filter((c) => c.linkage && c.linkage.status === "unlinked");
    if (!list.length) return "";
    return `<div class="guide-callout warn"><b>${list.length === 1 ? "1 item doesn't" : `${list.length} items don't`} seem connected to the rest of your data</b><br>${list.map((c) => esc(c.display_label || c.label)).join(", ")}. Check ${list.length === 1 ? "it belongs" : "they belong"} to the same organisation before sharing, or remove ${list.length === 1 ? "it" : "them"} from the map.</div>`;
  }

  async function openPack() {
    let plan;
    try { plan = await invoke("package_plan"); } catch (e) { toast(String(e)); return; }
    const byFolder = new Map();
    plan.entries.forEach((e) => e.folders.forEach((f) => {
      const x = byFolder.get(f) || { from: [], files: 0, bytes: 0 };
      x.from.push(e.collection); x.files += e.folders.length > 1 ? 0 : e.files; x.bytes += e.folders.length > 1 ? 0 : e.bytes;
      byFolder.set(f, x);
    }));
    const rows = [...byFolder.entries()].sort((a, b) => a[0].localeCompare(b[0])).map(([f, x]) =>
      `<tr><td><span class="folder-ico">▸</span><b>${esc(f)}/</b></td><td class="muted">${esc(x.from.join("; "))}</td></tr>`).join("");
    $("pack-body").innerHTML = `
      <p class="eyebrow">Create data pack</p>
      <h2>One folder, tidy names, ready to share</h2>
      <p>Working Minds copies everything you added into a new folder with standard names, ready to put on a drive. Google Takeout parts are unpacked and merged by product. Your originals aren't changed and nothing is uploaded.</p>
      <table class="tbl compact pack-tbl"><thead><tr><th>Folder in the pack</th><th>From</th></tr></thead><tbody>${rows}</tbody></table>
      <p class="muted small">About ${num(plan.total_files)} files, ${bytes(plan.total_bytes)}${plan.excluded_files ? ` · ${num(plan.excluded_files)} files you excluded are left out` : ""}. Includes a README explaining the layout.</p>
      ${unlinkedWarning()}
      <div class="guide-callout"><b>This folder contains your actual data</b><br>Messages, documents, email and code are copied as they are. Only share it under an agreement you're happy with.</div>
      <div class="card dest-row"><div><div class="dest-label">Create it in</div><div class="dest-path" id="pack-dest">${packDest ? esc(packDest) : "No folder chosen yet"}</div></div><button class="btn ghost small" id="pack-pick">Choose folder…</button></div>
      <div id="pack-progress" hidden>
        <div class="bar"><span id="pack-bar"></span></div>
        <p class="muted small" id="pack-status">Starting…</p>
      </div>
      <div class="actions-row"><span></span><div><button class="btn ghost" id="pack-stop" hidden>Stop</button> <button class="btn primary" id="pack-go" ${packDest ? "" : "disabled"}>Create data pack</button></div></div>`;
    $("pack-panel").setAttribute("aria-hidden", "false");
    $("scrim").classList.add("on");
    $("pack-pick").onclick = async () => {
      const p = await invoke("pick_package_dest");
      if (p) { packDest = p; $("pack-dest").textContent = p; $("pack-go").disabled = false; }
    };
    $("pack-go").onclick = async () => {
      if (!packDest || packing) return;
      packing = true;
      $("pack-go").disabled = true; $("pack-pick").disabled = true;
      $("pack-progress").hidden = false; $("pack-stop").hidden = false;
      try {
        const s = await invoke("build_package", { dest: packDest });
        const ok = !s.error && !s.stopped;
        $("pack-bar").style.width = ok ? "100%" : $("pack-bar").style.width;
        $("pack-status").innerHTML = ok
          ? `<b>Your data pack is ready.</b> ${num(s.files_copied)} files, ${bytes(s.bytes_copied)}.${s.skipped_unsafe ? ` ${num(s.skipped_unsafe)} unsafe archive entries were skipped.` : ""}`
          : s.stopped ? "Stopped. The folder has what was copied so far, and its README says it is incomplete."
          : `Copying stopped: ${esc(s.error)}. The folder has what was copied so far.`;
        $("pack-stop").hidden = true;
        $("pack-go").outerHTML = `<button class="btn primary" id="pack-show">Show folder</button>`;
        $("pack-show").onclick = () => invoke("reveal", { path: s.folder });
      } catch (e) { toast("Could not create the pack: " + e); }
      packing = false;
    };
    $("pack-stop").onclick = async () => { $("pack-stop").disabled = true; $("pack-stop").textContent = "Stopping…"; await invoke("stop"); };
  }
  listen("scout://package", (e) => {
    const p = e.payload;
    const share = p.total_bytes ? Math.min(100, (p.bytes_copied * 100) / p.total_bytes) : 0;
    $("pack-bar").style.width = share + "%";
    $("pack-status").textContent = `${num(p.files_copied)} files copied · ${bytes(p.bytes_copied)} of about ${bytes(p.total_bytes)} · ${p.current}`;
  });
  $("pack-close").onclick = () => panel(false);

  // ---------------------------------------------------------------- chrome
  let toastTimer;
  function toast(msg, action) {
    const el = $("toast");
    el.innerHTML = `<span>${esc(msg)}</span>${action ? `<button>${esc(action.label)}</button>` : ""}`;
    if (action) el.querySelector("button").onclick = action.run;
    el.classList.add("on");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove("on"), action ? 7000 : 4000);
  }
  function panel(open) {
    $("privacy-panel").setAttribute("aria-hidden", String(!open));
    if (!open) $("guide-panel").setAttribute("aria-hidden", "true");
    if (!open && !packing) $("pack-panel").setAttribute("aria-hidden", "true");
    if (!open) $("report-panel").setAttribute("aria-hidden", "true");
    $("scrim").classList.toggle("on", open);
  }
  $("privacy-open").onclick = () => panel(true);
  $("privacy-close").onclick = () => panel(false);
  $("scrim").onclick = () => panel(false);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") panel(false); });
  window.addEventListener("scroll", () => document.body.classList.toggle("scrolled", scrollY > 4), { passive: true });

  invoke("initial_paths").then((p) => (p && p.length ? addPaths(p) : offerResume())).catch(() => offerResume());

  window.ScoutApp = { addPaths, showMap };
})();
