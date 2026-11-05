/* DataPack Scout Attachment Downloader — interface. The webview never makes
 * network requests itself; downloads run in Rust, restricted to Slack. */
(function () {
  "use strict";
  const T = window.__TAURI__;
  const invoke = (c, a) => T.core.invoke(c, a);
  const $ = (id) => document.getElementById(id);
  const NF = new Intl.NumberFormat();
  const num = (n) => NF.format(n || 0);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  function bytes(n) {
    if (!n) return "0 B";
    const u = ["B", "KB", "MB", "GB", "TB"], i = Math.min(4, Math.floor(Math.log(n) / Math.log(1024)));
    const v = n / Math.pow(1024, i);
    return (v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)) + " " + u[i];
  }
  let info = null, out = null, busy = false;

  function show(id) { document.querySelectorAll(".screen").forEach((s) => s.toggleAttribute("data-active", s.id === id)); }
  let tt;
  function toast(msg) { const el = $("toast"); el.textContent = msg; el.classList.add("on"); clearTimeout(tt); tt = setTimeout(() => el.classList.remove("on"), 5000); }

  async function choose(path) {
    if (!path || busy) return;
    try {
      info = await invoke("inspect", { path });
    } catch (e) { toast(String(e)); return; }
    out = info.default_out;
    $("r-title").textContent = info.export_name;
    $("r-files").textContent = num(info.files);
    $("r-images").textContent = num(info.images);
    $("r-docs").textContent = num(info.documents);
    $("r-size").textContent = info.known_bytes ? "≈ " + bytes(info.known_bytes) : "—";
    const skipped = [];
    if (info.skipped_hidden) skipped.push(`${num(info.skipped_hidden)} hidden or deleted in Slack`);
    if (info.skipped_external) skipped.push(`${num(info.skipped_external)} stored outside Slack (for example Google Drive links)`);
    if (info.skipped_unusable) skipped.push(`${num(info.skipped_unusable)} without a usable link`);
    $("r-skipped").textContent = skipped.length ? "Not downloadable: " + skipped.join(", ") + "." : "";
    $("r-out").textContent = out;
    $("go").disabled = info.files === 0;
    if (info.files === 0) toast("This export does not link to any downloadable files.");
    show("s-review");
  }

  $("pick-zip").onclick = async () => choose(await invoke("pick_export"));
  $("pick-dir").onclick = async () => choose(await invoke("pick_export_folder"));
  $("back").onclick = () => show("s-choose");
  $("change-out").onclick = async () => { const p = await invoke("pick_output"); if (p) { out = p; $("r-out").textContent = p; } };
  $("again").onclick = () => show("s-choose");
  $("show").onclick = () => invoke("reveal", { path: out });
  $("stop").onclick = async () => { $("stop").disabled = true; $("stop").textContent = "Stopping…"; await invoke("stop"); };

  T.event.listen("fetch://drag", (e) => document.body.classList.toggle("dragging", e.payload === "over" && !busy && $("s-choose").hasAttribute("data-active")));
  T.event.listen("fetch://drop", (e) => { document.body.classList.remove("dragging"); if ($("s-choose").hasAttribute("data-active")) choose(e.payload[0]); });
  window.addEventListener("dragover", (e) => e.preventDefault());
  window.addEventListener("drop", (e) => e.preventDefault());

  T.event.listen("fetch://progress", (e) => {
    const p = e.payload;
    $("p-title").textContent = `${num(p.finished)} of ${num(p.total)} files`;
    $("p-bar").style.width = (p.total ? (p.finished * 100) / p.total : 0) + "%";
    $("p-current").textContent = p.current ? "Fetching " + p.current : "Connecting to Slack…";
    $("p-done").textContent = num(p.downloaded);
    $("p-skip").textContent = num(p.already_present);
    $("p-fail").textContent = num(p.failed);
    $("p-bytes").textContent = bytes(p.bytes) + " received";
  });

  $("go").onclick = async () => {
    busy = true;
    $("stop").disabled = false; $("stop").textContent = "Stop";
    show("s-progress");
    try {
      const s = await invoke("start", { out });
      finish(s);
    } catch (e) { toast(String(e)); show("s-review"); }
    busy = false;
  };

  function finish(s) {
    const p = s.progress;
    const got = p.downloaded + p.already_present;
    const partial = p.failed > 0 || s.stopped;
    document.querySelector(".done-badge").classList.toggle("partial", partial);
    $("d-eyebrow").textContent = s.stopped ? "Download stopped" : partial ? "Download finished, with some gaps" : "Download complete";
    $("d-title").textContent = `${num(got)} of ${num(p.total)} files are on this computer`;
    $("d-sub").textContent = `${num(p.downloaded)} downloaded now${p.already_present ? `, ${num(p.already_present)} were already here` : ""} · ${bytes(p.bytes)} received`;
    const f = $("d-failures");
    f.hidden = !s.failures.length;
    f.innerHTML = s.failures.length
      ? `<h3>${num(p.failed)} could not be downloaded</h3>` +
        s.failures.map((x) => `<div class="row"><b>${num(x.files)}</b><span>${esc(x.explanation)}</span></div>`).join("") +
        `<p>Run the download again later to retry; files already saved are skipped. If links have expired, ask your Slack admin for a fresh export.</p>`
      : "";
    show("s-done");
  }

  invoke("initial_paths").then((p) => p && p[0] && choose(p[0])).catch(() => {});
})();
