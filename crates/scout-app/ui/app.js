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

  async function addPaths(paths) {
    if (!paths || !paths.length || assessing) return;
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
  listen("scout://drag", (e) => document.body.classList.toggle("dragging", e.payload === "over" && !assessing));
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

  $("start").onclick = async () => {
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
  };

  listen("scout://progress", (e) => {
    const p = e.payload;
    $("c-files").textContent = num(p.files_seen);
    $("c-bytes").textContent = bytes(p.bytes_read);
    $("c-records").textContent = num(p.records);
    $("assess-current").textContent = `Reading ${p.collection}`;
    setStage(p.records > 0 ? 2 : p.files_seen > 3 ? 1 : 0);
  });
  listen("scout://stage", () => { setStage(3); $("assess-current").textContent = "Checking duplicates, links and gaps…"; });
  $("stop").onclick = async () => { $("stop").textContent = "Stopping…"; $("stop").disabled = true; await invoke("stop"); };

  // ---------------------------------------------------------------- map
  function showMap(map) {
    lastSaved = null;
    show("screen-map");
    window.ScoutMap.render($("map-page"), map, {
      onSave: async () => {
        try {
          lastSaved = await invoke("save_map");
          const parts = lastSaved.split(/[\\/]/).filter(Boolean);
          toast("Saved to " + parts.slice(-3, -1).join(" › "), { label: "Show in folder", run: () => invoke("reveal", { path: lastSaved }) });
        } catch (err) { toast("Could not save: " + err); }
      },
      onRestart: () => { selections = []; renderSelection(); show("screen-add"); },
    });
  }

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
    $("scrim").classList.toggle("on", open);
  }
  $("privacy-open").onclick = () => panel(true);
  $("privacy-close").onclick = () => panel(false);
  $("scrim").onclick = () => panel(false);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") panel(false); });
  window.addEventListener("scroll", () => document.body.classList.toggle("scrolled", scrollY > 4), { passive: true });

  invoke("initial_paths").then(addPaths).catch(() => {});

  window.ScoutApp = { addPaths, showMap };
})();
