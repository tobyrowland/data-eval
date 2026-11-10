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
    if (c.teams) return { badge: "T", color: "#5059C9", kind: "Microsoft Teams export" };
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
      // Wide layout for a wide frame: an ellipse, and for 4+ collections
      // rotated so none sits straight above the hub (satellites would stack up).
      const start = n === 2 ? 0 : n >= 4 ? -Math.PI / 2 + Math.PI / n : -Math.PI / 2;
      const a = start + (i * 2 * Math.PI) / Math.max(1, n);
      c.angle = a;
      c.x = Math.cos(a) * ring * 2.0;
      c.y = Math.sin(a) * ring * 0.72;
      nodes.push(c);
    });

    // satellites for referenced-but-missing material
    const rels = map.relationships || [];
    const maxCount = Math.max(1, ...rels.map((r) => r.count));
    const ghosts = [];
    cols.forEach((c) => {
      const mine = rels.filter((r) => r.from === c.c.label);
      // Point satellites outward, leaning sideways so the map stays wide.
      const base = n === 1 ? 0 : Math.atan2(c.y * 0.55, c.x);
      mine.forEach((r, j) => {
        const spread = mine.length > 1 ? (j - (mine.length - 1) / 2) * 0.72 : 0;
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
    let s = `<svg class="map-svg" style="max-width:${Math.round(bw * 1.4)}px;margin:0 auto" viewBox="${bx.toFixed(0)} ${by.toFixed(0)} ${bw.toFixed(0)} ${bh.toFixed(0)}" role="img" aria-label="Map of your data collections">
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
      s += `<text class="halo-text" y="${(n.r + 22).toFixed(1)}" text-anchor="middle" font-size="14.5" font-weight="650" fill="#14283F">${esc(trunc(c.display_label || c.label, 34))}</text>
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
    const totalCov = Object.values(cov.by_status).reduce((a, b) => a + b, 0) || 1;
    const plan = normalisePlan(extra.plan);

    const legendCats = new Set();
    map.collections.forEach((c) => (c.formats || []).forEach((f) => { if (!(c.slack || []).length) legendCats.add(f.category); }));
    const hasSlack = map.collections.some((c) => (c.slack || []).length);
    const legend = [
      ...[...legendCats].map((k) => `<span><i style="background:${CAT[k] || CAT.Other}"></i>${esc(k)}</span>`),
      ...(hasSlack ? SLACK_PARTS.map(([, l, c]) => `<span><i style="background:${c}"></i>${l}</span>`) : []),
      `<span><i class="hatch"></i>Counted, not yet readable</span>`,
      ...((map.relationships || []).length ? [`<span><i class="dashed"></i>Referenced, not included</span>`] : []),
    ].join("");

    root.innerHTML = `
      <div class="map-head reveal">
        <div>
          <p class="eyebrow">Your data map</p>
          <h1>Here's what you have</h1>
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

      <section class="section reveal" style="animation-delay:400ms">
        <div class="section-head"><div><h2>Analysis</h2><p>What is in the data you added, measured on this computer. Every file is counted, including those we can't read yet.</p></div></div>
        ${collectionsTable()}
        <div class="grid-2">
          ${coverageCard()}
          ${T ? `<div class="card panel-card"><div class="card-head"><div><h3>Activity over time</h3><p>Message records per month, from timestamps inside your exports</p></div></div><div id="tl-host">${T.svg}</div></div>` : formatsCard()}
        </div>
        ${T ? formatsCard() : ""}
        ${languagesBlock()}
      </section>
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
    root.querySelector('[data-act="restart"]').onclick = handlers.onRestart;
    root.querySelector('[data-act="pack"]').onclick = handlers.onPack;
    root.querySelectorAll('[data-act="adddata"]').forEach((b) => (b.onclick = () => handlers.onAddData(b)));
    const steps = root.querySelector("#next-steps");
    const redraw = () => renderNextSteps(steps, map, plan, handlers, redraw, extra || {});
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
    function linkedFrom(labels, service, summaryText) {
      const list = labels.filter(colOf);
      if (!list.length) return "";
      return `<details class="pc-links"><summary>${esc(summaryText)}</summary>
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
      const linkText = s && s.links ? `Linked from ${num(linkers.length)} other ${linkers.length === 1 ? "item" : "items"} · ${num(s.links)} links` : "";
      return `<div class="pcard">
        <div class="pc-head">
          <div class="pc-title"><h3>${esc(pname(p))}</h3><span class="pill ${partial ? "partial" : "ok"}">${partial ? "Partly collected" : "Collected ✓"}</span></div>
          <div class="pc-facts">${facts.map(esc).join(" · ")}</div>
        </div>
        ${notes.join("")}
        ${itemTable(list, p)}
        ${linkText ? linkedFrom(linkers, p, linkText) : ""}
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
      ${loose.length ? `<div class="pcard"><div class="pc-head"><div class="pc-title"><h3>Other files</h3></div><div class="pc-facts">${num(loose.length)} ${loose.length === 1 ? "item" : "items"} not tied to a platform</div></div>${itemTable(loose, null)}</div>` : ""}
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

  window.ScoutMap = { render, bytes, num, esc };
})();
