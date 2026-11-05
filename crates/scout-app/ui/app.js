/* DataPack Scout — screen flow and wiring to the Rust core.
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
      const map = await invoke("assess", { paths: selections.map((s) => s.path) });
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

  // ---------------------------------------------------------------- map
  function showMap(map, banner) {
    lastSaved = null;
    currentMap = map;
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
    }, { banner });
  }

  // ---------------------------------------------------------------- guides
  const TOOL_ICON = `<svg viewBox="0 0 64 64" aria-hidden="true"><rect x="2" y="2" width="60" height="60" rx="15" fill="#14283F"/><circle cx="32" cy="29" r="13" fill="#E8AA40"/><path d="M32 21v12m-5-4 5 5 5-5M26 39h12" stroke="#fff" stroke-width="3" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const SLACK_GUIDE = `
    <p class="eyebrow">How to get these</p>
    <h2>Get the files shared in Slack</h2>
    <p>Slack exports contain <b>links</b> to shared images and documents, not the files themselves. To include them, download them from Slack into a folder on this computer.</p>
    <div class="guide-tool">${TOOL_ICON}<div><b>DataPack Scout Attachment Downloader</b><span>A separate companion app. It reads your export and downloads the files straight from Slack, using the links already inside it. It connects only to Slack and never sends anything to us.</span></div></div>
    <button class="btn primary" data-open-download>Get the Attachment Downloader</button>
    <p class="guide-note">Opens the download page in your web browser. DataPack Scout itself stays offline.</p>
    <ol class="guide-steps" style="margin-top:18px">
      <li><b>Install and open</b> the Attachment Downloader.</li>
      <li><b>Drop in the same Slack export</b> you added here and choose <b>Download from Slack</b>. It saves the files to a folder called “<i>your export</i> attachments”.</li>
      <li><b>Drop that folder onto this map</b>, or choose <b>Add data</b>. Only the new files are read and your map updates in place.</li>
      <li>The amber circles on your map <b>turn teal</b> for every file found.</li>
    </ol>
    <div class="guide-callout"><b>Good to know</b><br>
      Exports from Slack's Free plan only include file links from the last 90 days. Files deleted from Slack can't be downloaded, and links stop working if your workspace admin revokes the export's links. On Enterprise plans, an admin can request a Discovery API export, which includes the files themselves.</div>
    <p class="guide-note"><b>Prefer to do it by hand?</b> Each message's <code class="k">files</code> section in the export has a download link you can open in a browser. Save each file into <code class="k">attachments/&lt;file ID&gt;/</code> so DataPack Scout can match it to its message.</p>`;
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
  function openGuide(kind) {
    $("guide-body").innerHTML = GUIDES[kind] || "<h2>No guide yet</h2>";
    const b = $("guide-body").querySelector("[data-open-download]");
    if (b) b.onclick = () => invoke("open_download_page").catch((e) => toast("Could not open the page: " + e));
    $("guide-panel").setAttribute("aria-hidden", "false");
    $("scrim").classList.add("on");
  }
  $("guide-close").onclick = () => panel(false);

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
