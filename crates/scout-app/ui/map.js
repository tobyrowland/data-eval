/* Working Minds — "Your data map" renderer.
 * Pure DOM/SVG, no libraries, no network. Every label that comes from the
 * user's files is escaped before it is placed in markup. */
(function () {
  "use strict";

  const NF = new Intl.NumberFormat();
  const num = (n) => NF.format(Math.round(n || 0));
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  function bytes(n) {
    if (!n) return "0 B";
    const u = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    const v = n / Math.pow(1024, i);
    return (v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)) + " " + u[i];
  }
  function trunc(s, n) { s = String(s); return s.length > n ? s.slice(0, n - 1) + "…" : s; }

  const CAT = {
    "Documents": "#187C75", "Tables": "#2F6FB0", "Structured data": "#6D5BD0", "Email": "#C2577A",
    "Media": "#D9921F", "Archives": "#7B8AA0", "Other": "#AEB8C6",
  };
  const STATUS = [
    ["assessed", "Assessed", "#187C75"],
    ["partly_assessed", "Partly assessed", "#E8B04B"],
    ["inventoried", "Inventoried", "#AEB8C6"],
    ["locked", "Locked", "#C2413A"],
    ["unavailable", "Unavailable", "#D9921F"],
    ["excluded", "Excluded", "#DDE3EA"],
    ["stopped_safely", "Stopped safely", "#5E6E84"],
  ];

  function sumSlack(c) {
    const out = {};
    for (const s of c.slack || []) for (const k in s) if (typeof s[k] === "number") out[k] = (out[k] || 0) + s[k];
    return out;
  }
  function dominantCategory(c) {
    const by = {};
    for (const f of c.formats || []) by[f.category] = (by[f.category] || 0) + f.files;
    return Object.entries(by).sort((a, b) => b[1] - a[1])[0]?.[0] || "Other";
  }
  // Collections where attachments referenced by an export were found, with
  // how many matched. Set per render.
  let attachTargets = new Map();
  function kindInfo(c) {
    if ((c.slack || []).length) return { badge: "#", color: "#6D5BD0", kind: "Slack workspace export" };
    if (c.teams) return { badge: "T", color: "#5059C9", kind: "Microsoft Teams export" };
    if ((c.platforms || []).includes("teams")) return { badge: "↓", color: "#5059C9", kind: "Teams linked files" };
    if (c.discord) return { badge: "D", color: "#5865F2", kind: c.discord.own_messages_only ? "Discord data package" : "Discord channel export" };
    if (attachTargets.has(c.label)) return { badge: "↓", color: "#22A094", kind: "Slack attachments", matched: attachTargets.get(c.label) };
    if ((c.display_label || "").startsWith("Google Takeout")) return { badge: "G", color: "#2F6FB0", kind: "Google Takeout" };
    if ((c.display_label || "").endsWith("(Git repository)")) return { badge: "‹›", color: "#5E6E84", kind: "Git repository" };
    if ((c.display_label || "").endsWith("(code download)")) return { badge: "‹›", color: "#5E6E84", kind: "Code download" };
    const cat = dominantCategory(c);
    if (cat === "Email") return { badge: "@", color: CAT.Email, kind: "Email archive" };
    if ((c.formats || []).length === 1 && c.logical_files === 1) {
      return { badge: (c.formats[0].format || "F")[0], color: CAT[cat] || CAT.Other, kind: c.formats[0].format + " file" };
    }
    return { badge: "▤", color: "#187C75", kind: "Files and documents" };
  }

  // ---------------------------------------------------------------- map
  // One bubble per platform around the company, sized by unique content.
  const PLATFORM_COLOR = { slack: "#6D5BD0", teams: "#5059C9", discord: "#5865F2", email: "#C4577A", google_drive: "#2F6FB0", google_other: "#4F86C6", github: "#5E6E84", gitlab: "#7A5E84", bitbucket: "#3D6E9E", other: "#8A97A8" };

  /** Collected platforms with their items, plus items tied to none. */
  function platformGroups(map) {
    const byPlat = new Map();
    const cols = map.collections || [];
    cols.forEach((c) => (c.platforms || []).forEach((p) => { if (!byPlat.has(p)) byPlat.set(p, []); byPlat.get(p).push(c); }));
    const rank = (p) => { const i = PLATFORM_ORDER.indexOf(p); return i < 0 ? 99 : i; };
    const collected = [...byPlat.keys()].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    const loose = cols.filter((c) => !(c.platforms || []).length);
    return { byPlat, collected, loose };
  }

  /** The platform's unique content: messages, emails or files. */
  function platformContent(p, list) {
    const sum = (f) => list.reduce((a, c) => a + (f(c) || 0), 0);
    if (p === "slack") return { value: sum((c) => (c.slack || []).reduce((x, s) => x + s.message_records - (s.records_also_in_other_exports || 0), 0)), unit: "messages" };
    if (p === "teams") { const v = sum((c) => c.teams && c.teams.messages - c.teams.messages_also_in_other_items); if (v) return { value: v, unit: "messages" }; }
    if (p === "discord") return { value: sum((c) => c.discord && c.discord.messages - c.discord.messages_also_in_other_items), unit: "messages" };
    if (p === "email") {
      // Emails in two mailboxes count once: each shared email is listed on
      // both items' overlaps, so half the shared total is the double count.
      const labels = new Set(list.map((c) => c.label));
      const shared = sum((c) => (c.overlaps || []).filter((o) => labels.has(o.with)).reduce((x, o) => x + o.shared_messages, 0));
      const v = sum((c) => c.structured_records) - Math.round(shared / 2);
      if (v > 0) return { value: v, unit: "emails" };
    }
    return { value: sum((c) => c.logical_files), unit: "files" };
  }

  function mapSvg(map, plan, company) {
    const { byPlat, collected, loose } = platformGroups(map);
    const W = 1000, H = 470, cx = W / 2, cy = 210;
    const tips = [];
    const tip = (html) => { tips.push(html); return `data-tip="${tips.length - 1}"`; };
    const nodes = collected.map((p) => ({ p, list: byPlat.get(p) }));
    if (loose.length) nodes.push({ p: "other", list: loose });
    nodes.forEach((n) => Object.assign(n, platformContent(n.p, n.list)));
    const maxV = Math.max(1, ...nodes.map((n) => n.value));
    nodes.forEach((n) => { n.r = 48 + 32 * Math.sqrt(n.value / maxV); });
    const k = nodes.length;
    nodes.forEach((n, i) => {
      const a = k === 1 ? 0 : -Math.PI / 2 + (i * 2 * Math.PI) / k + (k % 2 ? 0 : Math.PI / k);
      n.x = cx + (k === 1 ? 300 : 330 * Math.cos(a));
      n.y = cy + (k === 1 ? 0 : 150 * Math.sin(a));
    });
    const kept = new Set(plan.kept || []);
    const name = company ? company.name : "Your data";
    const total = map.totals.unique_records != null ? map.totals.unique_records : map.totals.structured_records;
    let s = `<svg viewBox="0 0 ${W} ${H}" class="pmap" role="img" aria-label="Your data by platform">`;
    nodes.forEach((n) => { s += `<line x1="${cx}" y1="${cy}" x2="${n.x}" y2="${n.y}" class="pm-spoke"/>`; });
    // the company in the middle
    s += `<g class="pm-hub"><circle cx="${cx}" cy="${cy}" r="74"/>
      <text x="${cx}" y="${cy - 6}" class="pm-hub-name">${esc(trunc(name, 18))}</text>
      <text x="${cx}" y="${cy + 18}" class="pm-hub-sub">${num(total)} records</text></g>`;
    nodes.forEach((n) => {
      const color = PLATFORM_COLOR[n.p] || PLATFORM_COLOR.other;
      const label = n.p === "other" ? "Other files" : PLATFORM_NAMES[n.p] || catalogName(map, n.p);
      const suspect = n.list.some((c) => c.linkage && c.linkage.status === "unlinked" && !kept.has(c.label));
      let badge = "";
      if (suspect) badge = "Check this data";
      else if (n.p === "slack" && map.slack_files) {
        const sf = map.slack_files, miss = sf.images + sf.documents - sf.images_found - sf.documents_found;
        if (miss > 0 && !(plan.done || []).includes("slack-files")) badge = `${num(miss)} files missing`;
      } else if (n.p === "teams" && map.teams_files) {
        const miss = map.teams_files.referenced - map.teams_files.found;
        if (miss > 0 && !(plan.done || []).includes("teams-files")) badge = `${num(miss)} linked files missing`;
      }
      const files = n.list.reduce((a, c) => a + c.logical_files, 0);
      const firsts = n.list.map((c) => c.date_coverage && c.date_coverage.first).filter(Boolean).sort();
      const lasts = n.list.map((c) => c.date_coverage && c.date_coverage.last).filter(Boolean).sort();
      const t = tip(`<b>${esc(label)}</b>${num(n.value)} ${n.unit}<br>${num(n.list.length)} ${n.list.length === 1 ? "item" : "items"} · ${num(files)} files · ${bytes(n.list.reduce((a, c) => a + c.expanded_bytes, 0))}${firsts.length ? `<br>${esc(firsts[0])} → ${esc(lasts[lasts.length - 1])}` : ""}${suspect ? "<br><i>An item here doesn't seem connected to your other data</i>" : ""}`);
      s += `<g class="pm-node ${suspect ? "suspect" : ""}" data-plat="${esc(n.p)}" ${t} tabindex="0" role="button" aria-label="${esc(label)}: ${num(n.value)} ${n.unit}">
        <circle cx="${n.x}" cy="${n.y}" r="${n.r}" style="--c:${color}"/>
        <text x="${n.x}" y="${n.y - 10}" class="pm-name">${esc(trunc(label, 16))}</text>
        <text x="${n.x}" y="${n.y + 12}" class="pm-value">${num(n.value)}</text>
        <text x="${n.x}" y="${n.y + 28}" class="pm-unit">${esc(n.unit)}</text>
        ${badge ? `<g class="pm-badge ${suspect ? "check" : ""}"><rect x="${n.x - 64}" y="${n.y + n.r - 4}" width="128" height="22" rx="11"/><text x="${n.x}" y="${n.y + n.r + 11}">${esc(badge)}</text></g>` : ""}
      </g>`;
    });
    // platforms the data points to but that aren't collected
    const removed = new Set(plan.removed || []);
    const todo = (map.sources || [])
      .filter((x) => !byPlat.has(x.id) && !x.in_selection && !removed.has(x.id) && (x.links || x.app_installed || x.bot_messages))
      .sort((a, b) => (b.links || 0) - (a.links || 0));
    if (todo.length) {
      const shown = todo.slice(0, 6);
      const more = todo.length - shown.length;
      const items = shown.map((x) => x.name).concat(more > 0 ? [`+${more} more`] : []);
      const widths = items.map((t) => 22 + t.length * 7.2);
      const gap = 10, totalW = widths.reduce((a, b) => a + b, 0) + gap * (items.length - 1);
      let x = cx - totalW / 2;
      s += `<text x="${cx}" y="${H - 44}" class="pm-todo-title">Not collected yet</text>`;
      items.forEach((label, i) => {
        const src = shown[i];
        const t = src ? tip(`<b>${esc(src.name)}</b>${esc(src.evidence)}`) : "";
        s += `<g class="pm-todo" ${t} ${src ? `data-todo="${esc(src.id)}"` : ""}><rect x="${x}" y="${H - 34}" width="${widths[i]}" height="26" rx="13"/><text x="${x + widths[i] / 2}" y="${H - 16}">${esc(label)}</text></g>`;
        x += widths[i] + gap;
      });
    }
    s += `</svg>`;
    return { svg: s, tips };
  }



  // ---------------------------------------------------------------- timeline
  function timelineSvg(activity) {
    const keys = Object.keys(activity || {}).sort();
    if (keys.length < 2) return null;
    const [y0, m0] = keys[0].split("-").map(Number);
    const [y1, m1] = keys[keys.length - 1].split("-").map(Number);
    const months = [];
    for (let y = y0, m = m0; y < y1 || (y === y1 && m <= m1); m === 12 ? (y++, m = 1) : m++) {
      const k = `${y}-${String(m).padStart(2, "0")}`;
      months.push({ k, y, m, v: activity[k] || 0 });
    }
    const W = 640, H = 170, pl = 6, pr = 6, pt = 10, pb = 26;
    const max = Math.max(...months.map((d) => d.v));
    const bw = (W - pl - pr) / months.length;
    const tips = [];
    let s = `<svg class="timeline-svg" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="Activity per month">
      <defs><linearGradient id="tg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#22A094"/><stop offset="1" stop-color="#187C75"/></linearGradient></defs>`;
    for (let g = 1; g <= 3; g++) {
      const y = pt + (H - pt - pb) * (1 - g / 3);
      s += `<line x1="${pl}" x2="${W - pr}" y1="${y}" y2="${y}" stroke="#EEF2F7"/>`;
    }
    months.forEach((d, i) => {
      const h = max ? ((H - pt - pb) * d.v) / max : 0;
      const x = pl + i * bw;
      tips.push(`<b>${d.k}</b>${num(d.v)} message records`);
      s += `<rect data-tip="${i}" x="${(x + bw * 0.14).toFixed(2)}" y="${(H - pb - h).toFixed(2)}" width="${Math.max(1, bw * 0.72).toFixed(2)}" height="${Math.max(d.v ? 1.5 : 0, h).toFixed(2)}" rx="${Math.min(3, bw * 0.3).toFixed(1)}" fill="url(#tg)" class="grow" style="animation-delay:${300 + i * 12}ms; transform-origin: bottom"/>`;
      if ((d.m === 1 || i === 0) && x < W - 40 && (i === 0 || i > 3)) {
        s += `<text x="${(x + 2).toFixed(1)}" y="${H - 8}" font-size="11" fill="#6B7C93">${d.m === 1 ? d.y : d.k}</text>`;
      }
    });
    s += `</svg>`;
    return { svg: s, tips, months: months.length };
  }

  // ---------------------------------------------------------------- page
  function span(map) {
    const ds = map.collections.map((c) => c.date_coverage).filter(Boolean);
    if (!ds.length) return { value: "—", note: "no dated content found yet" };
    const first = ds.map((d) => d.first).sort()[0], last = ds.map((d) => d.last).sort().reverse()[0];
    const days = (Date.parse(last) - Date.parse(first)) / 864e5;
    const years = days / 365.25;
    const value = years >= 1 ? `${years.toFixed(1)} yrs` : `${Math.max(1, Math.round(days / 30.44))} mo`;
    return { value, note: `${first.slice(0, 7)} to ${last.slice(0, 7)}` };
  }

  function render(root, map, handlers, extra) {
    extra = extra || {};
    attachTargets = new Map();
    for (const r of map.relationships || []) {
      for (const label of r.found_in || []) attachTargets.set(label, (attachTargets.get(label) || 0) + (r.resolved || 0));
    }
    const t = map.totals, cov = map.coverage;
    const pct = cov.content_examined_percent ?? 0;
    const sp = span(map);
    const plan = normalisePlan(extra.plan);
    // The company: the owner's own name if they set one, else the detected one.
    const company = plan.company ? { name: plan.company, confidence: "set", evidence: [] } : map.company || null;
    const M = mapSvg(map, plan, company);
    const T = timelineSvg(map.activity);
    const lead = map.summary[0] || "";
    const totalCov = Object.values(cov.by_status).reduce((a, b) => a + b, 0) || 1;

    const legend = [
      `<span><i class="lg-solid"></i>Collected</span>`,
      `<span><i class="lg-check"></i>Check this data</span>`,
      `<span><i class="lg-todo"></i>Not collected yet</span>`,
    ].join("");
    const evidence = company && company.evidence && company.evidence.length
      ? `Detected from ${company.evidence.length > 1 ? company.evidence.slice(0, -1).join(", ") + " and " + company.evidence[company.evidence.length - 1] : company.evidence[0]}`
      : company ? "Set by you" : "We couldn't tell whose data this is yet";

    root.innerHTML = `
      <div class="map-head reveal">
        <div>
          <p class="eyebrow">Data map</p>
          <h1 class="company-name" id="company-name">${esc(company ? company.name : "Your data")}</h1>
          <div class="company-src" id="company-src">${esc(evidence)} · <button class="link-btn" data-act="edit-company">${company ? "Edit" : "Add company name"}</button></div>
          <div class="map-id">Assessment ${esc(map.assessment_id)} · ${esc(map.app)} ${esc(map.app_version)} · rules ${esc(map.ruleset_version)}</div>
        </div>
        <div class="actions">
          <button class="btn ghost" data-act="restart">Start again</button>
          <button class="btn secondary" data-act="adddata"><span aria-hidden="true">＋</span> Add data</button>
          <button class="btn secondary" data-act="pack">Create data pack</button>
          <button class="btn primary" data-act="save">Save my data map</button>
        </div>
      </div>
      ${extra.banner || ""}
      <div class="tabs" role="tablist">
        <button role="tab" class="tab ${activeTab === "map" ? "on" : ""}" data-tab="map" aria-selected="${activeTab === "map"}">Inventory</button>
        <button role="tab" class="tab ${activeTab === "training" ? "on" : ""}" data-tab="training" aria-selected="${activeTab === "training"}">Analysis &amp; value${extra.evaluating ? ` <span class="tab-dot" aria-hidden="true"></span>` : ""}</button>
      </div>
      <div class="tab-pane" data-pane="map" ${activeTab === "map" ? "" : "hidden"}>
      <section class="card hero reveal" style="animation-delay:60ms">
        <div class="hero-top">
          <p class="hero-lead">${esc(lead)}</p>
          <div class="hero-stats">
            <div><strong data-count="${t.unique_files != null ? t.unique_files : t.logical_files}">0</strong><span>files</span>${t.exact_duplicate_files ? `<small class="dedup" title="Identical copies of files are counted once">${num(t.exact_duplicate_files)} identical copies not counted</small>` : ""}</div>
            <div><strong data-count="${t.unique_records != null ? t.unique_records : t.structured_records}">0</strong><span>records, messages and emails</span>${t.duplicate_records ? `<small class="dedup" title="A message or email that appears in more than one item is counted once">${num(t.duplicate_records)} duplicates not counted</small>` : ""}</div>
            <div><strong>${esc(sp.value)}</strong><span>${esc(sp.note)}</span></div>
            <div><strong data-count="${pct}" data-fmt="pct">0%</strong><span>contents examined</span></div>
          </div>
        </div>
        <div id="map-svg-host" class="hero-map">${M.svg}</div>
        <div class="hero-foot">
          <div class="legend">${legend}</div>
          <div class="add-hint"><span class="status-dot"></span>Drop more files or folders anywhere to add them. Only new material is read.</div>
        </div>
      </section>

      <section class="section reveal" style="animation-delay:250ms">
        <div class="section-head">
          <div><h2>Your platforms</h2><p>What you've collected from each platform, the items in it and their stats, and what your data points to that's still missing. Remove duplicates and anything that isn't yours.</p></div>
          <div class="section-actions">
            <button class="btn secondary small" data-act="addsource"><span aria-hidden="true">＋</span> Add a source</button>
          </div>
        </div>
        <div id="next-steps"></div>
      </section>

      <div id="tv-card" class="reveal" style="animation-delay:300ms"></div>
      </div>
      <div class="tab-pane" data-pane="training" ${activeTab === "training" ? "" : "hidden"}>
        <div class="section-head tv-pane-head">
          <div><h2>Training value</h2><p>How much of your data is useful for training AI models, measured document by document on this computer, and how to raise it by linking your work across systems.</p></div>
          <div class="section-actions"><button class="btn primary" data-act="send-report" ${extra.evaluation ? "" : "disabled"}>Send report to Working Minds</button></div>
        </div>
        <div id="training-value"></div>
        <section class="section">
          <div class="section-head"><div><h2>What's in your data</h2><p>Measured on this computer. Every file is counted, including those we can't read yet.</p></div></div>
          <div class="grid-2">
            ${coverageCard()}
            ${T ? `<div class="card panel-card"><div class="card-head"><div><h3>Activity over time</h3><p>Message records per month, from timestamps inside your exports</p></div></div><div id="tl-host">${T.svg}</div></div>` : formatsCard()}
          </div>
          ${T ? formatsCard() : ""}
          ${languagesBlock()}
          ${collectionsTable()}
        </section>
      </div>
      <div class="privacy-foot reveal" style="animation-delay:600ms"><span class="status-dot"></span>${esc(map.privacy.statement)}</div>
    `;

    // ------------------------------------------------------------ tables
    function examinedOf(c) {
      const examined = (c.by_status.assessed || 0) + (c.by_status.partly_assessed || 0);
      return Math.round((examined * 100) / Math.max(1, c.logical_files - (c.by_status.excluded || 0)));
    }
    function meter(p, color) {
      return `<span class="cell-meter"><span style="width:${p}%;background:${color || "var(--teal)"}"></span></span>`;
    }
    function collectionsTable() {
      const rows = map.collections.map((c) => {
        const info = kindInfo(c);
        const s = sumSlack(c);
        const records = (c.slack || []).length ? s.message_records : c.structured_records;
        const p = examinedOf(c);
        return `<tr>
          <td><span class="tbl-badge" style="background:${info.color}">${esc(info.badge)}</span><b title="${esc(c.label)}">${esc(trunc(c.display_label || c.label, 44))}</b></td>
          <td>${esc(info.kind)}</td>
          <td class="num">${num(c.logical_files)}</td>
          <td class="num">${records ? num(records) : "—"}</td>
          <td class="num">${bytes(c.expanded_bytes)}</td>
          <td>${c.date_coverage ? `${esc(c.date_coverage.first)} → ${esc(c.date_coverage.last)}` : `<span class="muted">—</span>`}</td>
          <td class="num">${info.matched != null ? `${num(info.matched)} matched` : `${meter(p)}${p}%`}</td>
        </tr>`;
      }).join("");
      return `<div class="card table-card"><div class="card-head"><div><h3>Collections</h3><p>One row per file, folder or export you added</p></div></div>
        <div class="tbl-wrap"><table class="tbl">
          <thead><tr><th>Collection</th><th>Type</th><th class="num">Files</th><th class="num">Records</th><th class="num">Size</th><th>Dates in content</th><th class="num">Examined</th></tr></thead>
          <tbody>${rows}</tbody>
          <tfoot><tr><td><b>Total</b></td><td></td><td class="num">${num(t.logical_files)}</td><td class="num">${num(t.structured_records)}</td><td class="num">${bytes(t.expanded_bytes)}</td><td>${esc(sp.note)}</td><td class="num">${pct}%</td></tr></tfoot>
        </table></div></div>`;
    }
    function coverageCard() {
      const reasons = (cov.reasons || []).slice().sort((a, b) => b.files - a.files).slice(0, 4);
      const rows = STATUS.map(([k, l, c]) => {
        const v = cov.by_status[k] || 0;
        const p = Math.round((v * 1000) / totalCov) / 10;
        return `<tr class="${v ? "" : "zero"}"><td><i class="swatch" style="background:${c}"></i>${l}</td><td class="num">${num(v)}</td><td class="num">${meter(p, c)}${p}%</td></tr>`;
      }).join("");
      return `<div class="card panel-card"><div class="card-head"><div><h3>Coverage</h3><p>How far each file could be read</p></div></div>
        <table class="tbl compact"><thead><tr><th>Status</th><th class="num">Files</th><th class="num">Share</th></tr></thead><tbody>${rows}</tbody></table>
        ${reasons.length ? `<div class="reasons">${reasons.map((r) => `<div><b>${num(r.files)}</b><span>${esc(r.explanation)}</span></div>`).join("")}</div>` : ""}</div>`;
    }
    function formatsCard() {
      const by = new Map();
      map.collections.forEach((c) => (c.formats || []).forEach((f) => {
        const e = by.get(f.format) || { format: f.format, category: f.category, files: 0, bytes: 0, examined: 0 };
        e.files += f.files; e.bytes += f.bytes; e.examined += f.examined;
        by.set(f.format, e);
      }));
      const rows = [...by.values()].sort((a, b) => b.files - a.files).map((f) => {
        const p = Math.round((f.examined * 100) / Math.max(1, f.files));
        return `<tr><td><i class="swatch" style="background:${CAT[f.category] || CAT.Other}"></i>${esc(f.format)}</td><td>${esc(f.category)}</td><td class="num">${num(f.files)}</td><td class="num">${bytes(f.bytes)}</td><td class="num">${meter(p, CAT[f.category])}${p}%</td></tr>`;
      }).join("");
      return `<div class="card table-card"><div class="card-head"><div><h3>File formats</h3><p>Detected from file contents, not just names</p></div></div>
        <div class="tbl-wrap"><table class="tbl"><thead><tr><th>Format</th><th>Family</th><th class="num">Files</th><th class="num">Size</th><th class="num">Examined</th></tr></thead><tbody>${rows}</tbody></table></div></div>`;
    }
    function languagesBlock() {
      const l = map.languages;
      if (!l || !l.words_examined) return "";
      const LANG_COLORS = ["#187C75", "#2F6FB0", "#6D5BD0", "#C2577A", "#D9921F", "#5E8F3A", "#7B8AA0"];
      const top = l.rows.slice(0, 6);
      const colorOf = {};
      top.forEach((r, i) => (colorOf[r.code] = LANG_COLORS[i]));
      const otherWords = l.rows.slice(6).reduce((a, r) => a + r.words, 0);
      const pctOf = (w) => (l.words_examined ? Math.round((w * 1000) / l.words_examined) / 10 : 0);
      const row = (name, color, words, share, note) => `<tr>
          <td><span class="lang-dot" style="background:${color}"></span>${esc(name)}${note ? ` <span class="muted small">${esc(note)}</span>` : ""}</td>
          <td class="num">${num(words)}</td>
          <td class="share"><div class="share-bar"><span style="width:${share}%;background:${color}"></span></div><b>${Number(share).toFixed(1)}%</b></td>
        </tr>`;
      const overall = top.map((r) => row(r.language, colorOf[r.code], r.words, r.share_percent)).join("")
        + (otherWords ? row(`${l.rows.length - 6} other languages`, "#AEB8C6", otherWords, pctOf(otherWords)) : "")
        + (l.undetermined_words ? row("Too short to tell", "#DDE3EA", l.undetermined_words, l.undetermined_percent, "short replies, links, emoji") : "")
        + (l.code_or_data_words ? `<tr class="aside-row"><td><span class="lang-dot" style="background:#C9D3DF"></span>Code, data or placeholder text <span class="muted small">set aside, not counted</span></td><td class="num">${num(l.code_or_data_words)}</td><td class="share muted small">not part of the shares</td></tr>` : "");

      // by-collection matrix: one column per top language, then other and undetermined
      const cols = top.slice(0, 3);
      const matrix = l.by_collection.map((c) => {
        const cell = (w) => {
          const p = c.words_examined ? Math.round((w * 100) / c.words_examined) : 0;
          return `<td class="num">${w ? `${p < 1 ? "<1" : p}%` : "—"}</td>`;
        };
        const known = new Set(cols.map((x) => x.code));
        const others = c.rows.filter((r) => !known.has(r.code));
        const other = others.reduce((a, r) => a + r.words, 0);
        // Name the biggest language hiding in "Other", e.g. Hungarian.
        const otherCell = () => {
          if (!other) return `<td class="num">—</td>`;
          const p = c.words_examined ? Math.round((other * 100) / c.words_examined) : 0;
          const top = others[0];
          return `<td class="num">${p < 1 ? "<1" : p}%${top ? `<span class="other-lang">mostly ${esc(top.language)}</span>` : ""}</td>`;
        };
        return `<tr><td class="coll" title="${esc(c.label)}">${esc(displayOf(map, c.label))}<span class="muted small">${num(c.words_examined)} words of prose · ${esc(c.basis)}</span></td>
          ${cols.map((x) => cell((c.rows.find((r) => r.code === x.code) || {}).words || 0)).join("")}
          ${otherCell()}${cell(c.undetermined_words)}</tr>`;
      }).join("");

      const en = l.english_percent;
      const notes = [];
      if (l.unreadable_documents) notes.push(`${num(l.unreadable_documents)} PDF and Office ${l.unreadable_documents === 1 ? "document is" : "documents are"} not included yet: reading their text arrives in a later release`);
      if (l.sampled_documents) notes.push(`${num(l.sampled_documents)} long ${l.sampled_documents === 1 ? "document was" : "documents were"} sampled from the start (first 256 KB)`);
      if (l.code_or_data_files) notes.push(`${num(l.code_or_data_files)} code and data files (such as .js, .json, .csv and .log) were skipped`);
      notes.push("Slack bot and integration messages, code blocks and emoji codes are left out, so the mix reflects what people wrote");
      return `<div class="card lang-card">
        <div class="card-head"><div><h3>Languages</h3><p>Apparent language of the readable text, measured in words</p></div>
          ${en != null ? `<div class="lang-headline"><strong>${en}%</strong><span>of the readable text<br>is in English</span></div>` : ""}</div>
        <div class="lang-grid">
          <table class="lang-table">
            <thead><tr><th>Language</th><th class="num">Words</th><th>Share</th></tr></thead>
            <tbody>${overall}</tbody>
          </table>
          <div class="matrix-wrap">
            <table class="lang-table matrix">
              <thead><tr><th>By collection</th>${cols.map((x) => `<th class="num"><span class="lang-dot" style="background:${colorOf[x.code]}"></span>${esc(x.language)}</th>`).join("")}<th class="num">Other</th><th class="num">Unclear</th></tr></thead>
              <tbody>${matrix}</tbody>
            </table>
          </div>
        </div>
        <p class="lang-method"><b>How this is measured.</b> ${esc(l.method)}. ${esc(notes.join(". "))}. Detection on very short or mixed-language text is less certain, and a channel's name is never used as a language label.</p>
      </div>`;
    }


    // ------------------------------------------------------------ tooltips
    const tipEl = document.getElementById("tooltip");
    function bindTips(host, list) {
      if (!host) return;
      host.addEventListener("mousemove", (e) => {
        const el = e.target.closest("[data-tip]");
        if (!el) { tipEl.classList.remove("on"); return; }
        tipEl.innerHTML = list[+el.dataset.tip];
        const w = tipEl.offsetWidth, h = tipEl.offsetHeight;
        tipEl.style.left = clamp(e.clientX + 14, 8, innerWidth - w - 8) + "px";
        tipEl.style.top = clamp(e.clientY + 14, 8, innerHeight - h - 8) + "px";
        tipEl.classList.add("on");
      });
      host.addEventListener("mouseleave", () => tipEl.classList.remove("on"));
    }
    bindTips(root.querySelector("#map-svg-host"), M.tips);
    if (T) bindTips(root.querySelector("#tl-host"), T.tips);

    requestAnimationFrame(() => {
      root.querySelectorAll("[data-count]").forEach((el) => countUp(el, +el.dataset.count, el.dataset.fmt));
    });
    root.querySelector('[data-act="save"]').onclick = handlers.onSave;
    // Edit the company name in place; kept with the owner's other choices.
    root.querySelector('[data-act="edit-company"]').onclick = () => {
      const h = root.querySelector("#company-name");
      const src = root.querySelector("#company-src");
      h.innerHTML = `<input class="company-input" maxlength="80" value="${esc(company ? company.name : "")}" placeholder="Company name" aria-label="Company name">`;
      src.innerHTML = `<button class="btn primary small" data-co="save">Save</button> <button class="btn ghost small" data-co="cancel">Cancel</button>${plan.company ? ` <button class="link-btn" data-co="reset">Use detected name</button>` : ""}`;
      const input = h.querySelector("input");
      input.focus(); input.select();
      const done = (value) => { plan.company = value; handlers.onPlan && handlers.onPlan(plan); render(root, map, handlers, { ...extra, plan, banner: "" }); };
      src.querySelector('[data-co="save"]').onclick = () => done(input.value.trim() || null);
      src.querySelector('[data-co="cancel"]').onclick = () => render(root, map, handlers, { ...extra, plan, banner: "" });
      const reset = src.querySelector('[data-co="reset"]');
      if (reset) reset.onclick = () => done(null);
      input.onkeydown = (e) => { if (e.key === "Enter") done(input.value.trim() || null); if (e.key === "Escape") src.querySelector('[data-co="cancel"]').click(); };
    };
    // A platform bubble opens its card.
    root.querySelectorAll(".pm-node").forEach((g) => {
      const go = () => {
        const card = root.querySelector(`.pcard[data-plat="${g.dataset.plat}"]`);
        if (card) { card.scrollIntoView({ behavior: "smooth", block: "start" }); card.classList.add("flash"); setTimeout(() => card.classList.remove("flash"), 1600); }
      };
      g.onclick = go;
      g.onkeydown = (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); go(); } };
    });
    root.querySelector('[data-act="restart"]').onclick = handlers.onRestart;
    root.querySelector('[data-act="pack"]').onclick = handlers.onPack;
    root.querySelectorAll('[data-act="adddata"]').forEach((b) => (b.onclick = () => handlers.onAddData(b)));
    const steps = root.querySelector("#next-steps");
    const redraw = () => renderNextSteps(steps, map, plan, handlers, redraw, extra || {});
    renderTraining(root.querySelector("#training-value"), map, extra.evaluation || null, handlers, extra.evaluating || null);
    renderTvCard(root.querySelector("#tv-card"), map, extra.evaluation || null, extra.evaluating || null, handlers);
    root.querySelectorAll("[data-tab]").forEach((b) => (b.onclick = () => setTab(b.dataset.tab)));
    const send = root.querySelector('[data-act="send-report"]');
    if (send) send.onclick = () => handlers.onSendReport && handlers.onSendReport();
    root.querySelector('[data-act="addsource"]').onclick = (e) => openSourcePicker(e.currentTarget, map, plan, () => { handlers.onPlan(plan); redraw(); });
    redraw();
  }

  // ------------------------------------------------------------ next steps
  function normalisePlan(p) {
    p = p && typeof p === "object" ? p : {};
    return {
      added: Array.isArray(p.added) ? p.added : [],
      removed: Array.isArray(p.removed) ? p.removed : [],
      done: Array.isArray(p.done) ? p.done : [],
      kept: Array.isArray(p.kept) ? p.kept : [],
      company: typeof p.company === "string" && p.company.trim() ? p.company.trim().slice(0, 80) : null,
    };
  }
  const catalogName = (map, id) => ((map.source_catalog || []).find((s) => s.id === id) || {}).name || id;
  const displayOf = (map, label) => ((map.collections || []).find((c) => c.label === label) || {}).display_label || label;

  // ------------------------------------------------------------ platforms
  // One card per platform with the items (files) collected for it, their
  // stats and overlaps, then the platforms the data points to that aren't
  // collected yet.
  const PLATFORM_NAMES = { slack: "Slack", teams: "Microsoft Teams", discord: "Discord", email: "Email", google_drive: "Google Drive", google_other: "Google (other products)", github: "GitHub", gitlab: "GitLab", bitbucket: "Bitbucket" };
  const PLATFORM_ORDER = ["slack", "teams", "discord", "email", "google_drive", "google_other", "github", "gitlab", "bitbucket"];
  // Which tab is showing, kept across redraws.
  let activeTab = "map";
  // Item rows whose file tree is open, kept across redraws.
  const openTrees = new Set();

  function renderNextSteps(host, map, plan, handlers, redraw, extra) {
    const removed = new Set(plan.removed);
    const doneByHand = new Set(plan.done);
    const sources = map.sources || [];
    const cols = map.collections || [];
    const F = handlers.files || null;
    const save = () => { handlers.onPlan(plan); redraw(); };
    const markDone = (key) => () => { if (!plan.done.includes(key)) plan.done.push(key); save(); };
    const undoDone = (key) => () => { plan.done = plan.done.filter((k) => k !== key); save(); };
    const srcOf = (id) => sources.find((s) => s.id === id);
    const colOf = (label) => cols.find((c) => c.label === label);
    const pname = (id) => PLATFORM_NAMES[id] || catalogName(map, id);
    const nameOf = (label) => { const c = colOf(label); return c ? c.display_label || c.label : label; };
    const actions = []; // [label, fn, cls] referenced by data-act-i
    const act = (label, fn, cls) => { actions.push(fn); return `<button class="btn ${cls || "secondary"} small" data-act-i="${actions.length - 1}">${esc(label)}</button>`; };

    // Platforms with data in the map.
    const byPlat = new Map();
    cols.forEach((c) => (c.platforms || []).forEach((p) => { if (!byPlat.has(p)) byPlat.set(p, []); byPlat.get(p).push(c); }));
    const rank = (p) => { const i = PLATFORM_ORDER.indexOf(p); return i < 0 ? 99 : i; };
    const collected = [...byPlat.keys()].sort((a, b) => rank(a) - rank(b) || pname(a).localeCompare(pname(b)));
    const loose = cols.filter((c) => !(c.platforms || []).length);

    // Platforms the data points to that aren't collected.
    const weight = (s) => (s.distinct_targets || 0) * 3 + (s.links || 0) + (s.bot_messages || 0) / 2;
    const todos = sources
      .filter((s) => !byPlat.has(s.id) && !s.in_selection && !removed.has(s.id) && (s.links || s.app_installed || s.bot_messages))
      .sort((a, b) => weight(b) - weight(a))
      .map((s) => ({ key: s.id, name: s.name, tag: s.category, evidence: s.evidence, from: s.referenced_from || [], service: s.id, stat: s.links || s.bot_messages, statNoun: s.links ? (s.links === 1 ? "link" : "links") : "bot messages", remove: () => { plan.removed.push(s.id); save(); } }));
    plan.added.forEach((a, idx) => {
      const id = a.id || null;
      if (id && (byPlat.has(id) || todos.some((t) => t.key === id))) return;
      todos.push({ key: id || "custom:" + a.custom, name: id ? catalogName(map, id) : a.custom, tag: "Added by you", evidence: id ? "No links to it found in your data yet" : "A source you added", from: [], guide: id || "custom", remove: () => { plan.added.splice(idx, 1); save(); } });
    });
    const doneTodos = todos.filter((t) => doneByHand.has(t.key)).length;
    const total = collected.length + todos.length;
    const doneCount = collected.length + doneTodos;
    const pctDone = total ? Math.round((doneCount * 100) / total) : 100;

    // ---- item table (the files in a collection)
    function overlapBadge(c) {
      const l = c.linkage || {};
      const link = l.status === "unlinked" ? `<span class="ov unlinked" title="${esc(l.explanation)}">Not linked to your other data</span>`
        : l.status === "weak" ? `<span class="ov weak" title="${esc(l.explanation)}">Weak link</span>` : "";
      const ov = overlapOnly(c);
      return link && ov.includes('class="ov uniq"') ? link : link + ov;
    }
    function overlapOnly(c) {
      if (c.obsolete_given) return `<span class="ov dup" title="Everything in it is also in ${esc(nameOf(c.obsolete_given))}">Duplicate of ${esc(trunc(nameOf(c.obsolete_given), 34))}</span>`;
      const ovs = c.overlaps || [];
      const noun = (c.slack || []).length ? "messages" : "emails";
      const o = ovs.find((x) => x.share_percent >= 1);
      if (o) {
        const what = o.shared_messages ? `${num(o.shared_messages)} ${noun}` : `${num(o.identical_files)} identical files`;
        return `<span class="ov part" title="${esc(what)} also in ${esc(nameOf(o.with))}">${Math.round(o.share_percent)}% also in ${esc(trunc(nameOf(o.with), 28))}</span>`;
      }
      const m = ovs.find((x) => x.shared_messages > 0);
      if (m) return `<span class="ov part" title="The rest of this item is not elsewhere">${num(m.shared_messages)} ${noun} also in ${esc(trunc(nameOf(m.with), 26))}</span>`;
      return `<span class="ov uniq">Unique</span>`;
    }
    function recordsOf(c) {
      const s = sumSlack(c);
      if ((c.slack || []).length) return `${num(s.message_records)} messages`;
      if (c.discord) return `${num(c.discord.messages)} messages`;
      if (c.teams) return `${num(c.teams.messages)} messages${c.teams.meetings ? ` · ${num(c.teams.meetings)} meetings` : ""}`;
      const mail = (c.formats || []).filter((f) => f.category === "Email").length;
      if (!c.structured_records) return "—";
      return `${num(c.structured_records)} ${mail && (c.formats || []).length === mail ? "emails" : "records"}`;
    }
    function itemTable(list, service) {
      const rows = list.map((c) => {
        const info = kindInfo(c);
        const key = `${service || "other"}|${c.label}`;
        const open = openTrees.has(key);
        const left = (c.by_status && c.by_status.excluded) || 0;
        return `<tr class="it-row ${c.obsolete_given ? "is-dup" : ""}" data-key="${esc(key)}" data-label="${esc(c.label)}">
            <td><button class="it-name" data-tree aria-expanded="${open}" title="${esc(c.label)}"><span class="it-twist">${open ? "▾" : "▸"}</span><span class="tbl-badge" style="background:${info.color}">${esc(info.badge)}</span><span class="it-text"><b>${esc(c.display_label || c.label)}</b><small>${esc(info.kind)}${c.display_label && c.display_label !== c.label ? ` · ${esc(c.label)}` : ""}</small></span></button></td>
            <td class="num">${num(c.logical_files)}${left ? `<small class="muted"> (${num(left)} left out)</small>` : ""}</td>
            <td class="num">${bytes(c.expanded_bytes)}</td>
            <td>${c.date_coverage ? `${esc(c.date_coverage.first)} → ${esc(c.date_coverage.last)}` : `<span class="muted">—</span>`}</td>
            <td class="num">${recordsOf(c)}</td>
            <td>${overlapBadge(c)}</td>
            <td class="num">${F ? `<button class="btn ${c.obsolete_given ? "danger-solid" : "ghost danger"} small" data-remove-item title="Takes it out of this assessment and the data pack. Your file is not deleted.">${c.obsolete_given ? "Remove duplicate" : "Remove"}</button>` : ""}</td>
          </tr>
          ${open ? `<tr class="tree-row"><td colspan="7"><div class="si-tree" data-tree-for="${esc(c.label)}"></div></td></tr>` : ""}`;
      }).join("");
      return `<div class="tbl-wrap"><table class="tbl items-tbl">
        <thead><tr><th>Item</th><th class="num">Files</th><th class="num">Size</th><th>Dates</th><th class="num">Content</th><th>Overlap</th><th></th></tr></thead>
        <tbody>${rows}</tbody></table></div>`;
    }
    // Items that only link to a platform, with how many of their files do.
    function linkedFrom(labels, service, summaryText, tip) {
      const list = labels.filter(colOf);
      if (!list.length) return "";
      return `<details class="pc-links"><summary${tip ? ` title="${esc(tip)}"` : ""}>${esc(summaryText)}</summary>
        ${list.map((l) => {
          const n = ((extra.linkCounts || {})[l] || {})[service] || 0;
          return `<div class="lf-row" data-label="${esc(l)}" data-service="${esc(service)}"><span><b>${esc(trunc(nameOf(l), 60))}</b>${n ? ` <span class="muted">· links in ${num(n)} ${n === 1 ? "file" : "files"}</span>` : ""}</span>
            ${F ? `<button class="btn ghost small" data-linking>Show files ▸</button>` : ""}<div class="si-tree" hidden></div></div>`;
        }).join("")}</details>`;
    }

    // ---- platform cards
    function platformCard(p) {
      const list = byPlat.get(p);
      const s = srcOf(p);
      const files = list.reduce((a, c) => a + c.logical_files, 0);
      const size = list.reduce((a, c) => a + c.expanded_bytes, 0);
      const firsts = list.map((c) => c.date_coverage && c.date_coverage.first).filter(Boolean).sort();
      const lasts = list.map((c) => c.date_coverage && c.date_coverage.last).filter(Boolean).sort();
      const dups = list.filter((c) => c.obsolete_given).length;
      const facts = [`${num(list.length)} ${list.length === 1 ? "item" : "items"}`, `${num(files)} files`, bytes(size)];
      const notes = [];
      let partial = false;
      if (firsts.length) facts.push(`${firsts[0]} → ${lasts[lasts.length - 1]}`);
      if (p === "slack") { const m = list.reduce((a, c) => a + (c.slack || []).reduce((x, s) => x + s.message_records - (s.records_also_in_other_exports || 0), 0), 0); if (m) facts.push(`${num(m)} messages, each counted once`); }
      if (p === "discord") {
        const d = list.filter((c) => c.discord).map((c) => c.discord);
        const m = d.reduce((a, x) => a + x.messages - x.messages_also_in_other_items, 0);
        const servers = d.reduce((a, x) => a + x.servers, 0);
        const chats = d.reduce((a, x) => a + x.channels + x.direct_messages + x.group_messages, 0);
        const att = d.reduce((a, x) => a + x.attachments_referenced, 0);
        if (m) facts.push(`${num(m)} messages, each counted once`, `${num(servers)} ${servers === 1 ? "server" : "servers"}`, `${num(chats)} channels and DMs`);
        if (att) facts.push(`${num(att)} attachments linked, not included`);
        if (d.some((x) => x.own_messages_only)) {
          partial = true;
          notes.push(`<div class="pc-note warn"><span><b>Only one person's messages.</b> A Discord data package holds only the messages written by the account that requested it; other people's replies aren't in it. For whole servers, an admin can export channels with a tool such as DiscordChatExporter.</span><span class="pc-note-act">${act("How to export", () => handlers.onGuide("export:discord"))}</span></div>`);
        }
      }
      if (p === "teams" && map.teams_files && map.teams_files.referenced) {
        const tf = map.teams_files, miss = tf.referenced - tf.found;
        const manual = miss > 0 && doneByHand.has("teams-files");
        if (miss <= 0) notes.push(`<div class="pc-note ok">All ${num(tf.referenced)} files linked from Teams messages are in your map.</div>`);
        else {
          if (!manual) partial = true;
          notes.push(`<div class="pc-note ${manual ? "" : "warn"}"><span>${manual ? "You marked the linked files as done." : tf.found
            ? `<b>${num(tf.found)} of ${num(tf.referenced)} files shared in Teams are in your map.</b> The rest live in SharePoint or OneDrive: add that folder here, or ask your Microsoft 365 admin to include linked files in the export.`
            : `<b>${num(tf.referenced)} files are linked from Teams messages but not in your map.</b> They live in SharePoint or OneDrive: add that folder here, or ask your Microsoft 365 admin to include linked files in the eDiscovery export.`}</span>
            <span class="pc-note-act">${manual ? act("Undo", undoDone("teams-files"), "ghost") : act("Mark as done", markDone("teams-files"), "ghost")}</span></div>`);
        }
      }
      if (p === "teams") {
        const m = list.reduce((a, c) => a + (c.teams ? c.teams.messages - c.teams.messages_also_in_other_items : 0), 0);
        const ch = list.reduce((a, c) => a + (c.teams ? c.teams.channels + c.teams.chats : 0), 0);
        if (m) facts.push(`${num(m)} messages, each counted once`, `${num(ch)} channels and chats`);
        if (list.some((c) => c.teams && !c.teams.messages)) notes.push(`<div class="pc-note warn"><span>An eDiscovery export in Outlook PST files is counted but can't be read yet. Ask your admin for the HTML or JSON export instead.</span><span class="pc-note-act">${act("How to export", () => handlers.onGuide("export:teams"))}</span></div>`);
      }
      if (p === "slack" && map.slack_files) {
        const sf = map.slack_files, totalF = sf.images + sf.documents, got = sf.images_found + sf.documents_found;
        if (totalF && got < totalF) {
          const manual = doneByHand.has("slack-files");
          partial = !manual;
          notes.push(`<div class="pc-note ${manual ? "" : "warn"}"><span>${manual ? "You marked the shared files as done." : `<b>${num(totalF - got)} shared files still missing</b> · ${num(got)} of ${num(totalF)} matched to messages`}</span>
            <span class="pc-note-act">${manual ? act("Undo", undoDone("slack-files"), "ghost") : act("How to get these", () => handlers.onGuide("slack_images")) + act("Mark as done", markDone("slack-files"), "ghost")}</span></div>`);
        } else if (totalF) notes.push(`<div class="pc-note ok">All ${num(totalF)} shared files are in your map.</div>`);
      }
      if (s && s.resolved_targets != null && s.distinct_targets) {
        const others = s.other_repositories || 0;
        if (others) { partial = true; notes.push(`<div class="pc-note warn"><span><b>${num(others)} other linked ${others === 1 ? "repository" : "repositories"} not added</b> · ${num(s.resolved_targets)} of ${num(s.distinct_targets)} linked pages are in what you added</span><span class="pc-note-act">${act("How to export", () => handlers.onGuide("export:" + p))}</span></div>`); }
        if (s.snapshot_only) notes.push(`<div class="pc-note"><span>Added as a download, so there is no commit history or pull requests. A clone (git clone) includes them.</span></div>`);
      }
      if (dups) notes.push(`<div class="pc-note warn"><span><b>${num(dups)} ${dups === 1 ? "item looks" : "items look"} like a duplicate</b> of another item. Removing ${dups === 1 ? "it" : "them"} loses nothing.</span></div>`);
      const own = new Set(list.map((c) => c.label));
      const linkers = s ? (s.referenced_from || []).filter((l) => !own.has(l)) : [];
      // Links to the platform (e.g. discord.com), not necessarily to this export.
      const linkText = s && s.links && linkers.length ? `Your other data mentions ${pname(p)}: ${num(s.links)} ${s.links === 1 ? "link" : "links"} in ${num(linkers.length)} ${linkers.length === 1 ? "item" : "items"}` : "";
      const suspect = list.some((c) => c.linkage && c.linkage.status === "unlinked");
      return `<div class="pcard" data-plat="${esc(p)}">
        <div class="pc-head">
          <div class="pc-title"><h3>${esc(pname(p))}</h3><span class="pill ${suspect ? "check" : partial ? "partial" : "ok"}">${suspect ? "Check this data" : partial ? "Partly collected" : "Collected ✓"}</span></div>
          <div class="pc-facts">${facts.map(esc).join(" · ")}</div>
        </div>
        ${notes.join("")}
        ${itemTable(list, p)}
        ${linkText ? linkedFrom(linkers, p, linkText, `Links to ${pname(p)} as a platform, not necessarily to the data in this card.`) : ""}
      </div>`;
    }
    function todoRow(t) {
      const manual = doneByHand.has(t.key);
      return `<div class="todo ${manual ? "done" : ""}">
        <div class="todo-main">
          <div class="todo-title">${manual ? "✓ " : ""}${esc(t.name)} <span class="step-tag">${esc(t.tag)}</span>${manual ? ` <span class="step-tag by-you">marked done</span>` : ""}</div>
          <div class="todo-ev">${esc(t.evidence)}</div>
          ${t.service && t.from.length ? linkedFrom(t.from, t.service, `Linked from ${t.from.length === 1 ? "1 item" : `${num(t.from.length)} items`} · show which files`) : ""}
        </div>
        ${t.stat ? `<div class="step-stat"><strong>${num(t.stat)}</strong><span>${esc(t.statNoun)}</span></div>` : "<div></div>"}
        <div class="step-actions">${manual ? act("Undo", undoDone(t.key), "ghost") : act("How to export", () => handlers.onGuide("export:" + (t.guide || t.key))) + act("Mark as done", markDone(t.key), "ghost") + act("Remove", t.remove, "ghost")}</div>
      </div>`;
    }

    const removedSources = plan.removed.map((id) => sources.find((s) => s.id === id)).filter((s) => s && !s.in_selection);
    const other = (map.preparation_priorities || []).filter((p) => !/^(Add the original attachments|If you hold the repositories)/.test(p));
    const kept = new Set(plan.kept || []);
    const unlinked = cols.filter((c) => c.linkage && c.linkage.status === "unlinked" && !kept.has(c.label));
    const warnings = unlinked.map((c) => `<div class="link-warn" data-label="${esc(c.label)}">
        <div class="lw-icon" aria-hidden="true">!</div>
        <div class="lw-text"><b>${esc(c.display_label || c.label)} doesn't seem connected to your other data</b><span>${esc(c.linkage.explanation)} If it isn't yours, remove it so it doesn't inflate your totals or end up in the data pack.</span></div>
        <div class="lw-act"><button class="btn ghost small" data-lw-show>Show</button><button class="btn ghost small" data-lw-keep>Keep it</button>${F ? `<button class="btn danger-solid small" data-lw-remove>Remove</button>` : ""}</div>
      </div>`).join("");
    host.innerHTML = `
      ${warnings}
      ${total ? `<div class="steps-progress card ${doneCount < total ? "" : "all"}">
        <div class="sp-text"><strong>${num(collected.length)} of ${num(total)} platforms collected</strong><span>${todos.length - doneTodos ? `${num(todos.length - doneTodos)} more your data points to` : "Everything your data points to is accounted for"}</span></div>
        <div class="sp-bar"><span style="width:${pctDone}%"></span></div>
      </div>` : ""}
      ${collected.map(platformCard).join("")}
      ${loose.length ? `<div class="pcard" data-plat="other"><div class="pc-head"><div class="pc-title"><h3>Other files</h3></div><div class="pc-facts">${num(loose.length)} ${loose.length === 1 ? "item" : "items"} not tied to a platform</div></div>${itemTable(loose, null)}</div>` : ""}
      ${todos.length ? `<div class="card todo-card"><div class="todo-head"><h3>Not collected yet</h3><p>Platforms your data links to or mentions. Add their exports, or remove any that aren't yours.</p></div>${todos.map(todoRow).join("")}</div>` : ""}
      ${other.length ? `<div class="step-other card"><b>Also worth doing</b><ul>${other.map((o) => `<li>${esc(o)}</li>`).join("")}</ul></div>` : ""}
      ${removedSources.length ? `<div class="removed-line">${num(removedSources.length)} removed: ${removedSources.map((s) => `<button class="link-btn" data-restore="${esc(s.id)}">${esc(s.name)} ↺</button>`).join(" ")}</div>` : ""}
      ${(extra.removedItems || []).length ? `<div class="removed-line">Taken out of this assessment: ${extra.removedItems.map((r) => `<button class="link-btn" data-restore-item="${esc(r.path)}" title="Put back">${esc(r.name)} ↺</button>`).join(" ")}</div>` : ""}`;

    host.querySelectorAll("[data-act-i]").forEach((b) => (b.onclick = () => actions[+b.dataset.actI]()));
    host.querySelectorAll(".link-warn").forEach((w) => {
      const label = w.dataset.label;
      w.querySelector("[data-lw-show]").onclick = () => {
        const row = [...host.querySelectorAll(".it-row")].find((r) => r.dataset.label === label);
        if (row) { row.scrollIntoView({ behavior: "smooth", block: "center" }); row.classList.add("flash"); setTimeout(() => row.classList.remove("flash"), 1600); }
      };
      w.querySelector("[data-lw-keep]").onclick = () => { if (!plan.kept.includes(label)) plan.kept.push(label); save(); };
      const rm = w.querySelector("[data-lw-remove]");
      if (rm && F) confirmClick(rm, "Confirm remove", () => F.removeItem(label, nameOf(label)));
    });
    host.querySelectorAll("[data-restore]").forEach((b) => (b.onclick = () => { plan.removed = plan.removed.filter((id) => id !== b.dataset.restore); save(); }));
    host.querySelectorAll("[data-tree]").forEach((b) => (b.onclick = () => {
      const k = b.closest("tr").dataset.key;
      if (openTrees.has(k)) openTrees.delete(k); else openTrees.add(k);
      redraw();
    }));
    if (!F) return;
    host.querySelectorAll("[data-restore-item]").forEach((b) => (b.onclick = () => F.restoreItem(b.dataset.restoreItem, b.textContent.replace(" ↺", ""))));
    host.querySelectorAll(".it-row").forEach((row) => {
      const label = row.dataset.label;
      confirmClick(row.querySelector("[data-remove-item]"), "Confirm remove", () => F.removeItem(label, nameOf(label)));
    });
    host.querySelectorAll("[data-tree-for]").forEach((el) => mountTree(el, el.dataset.treeFor, null, false));
    host.querySelectorAll(".lf-row").forEach((row) => {
      const btn = row.querySelector("[data-linking]");
      const tree = row.querySelector(".si-tree");
      btn.onclick = () => {
        if (!tree.hidden) { tree.hidden = true; btn.textContent = "Show files ▸"; return; }
        tree.hidden = false; btn.textContent = "Hide files ▾";
        mountTree(tree, row.dataset.label, row.dataset.service, true);
      };
    });

    // ---- file tree inside an item
    function mountTree(tree, label, service, linkingFirst) {
      const show = linkingFirst && service ? showLinking : showTree;
      show();
      async function showTree() {
        tree.innerHTML = `<div class="muted small">Loading…</div>`;
        try {
          const entries = await F.list(label, null, service);
          tree.innerHTML = (service ? `<div class="ft-switch"><span class="muted small">All files</span><button class="link-btn" data-linking-only>Show only files with links</button></div>` : "") +
            `<ul class="ft">${entries.length ? entries.map((e) => entryRow(e)).join("") : `<li class="muted small">No files.</li>`}</ul>`;
          bindTree(tree);
          const sw = tree.querySelector("[data-linking-only]");
          if (sw) sw.onclick = showLinking;
        } catch (e) { tree.innerHTML = `<div class="muted small">Could not list files: ${esc(String(e))}</div>`; }
      }
      async function showLinking() {
        tree.innerHTML = `<div class="muted small">Loading…</div>`;
        try {
          const entries = await F.linking(label, service);
          const svc = pname(service);
          tree.innerHTML = `<div class="ft-switch"><span class="muted small">Files with links to ${esc(svc)}, most first</span><button class="link-btn" data-all>Show all files</button></div>
            <ul class="ft">${entries.length ? entries.map((e) => entryRow(e, true)).join("") : `<li class="muted small">None of its files link to ${esc(svc)}.</li>`}</ul>`;
          bindTree(tree);
          tree.querySelector("[data-all]").onclick = showTree;
        } catch (e) { tree.innerHTML = `<div class="muted small">Could not list files: ${esc(String(e))}</div>`; }
      }
      function entryRow(e, flat) {
        const out = e.excluded_files > 0 && e.excluded_files >= e.files;
        const name = flat ? e.path.split("/").slice(1).join(" / ") : e.name;
        const meta = [e.folder ? `${num(e.files)} ${e.files === 1 ? "file" : "files"}` : e.format, bytes(e.bytes)];
        if (e.links) meta.push(`${num(e.links)} ${e.links === 1 ? "link" : "links"}`);
        if (e.folder && e.excluded_files && !out) meta.push(`${num(e.excluded_files)} left out`);
        const action = e.excluded_here ? `<button class="link-btn" data-restore-path="${esc(e.path)}">Put back</button>`
          : out ? `<span class="muted small">left out</span>`
          : `<button class="link-btn danger" data-exclude-path="${esc(e.path)}">Remove</button>`;
        return `<li class="ft-row ${out ? "out" : ""}" data-path="${esc(e.path)}">
          <div class="ft-line">${e.folder && !flat ? `<button class="ft-twist" aria-expanded="false" aria-label="Open ${esc(e.name)}">▸</button>` : `<span class="ft-dot"></span>`}
            <span class="ft-name" title="${esc(e.path)}">${esc(name)}</span><span class="ft-meta">${meta.filter(Boolean).map(esc).join(" · ")}</span>${action}</div>
          ${e.folder && !flat ? `<ul class="ft" hidden></ul>` : ""}
        </li>`;
      }
      function bindTree(el) {
        el.querySelectorAll(".ft-twist:not([data-bound])").forEach((t) => {
          t.dataset.bound = "1";
          t.onclick = async () => {
            const li = t.closest(".ft-row");
            const sub = li.querySelector(":scope > ul");
            if (t.getAttribute("aria-expanded") === "true") { sub.hidden = true; t.setAttribute("aria-expanded", "false"); t.textContent = "▸"; return; }
            t.setAttribute("aria-expanded", "true"); t.textContent = "▾"; sub.hidden = false;
            if (!sub.dataset.loaded) {
              sub.innerHTML = `<li class="muted small">Loading…</li>`;
              const kids = await F.list(label, li.dataset.path, service);
              sub.dataset.loaded = "1";
              sub.innerHTML = kids.map((k) => entryRow(k)).join("");
              bindTree(sub);
            }
          };
        });
        el.querySelectorAll("[data-exclude-path]:not([data-bound])").forEach((b) => {
          b.dataset.bound = "1";
          confirmClick(b, "Confirm", () => F.exclude(b.dataset.excludePath, b.dataset.excludePath.split("/").pop()));
        });
        el.querySelectorAll("[data-restore-path]:not([data-bound])").forEach((b) => {
          b.dataset.bound = "1";
          b.onclick = () => F.restore(b.dataset.restorePath, b.dataset.restorePath.split("/").pop());
        });
      }
    }
  }

  // ------------------------------------------------------------ training value
  const compact = (n) => {
    n = n || 0;
    if (n >= 1e9) return (n / 1e9).toFixed(n >= 1e10 ? 0 : 1).replace(/\.0$/, "") + "B";
    if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e7 ? 0 : 1).replace(/\.0$/, "") + "M";
    if (n >= 1e3) return (n / 1e3).toFixed(n >= 1e4 ? 0 : 1).replace(/\.0$/, "") + "K";
    return num(n);
  };
  const KIND_COLOR = { conversation: "#6D5BD0", email: "#C4577A", document: "#2F6FB0", code: "#5E6E84", structured: "#B7C2CF", machine: "#DDE3EA" };

  /** Rough time for the evaluation, from the size of the readable content. */
  function evaluationEstimate(map) {
    let bytes = 0;
    (map.collections || []).forEach((c) => (c.formats || []).forEach((f) => {
      if (["Documents", "Email", "Structured data", "Tables"].includes(f.category) && !["PDF", "Office document", "OpenDocument file"].includes(f.format)) bytes += f.bytes;
    }));
    const secs = bytes / 40e6 + 5; // about 40 MB/s on a typical laptop
    if (secs < 60) return "under a minute";
    if (secs < 180) return "a minute or two";
    return `about ${Math.round(secs / 60)} minutes`;
  }

  function renderTraining(host, map, ev, handlers, state) {
    // The platform a row belongs to: email rows under Email, documents
    // under the item's document platform, chat under its chat platform.
    const platformOf = (label, kind) => {
      const c = (map.collections || []).find((x) => x.label === label);
      const ps = (c && c.platforms) || [];
      const pick = kind === "email" && ps.includes("email") ? "email"
        : ps.find((p) => p !== "email") || ps[0];
      return pick ? PLATFORM_NAMES[pick] || catalogName(map, pick) : "Other files";
    };
    const nameOf = (label) => { const c = (map.collections || []).find((x) => x.label === label); return c ? c.display_label || c.label : label; };
    if (state && state.running) {
      const p = state.progress || {};
      host.innerHTML = `<div class="card tv-card tv-running">
        <div class="adding-ring" aria-hidden="true"></div>
        <div class="tv-run-text"><b>Evaluating training value…</b>
          <span>${num(p.documents || 0)} documents measured · ${bytes(p.bytes_read || 0)} read${p.collection ? ` · reading ${esc(trunc(nameOf(p.collection) || p.collection, 40))}` : ""}</span>
          <small class="muted">Everything stays on this computer. You can keep using the map.</small></div>
        <button class="btn ghost small" data-tv="stop">Stop</button></div>`;
      host.querySelector('[data-tv="stop"]').onclick = (e) => { e.currentTarget.disabled = true; e.currentTarget.textContent = "Stopping…"; handlers.onStopEvaluate && handlers.onStopEvaluate(); };
      return;
    }
    if (!ev) {
      host.innerHTML = `<div class="card tv-card tv-intro">
        <div><b>How much of this is useful for training AI models?</b>
          <p>Working Minds reads every message, email and document in full, on this computer, and measures how much is written by people, how good the text is, how much is duplicated and how much predates AI-written text. It also finds work linked across your systems (a conversation, its ticket, the code change and the build) and tells you what to add to link more. It takes ${esc(evaluationEstimate(map))} for your data.</p></div>
        <button class="btn primary" data-tv="run">Evaluate training value</button></div>`;
      host.querySelector('[data-tv="run"]').onclick = () => handlers.onEvaluate && handlers.onEvaluate();
      return;
    }
    const t = ev.totals;
    const pct = (a, b) => (b ? Math.round((a * 100) / b) : 0);
    const rating = { high: "High", medium: "Medium", low: "Low" }[ev.rating] || ev.rating;
    // Rows grouped by platform, biggest usable first.
    const groups = new Map();
    (ev.rows || []).forEach((r) => { const p = platformOf(r.collection, r.kind); if (!groups.has(p)) groups.set(p, []); groups.get(p).push(r); });
    const order = [...groups.entries()].sort((a, b) => b[1].reduce((x, r) => x + r.usable_tokens, 0) - a[1].reduce((x, r) => x + r.usable_tokens, 0));
    const dots = (n) => `<span class="tv-dots v${n}" title="${["", "Low", "Medium", "High"][n]} value">${"●".repeat(n)}${"○".repeat(3 - n)}</span>`;
    const years = (r) => (r.first_year ? (r.first_year === r.last_year ? `${r.first_year}` : `${r.first_year}–${r.last_year}`) : "—");
    const rows = order.map(([p, list]) => {
      const usable = list.reduce((x, r) => x + r.usable_tokens, 0);
      return `<tr class="tv-group"><td colspan="7"><b>${esc(p)}</b><span class="muted"> · ${compact(usable)} usable tokens</span></td></tr>` +
        list.map((r) => `<tr>
          <td><div class="tv-name"><span><span class="tv-kind" style="background:${KIND_COLOR[r.kind] || "#B7C2CF"}"></span>${esc(r.kind_label)}</span><small class="muted">${esc(trunc(nameOf(r.collection), 46))}</small></div></td>
          <td class="num">${num(r.documents)}</td>
          <td class="num">${compact(r.tokens)}</td>
          <td class="num"><b>${compact(r.usable_tokens)}</b></td>
          <td>${esc(years(r))}</td>
          <td>${dots(r.value_band)}</td>
          <td class="tv-why">${esc(r.reason)}</td></tr>`).join("");
    }).join("");
    // Usable tokens by kind.
    const byKind = new Map();
    (ev.rows || []).forEach((r) => byKind.set(r.kind, { label: r.kind_label, v: (byKind.get(r.kind) || { v: 0 }).v + r.usable_tokens }));
    const kinds = [...byKind.entries()].filter(([, k]) => k.v > 0).sort((a, b) => b[1].v - a[1].v);
    const maxK = Math.max(1, ...kinds.map(([, k]) => k.v));
    const bars = kinds.map(([kind, k]) => `<div class="tv-bar"><span class="tv-bar-label">${esc(k.label)}</span><span class="tv-bar-track"><span style="width:${(k.v * 100) / maxK}%;background:${KIND_COLOR[kind]}"></span></span><span class="tv-bar-val">${compact(k.v)}</span></div>`).join("");
    // Other ways to raise the value (housekeeping such as duplicates and
    // missing Slack files lives on the Inventory tab).
    const tips = [];
    const gapIds = new Set(((ev.episodes || {}).gaps || []).map((g) => g.id));
    const gitGap = ["github", "bitbucket", "gitlab", "git"].some((g) => gapIds.has(g));
    if (!gitGap && (map.sources || []).some((s) => s.snapshot_only)) tips.push("Clone code repositories with their history (git clone): commit messages link conversations to the code they led to.");
    if (t.tokens && t.machine_tokens * 4 > t.tokens) tips.push(`${pct(t.machine_tokens, t.tokens)}% of the text is automated (bots, notifications, newsletters). Human conversation is what buyers value.`);
    if (ev.not_evaluated.pdf_and_office_files) tips.push(`${num(ev.not_evaluated.pdf_and_office_files)} PDF and Office files aren't evaluated yet; a later release reads their text.`);
    const todo = (map.sources || []).filter((s) => !s.in_selection && (s.links || 0) > 50 && !gapIds.has(s.id)).slice(0, 2);
    if (todo.length) tips.push(`Add ${todo.map((s) => esc(s.name)).join(" and ")}: your data links to ${todo.length === 1 ? "it" : "them"} often, so ${todo.length === 1 ? "its" : "their"} exports would extend these conversations.`);
    host.innerHTML = `
      ${ev.incomplete ? `<div class="pc-note warn"><span><b>Stopped before the end.</b> These numbers cover only what was read.</span></div>` : ""}
      <div class="card tv-card">
        <div class="tv-head">
          <div class="tv-headline">
            <span class="pill tv-${esc(ev.rating)}">${esc(rating)} training value</span>
            <h3>About <b>${compact(t.usable_tokens)}</b> usable tokens of human-written text</h3>
            <p>${esc(ev.rating_reason)} ${t.conversation_tokens ? `Including ${compact(t.conversation_tokens)} of multi-turn conversation. ` : ""}${t.tokens ? `${pct(t.pre_llm_tokens, t.tokens)}% was written before 2023.` : ""}</p>
          </div>
          <div class="tv-figures">
            <div><strong>${compact(t.tokens)}</strong><span>tokens in total</span></div>
            <div><strong>${pct(t.usable_tokens, t.tokens)}%</strong><span>usable</span></div>
            <div><strong>${compact(t.duplicate_tokens)}</strong><span>duplicated, not counted</span></div>
            <div><strong>${compact(t.low_quality_tokens + t.machine_tokens)}</strong><span>low quality or automated</span></div>
          </div>
        </div>
        ${bars ? `<div class="tv-bars"><div class="tv-sub">Usable tokens by kind</div>${bars}</div>` : ""}
        <div class="tbl-wrap"><table class="tbl tv-tbl">
          <thead><tr><th>Kind of data</th><th class="num">Documents</th><th class="num">Tokens</th><th class="num">Usable</th><th>Years</th><th>Value</th><th>Why</th></tr></thead>
          <tbody>${rows}</tbody></table></div>
        <div class="tv-foot"><span class="muted small">${esc(ev.method)} Evaluated in ${num(ev.seconds)}s.</span><button class="btn ghost small" data-tv="run">Evaluate again</button></div>
      </div>
      ${episodesCard(ev.episodes, tips)}`;
    host.querySelector('[data-tv="run"]').onclick = () => handlers.onEvaluate && handlers.onEvaluate();
  }

  /** What to add to link more work, in the owner's words. */
  function gapLine(g) {
    const n = num(g.records);
    const t = num(g.distinct_targets);
    let what;
    if (g.id === "ci") what = `${n} commits have no build or test results.`;
    else if (["github", "bitbucket", "gitlab", "git"].includes(g.id)) what = `${n} messages and documents link to ${esc(g.name)} pull requests or commits (${t}), but the repository's history isn't here.`;
    else if (g.id === "jira" || g.id === "linear") what = `${n} messages, emails and commits mention ${t} ${esc(g.name)} tickets you haven't added.`;
    else what = `${n} messages and documents link to ${t} ${esc(g.name)} pages or items you haven't added.`;
    return `<li><b>Add ${esc(g.name)}</b> <span>${what}</span><small class="muted">${esc(g.advice)}</small></li>`;
  }

  /** Linked work ("work trajectories"): one piece of work rebuilt across systems. */
  function episodesCard(e, tips) {
    if (!e) return "";
    tips = tips || [];
    const gaps = (e.gaps || []).slice(0, 3);
    const total = e.complete + e.partial + e.linked_discussion;
    const seg = (n, cls, label) => (n ? `<span class="ep-seg ${cls}" style="flex:${n}" title="${label}: ${num(n)}"></span>` : "");
    const pairs = (e.system_pairs || []).slice(0, 3).map((p) => `${esc(p.a_name)} ↔ ${esc(p.b_name)} (${num(p.episodes)})`).join(" · ");
    const sources = [
      e.commits ? `${num(e.commits)} commits` : "",
      e.tickets ? `${num(e.tickets)} tickets` : "",
      e.ci_logs ? `${num(e.ci_logs)} build logs (${num(e.ci_passed)} passed, ${num(e.ci_failed)} failed)` : "",
    ].filter(Boolean).join(" · ");
    const head = e.episodes
      ? `<h3><b>${num(e.episodes)}</b> pieces of work linked across your systems</h3>`
      : `<h3>No linked work found yet</h3>`;
    return `<div class="card tv-card ep-card">
      <div class="tv-head">
        <div class="tv-headline">
          <div class="tv-sub">Raise the value · linked work</div>
          ${head}
          <p>Working Minds joins the records of one piece of work: the conversation about it, the ticket, the code change, the document, and the build that tested it. Records are joined only when they cite the same ticket, pull request, commit, document or email thread. Linked work that shows what was done and how it turned out is the most valuable kind of training data.</p>
        </div>
        ${e.episodes ? `<div class="tv-figures">
          <div><strong>${num(e.complete)}</strong><span>complete: work and its result</span></div>
          <div><strong>${num(e.partial)}</strong><span>partial</span></div>
          <div><strong>${num(e.linked_discussion)}</strong><span>linked conversations</span></div>
          <div><strong>${num(e.multi_system)}</strong><span>across two or more systems</span></div>
        </div>` : ""}
      </div>
      ${total ? `<div class="ep-bar" aria-label="Linked work by completeness">${seg(e.complete, "c", "Complete")}${seg(e.partial, "p", "Partial")}${seg(e.linked_discussion, "d", "Linked conversations")}</div>
        <div class="ep-legend muted small"><span><i class="c"></i>Complete</span><span><i class="p"></i>Partial</span><span><i class="d"></i>Linked conversations</span></div>` : ""}
      ${pairs || sources ? `<p class="muted small ep-meta">${pairs ? `Most linked: ${pairs}` : ""}${pairs && sources ? "<br>" : ""}${sources ? `Read: ${sources}` : ""}</p>` : ""}
      ${gaps.length ? `<div class="step-other ep-gaps"><b>Link more of your work</b><ul>${gaps.map(gapLine).join("")}</ul></div>`
        : e.episodes ? "" : `<p class="muted small">Your items don't cite each other's tickets, pull requests, commits or documents yet. Adding a ticket tracker, repositories with history or build logs lets Working Minds join conversations to the work they led to.</p>`}
      ${tips.length ? `<div class="step-other ep-gaps"><b>More ways to raise the value</b><ul>${tips.map((x) => `<li>${x}</li>`).join("")}</ul></div>` : ""}
    </div>`;
  }

  /** Two-step button: the first click asks, the second acts. */
  function confirmClick(btn, ask, act) {
    if (!btn) return;
    const label = btn.textContent;
    let timer = null;
    btn.onclick = (e) => {
      e.stopPropagation();
      if (btn.dataset.armed) { clearTimeout(timer); act(); return; }
      btn.dataset.armed = "1";
      btn.textContent = ask;
      btn.classList.add("armed");
      timer = setTimeout(() => { delete btn.dataset.armed; btn.textContent = label; btn.classList.remove("armed"); }, 4000);
    };
  }

  function openSourcePicker(anchor, map, plan, onChange) {
    const pop = document.getElementById("source-picker");
    const taken = new Set([
      ...(map.sources || []).filter((s) => s.in_selection || !plan.removed.includes(s.id)).map((s) => s.id),
      ...plan.added.map((a) => a.id).filter(Boolean),
    ]);
    const choices = (map.source_catalog || []).filter((s) => !taken.has(s.id));
    const r = anchor.getBoundingClientRect();
    pop.style.top = r.bottom + 6 + "px";
    pop.style.left = Math.max(12, Math.min(innerWidth - 340, r.right - 320)) + "px";
    pop.hidden = false;
    const input = pop.querySelector("input");
    const list = pop.querySelector(".picker-list");
    input.value = "";
    const draw = () => {
      const q = input.value.trim().toLowerCase();
      const hits = choices.filter((s) => !q || s.name.toLowerCase().includes(q) || s.category.toLowerCase().includes(q)).slice(0, 8);
      list.innerHTML = hits.map((s) => `<button data-id="${esc(s.id)}"><b>${esc(s.name)}</b><small>${esc(s.category)}</small></button>`).join("")
        + (q && !hits.some((s) => s.name.toLowerCase() === q) ? `<button data-custom="1"><b>Add “${esc(input.value.trim())}”</b><small>As a source of your own</small></button>` : "");
      list.querySelectorAll("button").forEach((b) => (b.onclick = () => {
        if (b.dataset.id) {
          if (plan.removed.includes(b.dataset.id)) plan.removed = plan.removed.filter((x) => x !== b.dataset.id);
          else plan.added.push({ id: b.dataset.id });
        } else {
          plan.added.push({ custom: input.value.trim().slice(0, 80) });
        }
        close(); onChange();
      }));
    };
    const close = () => { pop.hidden = true; document.removeEventListener("mousedown", outside); };
    const outside = (e) => { if (!pop.contains(e.target) && e.target !== anchor) close(); };
    input.oninput = draw;
    input.onkeydown = (e) => { if (e.key === "Escape") close(); if (e.key === "Enter") { const f = list.querySelector("button"); if (f) f.click(); } };
    setTimeout(() => document.addEventListener("mousedown", outside), 0);
    draw();
    input.focus();
  }

  function countUp(el, target, fmt) {
    const f = fmt === "bytes" ? bytes : fmt === "pct" ? (v) => (Math.round(v * 10) / 10).toString().replace(/\.0$/, "") + "%" : num;
    const start = performance.now(), dur = 1300;
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    function step(now) {
      const p = reduce ? 1 : Math.min(1, (now - start) / dur);
      const e = 1 - Math.pow(1 - p, 3);
      el.textContent = f(p >= 1 ? target : target * e);
      if (p < 1) requestAnimationFrame(step);
    }
    requestAnimationFrame(step);
  }

  /** Shows the Data map or the Training value tab. */
  function setTab(tab) {
    activeTab = tab;
    document.querySelectorAll("[data-tab]").forEach((b) => { b.classList.toggle("on", b.dataset.tab === tab); b.setAttribute("aria-selected", String(b.dataset.tab === tab)); });
    document.querySelectorAll("[data-pane]").forEach((p) => (p.hidden = p.dataset.pane !== tab));
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  /** The last card on the Inventory tab: a pointer to Analysis & value. */
  function renderTvCard(host, map, ev, state) {
    if (!host) return;
    let text;
    if (state && state.running) {
      const p = state.progress || {};
      text = `<div class="adding-ring small" aria-hidden="true"></div><span><b>Evaluating training value…</b> ${num(p.documents || 0)} documents measured</span>`;
    } else if (ev) {
      const rating = { high: "High", medium: "Medium", low: "Low" }[ev.rating] || ev.rating;
      const ep = ev.episodes || {};
      text = `<span><b>Next: what it's worth.</b> ${esc(rating)} training value · about ${compact(ev.totals.usable_tokens)} usable tokens${ep.episodes ? ` · ${num(ep.episodes)} pieces of linked work` : ""}</span>`;
    } else {
      text = `<span><b>Next: what it's worth.</b> <span class="muted">See what's in your data, how much is useful for training AI models, and how to raise its value.</span></span>`;
    }
    host.innerHTML = `<div class="card tv-slim">${text}<button class="btn secondary small" data-tvc="view">Analysis &amp; value →</button></div>`;
    host.querySelector('[data-tvc="view"]').onclick = () => setTab("training");
  }

  window.ScoutMap = { render, bytes, num, esc, renderTraining, renderTvCard, setTab };
})();
