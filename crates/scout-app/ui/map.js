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
  const SLACK_PARTS = [
    ["participant_message_records", "People's messages", "#187C75"],
    ["bot_integration_message_records", "Bots & integrations", "#6D5BD0"],
    ["administrative_records", "Membership & admin", "#AEB8C6"],
  ];
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
    if (attachTargets.has(c.label)) return { badge: "↓", color: "#22A094", kind: "Slack attachments", matched: attachTargets.get(c.label) };
    const cat = dominantCategory(c);
    if (cat === "Email") return { badge: "@", color: CAT.Email, kind: "Email archive" };
    if ((c.formats || []).length === 1 && c.logical_files === 1) {
      return { badge: (c.formats[0].format || "F")[0], color: CAT[cat] || CAT.Other, kind: c.formats[0].format + " file" };
    }
    return { badge: "▤", color: "#187C75", kind: "Files and documents" };
  }

  // ---------------------------------------------------------------- packing
  /** Spiral packing: biggest first, each circle at the nearest free spot. */
  function pack(items, pad) {
    const placed = [];
    const sorted = items.slice().sort((a, b) => b.r - a.r);
    for (const it of sorted) {
      if (!placed.length) { it.x = 0; it.y = 0; placed.push(it); continue; }
      let ok = false;
      for (let t = 0; t < 4000 && !ok; t++) {
        const a = t * 0.35, d = 1.2 * t * 0.35;
        const x = Math.cos(a) * d, y = Math.sin(a) * d;
        if (placed.every((p) => Math.hypot(p.x - x, p.y - y) >= p.r + it.r + pad)) { it.x = x; it.y = y; ok = true; }
      }
      if (!ok) { it.x = 0; it.y = 0; }
      placed.push(it);
    }
    // centre on the enclosing circle's approximate centre
    const cx = placed.reduce((s, p) => s + p.x * p.r * p.r, 0) / placed.reduce((s, p) => s + p.r * p.r, 0);
    const cy = placed.reduce((s, p) => s + p.y * p.r * p.r, 0) / placed.reduce((s, p) => s + p.r * p.r, 0);
    placed.forEach((p) => { p.x -= cx; p.y -= cy; });
    const R = Math.max(...placed.map((p) => Math.hypot(p.x, p.y) + p.r));
    return { items: placed, R };
  }

  // ---------------------------------------------------------------- layout
  function layout(map) {
    const cols = map.collections.map((c, i) => {
      const s = sumSlack(c);
      const score = Math.max(c.logical_files, (c.structured_records || 0) / 25, s.message_records || 0);
      return { c, i, s, score, info: kindInfo(c) };
    });
    const maxScore = Math.max(1, ...cols.map((n) => n.score));
    cols.forEach((n) => { n.r = 58 + 64 * Math.sqrt(n.score / maxScore); n.type = "col"; });

    // inner bubbles
    cols.forEach((n) => {
      let parts;
      if ((n.c.slack || []).length) {
        parts = SLACK_PARTS.map(([k, label, color]) => ({ label, color, value: n.s[k] || 0, unit: "messages", examined: true }))
          .filter((p) => p.value > 0);
      } else {
        parts = (n.c.formats || []).map((f) => ({
          label: f.format, color: CAT[f.category] || CAT.Other, value: f.files, unit: f.files === 1 ? "file" : "files",
          examined: f.examined > 0, bytes: f.bytes, category: f.category, examinedCount: f.examined,
        }));
        if (n.c.logical_files === 1 && parts.length === 1 && n.c.structured_records > 0) {
          // A single file is more meaningfully shown by what is inside it.
          const isMail = parts[0].category === "Email";
          parts[0].note = "1 file · " + parts[0].label;
          parts[0].value = n.c.structured_records;
          parts[0].unit = isMail ? "emails" : "records";
          parts[0].label = isMail ? "emails" : "records";
        }
      }
      const maxV = Math.max(1, ...parts.map((p) => p.value));
      parts.forEach((p) => { p.r = 6 + 40 * Math.sqrt(p.value / maxV); });
      const packed = pack(parts, 3);
      const scale = packed.R > 0 ? (n.r * 0.8) / packed.R : 1;
      packed.items.forEach((p) => { p.x *= scale; p.y *= scale; p.r *= scale; });
      n.parts = packed.items;
    });

    // hub + collections on an ellipse
    const nodes = [];
    const hub = { type: "hub", x: 0, y: 0, r: cols.length > 1 ? 42 : 0, fixed: true };
    if (cols.length > 1) nodes.push(hub);
    const n = cols.length;
    const maxR = Math.max(...cols.map((c) => c.r));
    const ring = n === 1 ? 0 : Math.max(hub.r + maxR + 70, (2 * maxR + 60) / (2 * Math.sin(Math.PI / n)));
    cols.forEach((c, i) => {
      const a = -Math.PI / 2 + (i * 2 * Math.PI) / Math.max(1, n) + (n === 2 ? Math.PI / 2 : 0);
      c.angle = a;
      c.x = Math.cos(a) * ring * 1.55;
      c.y = Math.sin(a) * ring * 0.9;
      nodes.push(c);
    });

    // satellites for referenced-but-missing material
    const rels = map.relationships || [];
    const maxCount = Math.max(1, ...rels.map((r) => r.count));
    const ghosts = [];
    cols.forEach((c) => {
      const mine = rels.filter((r) => r.from === c.c.label);
      const base = n === 1 ? -Math.PI / 4 : Math.atan2(c.y, c.x);
      mine.forEach((r, j) => {
        const spread = mine.length > 1 ? (j - (mine.length - 1) / 2) * 0.62 : 0;
        const a = base + spread;
        const gr = 22 + 22 * Math.sqrt(r.count / maxCount);
        const d = c.r + 72 + gr;
        const g = { type: "ghost", rel: r, parent: c, r: gr, x: c.x + Math.cos(a) * d, y: c.y + Math.sin(a) * d, rest: d };
        ghosts.push(g);
        nodes.push(g);
      });
    });

    // relax: separate overlapping nodes, keep satellites near their parent
    const gap = 30;
    for (let it = 0; it < 260; it++) {
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const a = nodes[i], b = nodes[j];
          const need = a.r + b.r + gap + (a.type === "col" && b.type === "col" ? 40 : 0);
          let dx = b.x - a.x, dy = b.y - a.y;
          let d = Math.hypot(dx, dy) || 0.01;
          if (d < need) {
            const push = (need - d) / 2;
            dx /= d; dy /= d;
            const wa = a.fixed ? 0 : a.type === "col" ? 0.6 : 1;
            const wb = b.fixed ? 0 : b.type === "col" ? 0.6 : 1;
            const tot = wa + wb || 1;
            a.x -= dx * push * (2 * wa / tot); a.y -= dy * push * (2 * wa / tot);
            b.x += dx * push * (2 * wb / tot); b.y += dy * push * (2 * wb / tot);
          }
        }
      }
      for (const g of ghosts) {
        const p = g.parent;
        const dx = g.x - p.x, dy = g.y - p.y, d = Math.hypot(dx, dy) || 1;
        const k = 0.08 * (d - g.rest) / d;
        g.x -= dx * k; g.y -= dy * k;
      }
    }

    // bounds, including labels below collections and satellites
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const nd of nodes) {
      const below = nd.type === "col" ? 52 : nd.type === "ghost" ? 48 : 0;
      const side = nd.type === "ghost" ? 60 : nd.type === "col" ? 30 : 0;
      x0 = Math.min(x0, nd.x - nd.r - side); x1 = Math.max(x1, nd.x + nd.r + side);
      y0 = Math.min(y0, nd.y - nd.r - (nd.type === "ghost" ? 48 : 8)); y1 = Math.max(y1, nd.y + nd.r + below);
    }
    const padX = 24, padY = 16;
    return { hub, cols, ghosts, box: [x0 - padX, y0 - padY, x1 - x0 + 2 * padX, y1 - y0 + 2 * padY] };
  }

  function edgePath(a, b, bend) {
    const dx = b.x - a.x, dy = b.y - a.y, d = Math.hypot(dx, dy) || 1;
    const ux = dx / d, uy = dy / d;
    const sx = a.x + ux * a.r, sy = a.y + uy * a.r;
    const ex = b.x - ux * b.r, ey = b.y - uy * b.r;
    const mx = (sx + ex) / 2 - uy * bend, my = (sy + ey) / 2 + ux * bend;
    return { d: `M${sx.toFixed(1)},${sy.toFixed(1)} Q${mx.toFixed(1)},${my.toFixed(1)} ${ex.toFixed(1)},${ey.toFixed(1)}`, mx, my };
  }

  // ---------------------------------------------------------------- map SVG
  function mapSvg(map) {
    const L = layout(map);
    const [bx, by, bw, bh] = L.box;
    const tips = [];
    const tip = (html) => { tips.push(html); return `data-tip="${tips.length - 1}"`; };
    // Never draw larger than 1.25x natural size, so a map with one or two
    // collections stays legible instead of filling the card with giant type.
    let s = `<svg class="map-svg" style="max-width:${Math.round(bw * 1.25)}px;margin:0 auto" viewBox="${bx.toFixed(0)} ${by.toFixed(0)} ${bw.toFixed(0)} ${bh.toFixed(0)}" role="img" aria-label="Map of your data collections">
      <defs>
        <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <rect width="6" height="6" fill="#fff"/><rect width="2.2" height="6" fill="#CAD4E0"/>
        </pattern>
        <radialGradient id="hubg" cx="35%" cy="30%"><stop offset="0" stop-color="#2C4F73"/><stop offset="1" stop-color="#14283F"/></radialGradient>
        <filter id="soft" x="-30%" y="-30%" width="160%" height="160%"><feDropShadow dx="0" dy="6" stdDeviation="9" flood-color="#14283F" flood-opacity=".12"/></filter>
      </defs>`;

    // hub spokes
    if (L.hub.r) {
      L.cols.forEach((c, i) => {
        // stop short of the caption when a collection sits straight above the hub
        const clear = c.y < 0 && Math.abs(c.x) < c.r ? 48 : 0;
        const e = edgePath(L.hub, { x: c.x, y: c.y, r: c.r + clear }, 0);
        s += `<path d="${e.d}" stroke="#C9D5E2" stroke-width="2" fill="none" class="draw" style="animation-delay:${200 + i * 120}ms"/>`;
      });
    }
    // satellite links
    L.ghosts.forEach((g, i) => {
      const e = edgePath(g.parent, g, 18);
      const delay = 1100 + i * 140;
      const done = g.rel.resolved >= g.rel.count;
      s += `<path d="${e.d}" stroke="${done ? "#22A094" : "#E2A23B"}" stroke-width="2" fill="none" class="${done ? "" : "flow "}fade" style="animation-delay:${delay}ms, ${delay}ms"/>`;
      // where the originals were found: a solid teal link to that collection
      (g.rel.found_in || []).forEach((label) => {
        const target = L.cols.find((c) => c.c.label === label);
        if (!target) return;
        const f = edgePath(g, target, -26);
        s += `<path d="${f.d}" stroke="#22A094" stroke-width="2.5" fill="none" class="draw" style="animation-delay:${delay + 300}ms"/>`;
      });
    });

    // collections
    L.cols.forEach((n, i) => {
      const delay = 150 + i * 140;
      const c = n.c;
      const examined = (c.by_status.assessed || 0) + (c.by_status.partly_assessed || 0);
      const denom = Math.max(1, c.logical_files - (c.by_status.excluded || 0));
      const pct = Math.round((examined * 100) / denom);
      s += `<g transform="translate(${n.x.toFixed(1)},${n.y.toFixed(1)})"><g class="node-collection grow" style="animation-delay:${delay}ms">
        <circle class="halo" r="${(n.r + 9).toFixed(1)}" fill="none" stroke="${n.info.color}" stroke-opacity=".25" stroke-width="8"/>
        <circle r="${n.r.toFixed(1)}" fill="#F7FAFC" stroke="${n.info.color}" stroke-width="2" filter="url(#soft)"/>
        <circle r="${(n.r - 6).toFixed(1)}" fill="none" stroke="${n.info.color}" stroke-opacity=".14" stroke-width="1" stroke-dasharray="2 4"/>`;
      n.parts.forEach((p, j) => {
        const fill = p.examined ? p.color : "url(#hatch)";
        const t = tip(`<b>${esc(p.label)}</b>${num(p.value)} ${esc(p.unit)}${p.bytes ? " · " + bytes(p.bytes) : ""}${p.note ? "<br>" + esc(p.note) : ""}<br><span>${p.examined ? "Contents examined" : "Counted only: needs a parser that is not in this release"}</span>`);
        s += `<g class="bubble grow" style="animation-delay:${delay + 260 + j * 45}ms" ${t}>
          <circle cx="${p.x.toFixed(1)}" cy="${p.y.toFixed(1)}" r="${Math.max(2, p.r).toFixed(1)}" fill="${fill}" stroke="${p.color}" stroke-width="${p.examined ? 0 : 1.5}" fill-opacity="${p.examined ? 0.92 : 1}"/>`;
        if (p.r > 21) {
          const fs = clamp(p.r / 3.6, 9, 13);
          const ink = p.examined ? "#fff" : "#3E5370";
          s += `<text x="${p.x.toFixed(1)}" y="${(p.y - fs * 0.2).toFixed(1)}" text-anchor="middle" font-size="${(fs * 1.15).toFixed(1)}" font-weight="700" fill="${ink}" pointer-events="none">${num(p.value)}</text>
                <text x="${p.x.toFixed(1)}" y="${(p.y + fs * 1.05).toFixed(1)}" text-anchor="middle" font-size="${(fs * 0.82).toFixed(1)}" fill="${ink}" fill-opacity=".9" pointer-events="none">${esc(trunc(p.label, Math.floor(p.r / 2.9)))}</text>`;
        }
        s += `</g>`;
      });
      s += `<text class="halo-text" y="${(n.r + 22).toFixed(1)}" text-anchor="middle" font-size="14.5" font-weight="650" fill="#14283F">${esc(trunc(c.label, 30))}</text>
        <text class="halo-text" y="${(n.r + 40).toFixed(1)}" text-anchor="middle" font-size="12" fill="#6B7C93">${esc(n.info.kind)} · ${num(c.logical_files)} ${c.logical_files === 1 ? "file" : "files"} · ${n.info.matched != null ? `${num(n.info.matched)} matched to conversations` : `${pct}% examined`}</text>
      </g></g>`;
    });

    // satellites
    L.ghosts.forEach((g, i) => {
      const delay = 1200 + i * 140;
      const r = g.rel;
      const missing = Math.max(0, r.count - (r.resolved || 0));
      const frac = r.count ? (r.resolved || 0) / r.count : 0;
      const done = missing === 0;
      const t = tip(`<b>${esc(r.to)}</b>${esc(r.explanation)}<br><span>${done ? "All found in your selection." : "See “Make your data more complete” below for how to get these."}</span>`);
      const lines = wrap(r.to, 18);
      const fs = clamp(g.r / 2.3, 11, 17);
      const circ = 2 * Math.PI * g.r;
      s += `<g transform="translate(${g.x.toFixed(1)},${g.y.toFixed(1)})"><g class="grow" style="animation-delay:${delay}ms" ${t}>`;
      if (done) {
        s += `<circle r="${g.r.toFixed(1)}" fill="#E3F3F1" stroke="#22A094" stroke-width="2.5"/>
          <path d="M${-fs * 0.45},${-fs * 0.35} l${fs * 0.3},${fs * 0.3} l${fs * 0.6},${-fs * 0.6}" stroke="#187C75" stroke-width="2.6" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
          <text y="${(fs * 0.95).toFixed(1)}" text-anchor="middle" font-size="10.5" font-weight="650" fill="#187C75">${num(r.count)} added</text>`;
      } else {
        s += `<circle r="${g.r.toFixed(1)}" fill="#FDF6EA" stroke="#E2A23B" stroke-width="2" stroke-dasharray="5 4"/>`;
        if (frac > 0) {
          s += `<circle r="${g.r.toFixed(1)}" fill="none" stroke="#22A094" stroke-width="4" stroke-linecap="round" transform="rotate(-90)"
            stroke-dasharray="${(circ * frac).toFixed(1)} ${circ.toFixed(1)}"/>`;
        }
        s += `<text y="-2" text-anchor="middle" font-size="${fs.toFixed(1)}" font-weight="700" fill="#8A5A0B">${num(missing)}</text>
          <text y="${(fs * 0.85).toFixed(1)}" text-anchor="middle" font-size="10" fill="#A56E14">${frac > 0 ? "still missing" : "missing"}</text>`;
      }
      // put the caption on the side away from the parent, clear of the link
      const above = g.y < g.parent.y - g.parent.r * 0.3;
      lines.forEach((ln, k) => {
        const y = above ? -g.r - 10 - (lines.length - 1 - k) * 14 : g.r + 16 + k * 14;
        s += `<text class="halo-text" y="${y.toFixed(1)}" text-anchor="middle" font-size="12" fill="#6E4A0E">${esc(ln)}</text>`;
      });
      s += `</g></g>`;
    });

    // hub
    if (L.hub.r) {
      const total = map.totals.logical_files;
      s += `<g class="grow" style="animation-delay:40ms">
        <circle r="${L.hub.r}" fill="url(#hubg)" filter="url(#soft)"/>
        <circle r="${L.hub.r + 6}" fill="none" stroke="#14283F" stroke-opacity=".12" stroke-width="1.5" stroke-dasharray="3 4"/>
        <text y="-4" text-anchor="middle" font-size="11" fill="#9FB4CA" font-weight="600" letter-spacing=".06em">YOUR DATA</text>
        <text y="14" text-anchor="middle" font-size="15" fill="#fff" font-weight="700">${num(total)}</text>
      </g>`;
    }
    s += `</svg>`;
    return { svg: s, tips };
  }

  function wrap(text, width) {
    const words = String(text).split(/\s+/), out = [];
    let cur = "";
    for (const w of words) {
      if ((cur + " " + w).trim().length > width && cur) { out.push(cur); cur = w; } else cur = (cur + " " + w).trim();
    }
    if (cur) out.push(cur);
    return out.slice(0, 3);
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
    const M = mapSvg(map);
    const T = timelineSvg(map.activity);
    const lead = map.summary[0] || "";
    const more = map.summary.slice(1).join(" ");
    const ringC = 2 * Math.PI * 52;
    const totalCov = Object.values(cov.by_status).reduce((a, b) => a + b, 0) || 1;

    const legendCats = new Set();
    map.collections.forEach((c) => (c.formats || []).forEach((f) => { if (!(c.slack || []).length) legendCats.add(f.category); }));
    const hasSlack = map.collections.some((c) => (c.slack || []).length);
    const legend = [
      ...[...legendCats].map((k) => `<span><i style="background:${CAT[k] || CAT.Other}"></i>${esc(k)}</span>`),
      ...(hasSlack ? SLACK_PARTS.map(([, l, c]) => `<span><i style="background:${c}"></i>${l}</span>`) : []),
      `<span><i class="hatch"></i>Counted, contents not yet readable</span>`,
      ...((map.relationships || []).length ? [`<span><i class="dashed"></i>Referenced but not included</span>`] : []),
    ].join("");

    const sources = map.collections.map((c, i) => {
      const info = kindInfo(c);
      const s = sumSlack(c);
      const examined = (c.by_status.assessed || 0) + (c.by_status.partly_assessed || 0);
      const denom = Math.max(1, c.logical_files - (c.by_status.excluded || 0));
      const stats = (c.slack || []).length
        ? [["messages", s.message_records], ["threads", s.explicit_threads], ["posting accounts", s.accounts_posting_participant_content], ["channels", s.channels_in_metadata]]
        : info.matched != null
          ? [["files", c.logical_files], ["matched to messages", info.matched], ["size", null, bytes(c.expanded_bytes)], ["examined", null, Math.round((examined * 100) / denom) + "%"]]
          : [["files", c.logical_files], ["records", c.structured_records], ["size", null, bytes(c.expanded_bytes)], ["examined", null, Math.round((examined * 100) / denom) + "%"]];
      const bar = STATUS.map(([k, , color]) => {
        const v = c.by_status[k] || 0;
        return v ? `<span style="width:${(v * 100) / Math.max(1, c.logical_files)}%;background:${color}"></span>` : "";
      }).join("");
      return `<div class="card source reveal" style="animation-delay:${900 + i * 90}ms">
        <div class="source-top"><div class="source-badge" style="background:${info.color}">${esc(info.badge)}</div>
          <div style="min-width:0"><h3 title="${esc(c.label)}">${esc(c.label)}</h3><div class="kind">${esc(info.kind)}${c.date_coverage ? " · " + esc(c.date_coverage.first) + " to " + esc(c.date_coverage.last) : ""}</div></div></div>
        <p>${esc(c.explanation)}</p>
        <div class="source-stats">${stats.map(([l, v, txt]) => `<div><b>${txt ?? num(v)}</b>${l}</div>`).join("")}</div>
        <div class="mini-bar" title="Coverage">${bar}</div>
      </div>`;
    }).join("");

    const reasons = (cov.reasons || []).slice().sort((a, b) => b.files - a.files).slice(0, 4);

    root.innerHTML = `
      <div class="map-head reveal">
        <div>
          <p class="eyebrow">Step 3 of 3 · Your data map</p>
          <h1>Here's what you have</h1>
          <div class="map-id">Assessment ${esc(map.assessment_id)} · ${esc(map.app)} ${esc(map.app_version)} · rules ${esc(map.ruleset_version)}</div>
        </div>
        <div class="actions">
          <button class="btn ghost" data-act="restart">Start again</button>
          <button class="btn secondary" data-act="adddata"><span aria-hidden="true">＋</span> Add data</button>
          <button class="btn primary" data-act="save">Save my data map</button>
        </div>
      </div>
      ${extra.banner || ""}
      <div class="add-hint reveal" style="animation-delay:60ms"><span class="status-dot"></span>Drop more files or folders anywhere on this page to add them to your map. Only the new material is read.</div>

      <div class="card summary-card reveal" style="animation-delay:80ms">
        <div style="position:relative;z-index:1">
          <div class="summary-lead">${esc(lead)}</div>
          ${more ? `<p class="summary-more">${esc(more)}</p>` : ""}
        </div>
        <div class="ring-wrap" title="Share of files whose contents were examined">
          <svg viewBox="0 0 120 120"><circle cx="60" cy="60" r="52" fill="none" stroke="rgba(255,255,255,.14)" stroke-width="10"/>
            <circle class="ring-fg" cx="60" cy="60" r="52" fill="none" stroke="#7FD1C6" stroke-width="10" stroke-linecap="round"
              stroke-dasharray="${ringC.toFixed(1)}" stroke-dashoffset="${ringC.toFixed(1)}" style="transition: stroke-dashoffset 1.4s cubic-bezier(.2,.7,.2,1) .4s"/></svg>
          <div class="ring-label"><strong data-count="${pct}" data-fmt="pct">0%</strong><span>of files<br>examined</span></div>
        </div>
      </div>

      <div class="kpis">
        <div class="card kpi reveal" style="animation-delay:160ms"><strong data-count="${t.logical_files}">0</strong><span>files across ${num(map.collections.length)} ${map.collections.length === 1 ? "collection" : "collections"}</span></div>
        <div class="card kpi reveal" style="animation-delay:220ms"><strong data-count="${t.structured_records}">0</strong><span>records and messages counted</span></div>
        <div class="card kpi reveal" style="animation-delay:280ms"><strong>${esc(sp.value)}</strong><span>${esc(sp.note)}</span></div>
        <div class="card kpi reveal" style="animation-delay:340ms"><strong data-count="${t.unique_bytes}" data-fmt="bytes">0 B</strong><span>unique content${t.exact_duplicate_files ? ` · ${num(t.exact_duplicate_files)} exact duplicates` : ""}</span></div>
      </div>

      <div class="card map-card reveal" style="animation-delay:120ms">
        <div class="card-head"><div><h2>Your data, mapped</h2><p>Each circle is a collection you added; bubbles show what is inside. Amber satellites are things your data refers to that were not included.</p></div></div>
        <div id="map-svg-host">${M.svg}</div>
        <div class="legend">${legend}</div>
      </div>
      ${completeBlock()}

      <div class="grid-2">
        <div class="card panel-card reveal" style="animation-delay:600ms">
          <div class="card-head"><div><h2>${T ? "Activity over time" : "Assessment coverage"}</h2><p>${T ? "Message records per month, from timestamps inside your exports" : "How much of each file we could read"}</p></div></div>
          ${T ? `<div id="tl-host">${T.svg}</div>` : coverageBlock()}
        </div>
        <div class="card panel-card reveal" style="animation-delay:680ms">
          ${T ? `<div class="card-head"><div><h2>Assessment coverage</h2><p>Every file is counted, and nothing is left out of the totals</p></div></div>${coverageBlock()}` : stepsBlock(true)}
        </div>
      </div>

      ${languagesBlock()}
      <div class="sources">${sources}</div>
      ${T ? stepsBlock(false) : ""}
      <div class="privacy-foot reveal" style="animation-delay:1200ms"><span class="status-dot"></span>${esc(map.privacy.statement)}</div>
    `;

    function coverageBlock() {
      return `<div class="coverage-bar">${STATUS.map(([k, , c]) => `<span data-w="${((cov.by_status[k] || 0) * 100) / totalCov}" style="background:${c}"></span>`).join("")}</div>
        <ul class="coverage-list">${STATUS.map(([k, l, c]) => `<li class="${cov.by_status[k] ? "" : "zero"}"><i class="swatch" style="background:${c}"></i>${l}<b>${num(cov.by_status[k] || 0)}</b></li>`).join("")}</ul>
        ${reasons.length ? `<div class="reasons">${reasons.map((r) => `<div><b>${num(r.files)}</b><span>${esc(r.explanation)}</span></div>`).join("")}</div>` : ""}`;
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
        + (l.undetermined_words ? row("Too short to tell", "#DDE3EA", l.undetermined_words, l.undetermined_percent, "short replies, links, emoji") : "");

      // by-collection matrix: one column per top language, then other and undetermined
      const cols = top.slice(0, 3);
      const matrix = l.by_collection.map((c) => {
        const cell = (w) => {
          const p = c.words_examined ? Math.round((w * 100) / c.words_examined) : 0;
          return `<td class="num">${w ? `${p < 1 ? "<1" : p}%` : "—"}</td>`;
        };
        const known = new Set(cols.map((x) => x.code));
        const other = c.rows.filter((r) => !known.has(r.code)).reduce((a, r) => a + r.words, 0);
        return `<tr><td class="coll" title="${esc(c.label)}">${esc(c.label)}<span class="muted small">${num(c.words_examined)} words · ${esc(c.basis)}</span></td>
          ${cols.map((x) => cell((c.rows.find((r) => r.code === x.code) || {}).words || 0)).join("")}
          ${cell(other)}${cell(c.undetermined_words)}</tr>`;
      }).join("");

      const en = l.english_percent;
      const notes = [];
      if (l.unreadable_documents) notes.push(`${num(l.unreadable_documents)} PDF and Office ${l.unreadable_documents === 1 ? "document is" : "documents are"} not included yet: reading their text arrives in a later release`);
      if (l.sampled_documents) notes.push(`${num(l.sampled_documents)} long ${l.sampled_documents === 1 ? "document was" : "documents were"} sampled from the start (first 256 KB)`);
      notes.push("Slack bot and integration messages are left out, so the mix reflects what people wrote");
      return `<div class="card lang-card reveal" style="animation-delay:760ms">
        <div class="card-head"><div><h2>Languages</h2><p>Apparent language of the readable text, measured in words</p></div>
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

    function completeBlock() {
      const rels = map.relationships || [];
      if (!rels.length) return "";
      const open = rels.filter((r) => (r.resolved || 0) < r.count).length;
      const rows = rels.map((r) => {
        const missing = Math.max(0, r.count - (r.resolved || 0));
        const pctDone = r.count ? Math.round(((r.resolved || 0) * 100) / r.count) : 0;
        const status = missing === 0
          ? `<span class="pill ok">All ${num(r.count)} added</span>`
          : r.resolved
            ? `<span class="pill part">${num(r.resolved)} of ${num(r.count)} added</span>`
            : `<span class="pill miss">${num(r.count)} missing</span>`;
        return `<div class="gap-row">
          <div class="gap-meter" style="--p:${pctDone}"><span>${pctDone}%</span></div>
          <div class="gap-text"><b>${esc(r.to)}</b><span>${esc(r.explanation)}</span></div>
          ${status}
          ${missing ? `<button class="btn secondary small" data-guide="${esc(r.kind)}">How to get these</button>` : `<span></span>`}
        </div>`;
      }).join("");
      return `<div class="card complete-card reveal" style="animation-delay:500ms">
        <div class="card-head"><div><h2>Make your data more complete</h2><p>${open ? "Your data refers to material that is not in your selection yet. Adding it makes conversations easier to follow and more valuable." : "Everything your data refers to is in your selection. Nice work."}</p></div>
          <button class="btn secondary small" data-act="adddata"><span aria-hidden="true">＋</span> Add data</button></div>
        ${rows}
      </div>`;
    }

    function stepsBlock(inline) {
      const steps = map.preparation_priorities || [];
      if (!steps.length) return inline ? `<div class="card-head"><div><h2>Useful next steps</h2><p>Nothing obvious to add. This looks complete.</p></div></div>` : "";
      const body = `<div class="card-head"><div><h2>Useful next steps</h2><p>Ranked by how much they would add to the picture</p></div></div>
        <ol>${steps.map((x) => `<li><span>${esc(x)}</span></li>`).join("")}</ol>`;
      return inline ? `<div class="steps" style="padding:0;margin:0">${body}</div>` : `<div class="card steps reveal" style="animation-delay:1000ms">${body}</div>`;
    }

    // tooltips
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

    // entrance: count-ups, ring and bars
    requestAnimationFrame(() => {
      const ring = root.querySelector(".ring-fg");
      if (ring) ring.style.strokeDashoffset = (ringC * (1 - pct / 100)).toFixed(1);
      root.querySelectorAll(".coverage-bar span").forEach((el) => { setTimeout(() => (el.style.width = el.dataset.w + "%"), 500); });
      root.querySelectorAll("[data-count]").forEach((el) => countUp(el, +el.dataset.count, el.dataset.fmt));
    });
    root.querySelector('[data-act="save"]').onclick = handlers.onSave;
    root.querySelector('[data-act="restart"]').onclick = handlers.onRestart;
    root.querySelectorAll('[data-act="adddata"]').forEach((b) => (b.onclick = () => handlers.onAddData(b)));
    root.querySelectorAll("[data-guide]").forEach((b) => (b.onclick = () => handlers.onGuide(b.dataset.guide)));
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

  window.ScoutMap = { render, bytes, num, esc };
})();
