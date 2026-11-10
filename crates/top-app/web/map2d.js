// 2D map of the site: every pile in plan (design position), coloured by
// survey coverage, survey date, tracker status, reveal, survey − LIDAR, or the
// block's installers' file. Drawn on a canvas: the 37,000 piles of a site draw
// in a few milliseconds, so panning and zooming stay smooth.
//
// Colours follow the same reference palette as the 3D view (view3d.js):
// categorical slots in fixed order, one-hue sequential ramp, two-hue diverging
// ramp with a grey midpoint, status colours only for states.

const PAL = {
  light: { s1: '#2a78d6', s2: '#eb6834', other: '#898781', dim: '#d6d5cf', mid: '#e1e0d9', good: '#0ca30c' },
  dark: { s1: '#3987e5', s2: '#d95926', other: '#898781', dim: '#383835', mid: '#383835', good: '#0ca30c' },
};
const BLUE = ['#cde2fb', '#9ec5f4', '#6da7ec', '#3987e5', '#256abf', '#184f95', '#0d366b'];
const DIV_NEG = ['#184f95', '#2a78d6', '#86b6ef'];
const DIV_POS = ['#f0a3a2', '#e34948', '#a32c2b'];
const STATUS = {
  Compliant: ['#0ca30c', 'Compliant'],
  Graded: ['#fab219', 'Graded (cut/fill)'],
  NonCompliant: ['#d03b3b', 'Non-compliant'],
  Skipped: ['#ec835a', 'Skipped (< 3 piles)'],
  InvalidInput: ['#ec835a', 'Invalid input'],
  AwaitingSurvey: ['#898781', 'Awaiting elevations'],
  NotRun: ['#c3c2b7', 'Not optimised yet'],
};
const BUCKETS = 21; // steps of the continuous colour ramps

export const MODES = {
  coverage: 'Survey coverage',
  date: 'Survey date',
  status: 'Tracker status',
  reveal: 'Reveal',
  diff: 'Survey − LIDAR',
  installers: "Installers' file",
};

function hexRgb(h) {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function rampColor(stops, t) {
  const x = Math.min(1, Math.max(0, t)) * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(x));
  const f = x - i;
  const a = hexRgb(stops[i]), b = hexRgb(stops[i + 1]);
  return `rgb(${a.map((v, k) => Math.round(v + (b[k] - v) * f)).join(',')})`;
}
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const dayNumber = (iso) => (iso ? Math.floor(Date.parse(`${iso.slice(0, 10)}T00:00:00Z`) / 86400000) : NaN);

export class Map2D {
  constructor(host, legend, onPick) {
    this.host = host;
    this.legendEl = legend;
    this.onPick = onPick;
    this.canvas = document.createElement('canvas');
    this.canvas.setAttribute('role', 'img');
    this.canvas.setAttribute('aria-label', 'Map of the piles');
    host.prepend(this.canvas);
    this.tip = document.createElement('div');
    this.tip.className = 'tooltip';
    this.tip.hidden = true;
    host.appendChild(this.tip);
    this.ctx = this.canvas.getContext('2d');
    this.mode = 'coverage';
    this.opts = { lines: true, labels: true, outlines: true };
    this.outlines = [];
    this.rawOutlines = [];
    this.hidden = new Set();
    this.selected = -1;
    this.focusBlock = '';
    this.view = { cx: 0, cy: 0, scale: 1 };
    this.data = null;
    this.frame = 0;
    this.active = false;
    new ResizeObserver(() => this.resize()).observe(host);
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => this.draw());
    this.bindPointer();
  }

  setActive(on) {
    this.active = on;
    if (on) this.resize();
  }

  // ------------------------------------------------------------------ data

  setData(state, summary) {
    const p = state.piles;
    const n = p.id.length;
    let e0 = Infinity, n0 = Infinity;
    for (let i = 0; i < n; i++) { e0 = Math.min(e0, p.e[i]); n0 = Math.min(n0, p.n[i]); }
    const x = new Float64Array(n), y = new Float64Array(n);
    for (let i = 0; i < n; i++) { x[i] = p.e[i] - e0; y[i] = p.n[i] - n0; }
    // Spatial grid (10 m cells) to find the pile under the pointer.
    const grid = new Map();
    for (let i = 0; i < n; i++) {
      const k = `${Math.floor(x[i] / 10)},${Math.floor(y[i] / 10)}`;
      if (!grid.has(k)) grid.set(k, []);
      grid.get(k).push(i);
    }
    // Blocks: bounds and centre, for labels and zooming.
    const blocks = new Map();
    for (let i = 0; i < n; i++) {
      const b = p.block[i];
      let o = blocks.get(b);
      if (!o) blocks.set(b, (o = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity, sx: 0, sy: 0, n: 0 }));
      o.x0 = Math.min(o.x0, x[i]); o.y0 = Math.min(o.y0, y[i]);
      o.x1 = Math.max(o.x1, x[i]); o.y1 = Math.max(o.y1, y[i]);
      o.sx += x[i]; o.sy += y[i]; o.n++;
    }
    const trackerOf = new Int32Array(n);
    state.trackers.forEach((t, k) => { for (let i = t.first_pile; i < t.first_pile + t.n_piles; i++) trackerOf[i] = k; });
    const first = !this.data;
    this.state = state;
    this.summary = summary;
    this.data = { n, x, y, grid, blocks, trackerOf, e0, n0 };
    this.setOutlines(this.rawOutlines);
    this.classify();
    if (first) this.fit();
    else this.draw();
  }

  /** Block outlines from the program: [{ block, rings: [[[e, n], ...]] }]. */
  setOutlines(list) {
    this.rawOutlines = list || [];
    if (!this.data) return;
    const { e0, n0 } = this.data;
    this.outlines = this.rawOutlines.map((o) => ({
      block: o.block,
      rings: o.rings.map((r) => r.map(([e, n]) => [e - e0, n - n0])),
    }));
    this.draw();
  }

  setMode(mode) {
    this.mode = mode;
    this.hidden.clear();
    this.classify();
    this.draw();
  }

  setOptions(o) {
    Object.assign(this.opts, o);
    this.draw();
  }

  /** Colour class of every pile for the current mode, and the legend. */
  classify() {
    if (!this.data) return;
    const p = this.state.piles, n = this.data.n, s = this.summary.settings;
    const dark = this.isDark();
    const pal = PAL[dark ? 'dark' : 'light'];
    const cls = new Int16Array(n);
    let classes = [];
    let legend = null;
    if (this.mode === 'coverage') {
      classes = [
        { label: 'Surveyed', color: pal.s1 },
        { label: 'Not surveyed (LIDAR / design ground)', color: pal.other },
        { label: 'No elevation', color: pal.s2 },
      ];
      for (let i = 0; i < n; i++) cls[i] = p.zs[i] != null ? 0 : p.zl[i] != null || p.z[i] != null ? 1 : 2;
    } else if (this.mode === 'date') {
      const today = Math.floor(Date.now() / 86400000);
      // One hue, the most recent the most visible: darkest on a light
      // surface, lightest on a dark one.
      const steps = dark ? [BLUE[0], BLUE[2], BLUE[3], BLUE[5]] : [BLUE[6], BLUE[4], BLUE[2], BLUE[0]];
      classes = [
        { label: 'Today', color: steps[0] },
        { label: 'Last 7 days', color: steps[1] },
        { label: 'Last 30 days', color: steps[2] },
        { label: 'Earlier', color: steps[3] },
        { label: 'Not surveyed', color: pal.dim },
      ];
      for (let i = 0; i < n; i++) {
        if (p.zs[i] == null) { cls[i] = 4; continue; }
        const age = today - dayNumber(p.zdate[i]);
        cls[i] = age <= 0 ? 0 : age <= 7 ? 1 : age <= 30 ? 2 : 3;
      }
    } else if (this.mode === 'status') {
      const keys = Object.keys(STATUS);
      classes = keys.map((k) => ({ label: STATUS[k][1], color: STATUS[k][0] }));
      const status = this.state.trackers.map((t) => keys.indexOf(t.status));
      for (let i = 0; i < n; i++) cls[i] = Math.max(0, status[this.data.trackerOf[i]]);
    } else if (this.mode === 'installers') {
      const inst = new Map((this.summary.installers || []).map((b) => [b.block, b]));
      classes = [
        { label: 'File sent to the installers', color: pal.good },
        { label: 'Complete, file not sent yet', color: pal.s1 },
        { label: 'Not complete yet', color: pal.dim },
      ];
      for (let i = 0; i < n; i++) {
        const b = inst.get(p.block[i]);
        cls[i] = b?.last_export && !b.needs_export ? 0 : b?.ready ? 1 : 2;
      }
    } else {
      // Continuous: reveal around its nominal value, or survey − LIDAR around 0.
      const reveal = this.mode === 'reveal';
      const val = new Float64Array(n).fill(NaN);
      for (let i = 0; i < n; i++) {
        if (reveal) { if (p.rev[i] != null) val[i] = p.rev[i] - s.reveal_nominal; } else if (p.zs[i] != null && p.zl[i] != null) val[i] = p.zs[i] - p.zl[i];
      }
      let span;
      if (reveal) span = Math.max(Math.abs(s.delta_min), Math.abs(s.delta_max));
      else {
        const a = [...val].filter((v) => !Number.isNaN(v)).map(Math.abs).sort((u, v) => u - v);
        span = Math.min(0.5, Math.max(0.05, a.length ? a[Math.floor(a.length * 0.95)] : 0.1));
      }
      const stops = [DIV_NEG[0], DIV_NEG[1], DIV_NEG[2], pal.mid, DIV_POS[0], DIV_POS[1], DIV_POS[2]];
      for (let b = 0; b < BUCKETS; b++) classes.push({ color: rampColor(stops, b / (BUCKETS - 1)), ramp: true });
      classes.push({ label: reveal ? 'No result' : 'Not surveyed', color: pal.dim });
      for (let i = 0; i < n; i++) {
        cls[i] = Number.isNaN(val[i]) ? BUCKETS : Math.round(((Math.max(-span, Math.min(span, val[i])) / span + 1) / 2) * (BUCKETS - 1));
      }
      const f = (v) => v.toFixed(2);
      legend = reveal
        ? { stops, lo: f(s.reveal_nominal - span), hi: f(s.reveal_nominal + span), title: `Reveal (m), nominal ${f(s.reveal_nominal)}` }
        : { stops, lo: `−${f(span)}`, hi: `+${f(span)}`, title: 'Survey − LIDAR (m): blue = survey lower, red = survey higher' };
    }
    const members = classes.map(() => []);
    for (let i = 0; i < n; i++) members[cls[i]].push(i);
    classes.forEach((c, k) => { c.count = members[k].length; });
    this.cls = cls;
    this.classes = classes;
    this.members = members;
    this.rampLegend = legend;
    this.renderLegend();
  }

  renderLegend() {
    const n = this.data.n;
    const pct = (c) => (n ? Math.round((100 * c) / n) : 0);
    let html = `<div class="title">${esc(MODES[this.mode])}</div>`;
    if (this.rampLegend) {
      const r = this.rampLegend;
      html += `<div class="ramp" style="background:linear-gradient(90deg,${r.stops.join(',')})"></div><div class="ramp-labels"><span>${esc(r.lo)}</span><span>${esc(r.hi)}</span></div><div class="muted" style="margin:4px 0">${esc(r.title)}</div>`;
    }
    this.classes.forEach((c, k) => {
      if (c.ramp || !c.count) return;
      html += `<div class="item${this.hidden.has(k) ? ' off' : ''}" data-k="${k}" title="Click to hide or show"><span class="sw" style="background:${c.color}"></span>${esc(c.label)} <b class="num">${c.count.toLocaleString()}</b> <span class="muted">${pct(c.count)}%</span></div>`;
    });
    this.legendEl.innerHTML = html;
    this.legendEl.querySelectorAll('[data-k]').forEach((el) => el.addEventListener('click', () => {
      const k = +el.dataset.k;
      if (this.hidden.has(k)) this.hidden.delete(k); else this.hidden.add(k);
      this.renderLegend();
      this.draw();
    }));
  }

  // ------------------------------------------------------------------ view

  isDark() {
    const bg = css('--surface') || '#fcfcfb';
    const [r, g, b] = bg.startsWith('#') ? hexRgb(bg.length === 4 ? `#${[...bg.slice(1)].map((c) => c + c).join('')}` : bg) : [252, 252, 251];
    return 0.2126 * r + 0.7152 * g + 0.0722 * b < 128;
  }

  resize() {
    const r = this.host.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const dpr = window.devicePixelRatio || 1;
    this.w = r.width;
    this.h = r.height;
    this.canvas.width = Math.round(r.width * dpr);
    this.canvas.height = Math.round(r.height * dpr);
    this.canvas.style.width = `${r.width}px`;
    this.canvas.style.height = `${r.height}px`;
    this.dpr = dpr;
    this.draw();
  }

  fitBox(x0, y0, x1, y1) {
    if (!this.w) return;
    const pad = 40;
    this.view.cx = (x0 + x1) / 2;
    this.view.cy = (y0 + y1) / 2;
    this.view.scale = Math.min((this.w - 2 * pad) / Math.max(1, x1 - x0), (this.h - 2 * pad) / Math.max(1, y1 - y0));
    this.draw();
  }

  fit() {
    if (!this.data) return;
    if (!this.w) this.resize();
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const b of this.data.blocks.values()) { x0 = Math.min(x0, b.x0); y0 = Math.min(y0, b.y0); x1 = Math.max(x1, b.x1); y1 = Math.max(y1, b.y1); }
    this.fitBox(x0, y0, x1, y1);
  }

  setBlock(block) {
    this.focusBlock = block;
    const b = block ? this.data?.blocks.get(block) : null;
    if (b) this.fitBox(b.x0 - 20, b.y0 - 20, b.x1 + 20, b.y1 + 20);
    else this.fit();
  }

  /** Zoom to a tracker or a pile by ID; selects its (first) pile. */
  find(text) {
    const q = text.trim().toLowerCase();
    if (!q || !this.data) return false;
    const p = this.state.piles;
    let i = p.id.findIndex((id) => id.toLowerCase() === q);
    if (i < 0) {
      const t = this.state.trackers.find((t) => t.tracker.toLowerCase() === q);
      if (t) i = t.first_pile;
    }
    if (i < 0) return false;
    const t = this.state.trackers[this.data.trackerOf[i]];
    const { x, y } = this.data;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (let k = t.first_pile; k < t.first_pile + t.n_piles; k++) { x0 = Math.min(x0, x[k]); y0 = Math.min(y0, y[k]); x1 = Math.max(x1, x[k]); y1 = Math.max(y1, y[k]); }
    this.selected = i;
    this.fitBox(x0 - 60, y0 - 60, x1 + 60, y1 + 60);
    this.onPick(i);
    return true;
  }

  toScreen(x, y) {
    const v = this.view;
    return [(x - v.cx) * v.scale + this.w / 2, this.h / 2 - (y - v.cy) * v.scale];
  }

  toWorld(sx, sy) {
    const v = this.view;
    return [(sx - this.w / 2) / v.scale + v.cx, (this.h / 2 - sy) / v.scale + v.cy];
  }

  draw() {
    if (!this.data || !this.w || !this.active) return;
    cancelAnimationFrame(this.frame);
    this.frame = requestAnimationFrame(() => this.paint());
  }

  paint() {
    const { ctx, data } = this;
    const { x, y } = data;
    const v = this.view;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.fillStyle = css('--surface') || '#fcfcfb';
    ctx.fillRect(0, 0, this.w, this.h);
    const [wx0, wy1] = this.toWorld(-10, -10);
    const [wx1, wy0] = this.toWorld(this.w + 10, this.h + 10);
    const visible = (i) => x[i] >= wx0 && x[i] <= wx1 && y[i] >= wy0 && y[i] <= wy1;
    const focus = this.focusBlock;
    const p = this.state.piles;

    // Tracker lines, once trackers are long enough on screen (faded outside
    // the chosen block).
    if (this.opts.lines && v.scale > 0.6) {
      ctx.strokeStyle = css('--axis') || '#c3c2b7';
      ctx.lineWidth = 1;
      for (const pass of focus ? [false, true] : [true]) {
        ctx.globalAlpha = pass ? 1 : 0.3;
        ctx.beginPath();
        for (const t of this.state.trackers) {
          const a = t.first_pile, b = t.first_pile + t.n_piles - 1;
          if (!(visible(a) || visible(b)) || (focus && (t.block === focus) !== pass)) continue;
          const [x0, y0] = this.toScreen(x[a], y[a]);
          const [x1, y1] = this.toScreen(x[b], y[b]);
          ctx.moveTo(x0, y0);
          ctx.lineTo(x1, y1);
        }
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    }

    this.paintPiles(visible, focus);

    // Block outlines, over the piles (the chosen block's stands out).
    if (this.opts.outlines && this.outlines.length) {
      const ink = css('--ink-2') || '#52514e';
      for (const o of this.outlines) {
        const chosen = focus && o.block === focus;
        ctx.strokeStyle = chosen ? css('--ink') || '#0b0b0b' : ink;
        ctx.globalAlpha = focus && !chosen ? 0.35 : 0.85;
        ctx.lineWidth = chosen ? 2.5 : 1.5;
        ctx.beginPath();
        for (const ring of o.rings) {
          ring.forEach(([rx, ry], k) => {
            const [sx, sy] = this.toScreen(rx, ry);
            if (k === 0) ctx.moveTo(sx, sy); else ctx.lineTo(sx, sy);
          });
          ctx.closePath();
        }
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    }

    // Block labels when zoomed out.
    if (this.opts.labels && v.scale < 2.5) {
      ctx.font = '600 13px system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.lineWidth = 4;
      ctx.strokeStyle = css('--surface') || '#fcfcfb';
      ctx.fillStyle = css('--ink') || '#0b0b0b';
      for (const [name, b] of data.blocks) {
        if (!name) continue;
        const [sx, sy] = this.toScreen(b.sx / b.n, b.sy / b.n);
        if (sx < -50 || sy < -20 || sx > this.w + 50 || sy > this.h + 20) continue;
        ctx.strokeText(name, sx, sy);
        ctx.fillText(name, sx, sy);
      }
    }

    // Selected pile and its tracker.
    if (this.selected >= 0) {
      const t = this.state.trackers[data.trackerOf[this.selected]];
      ctx.strokeStyle = css('--ink') || '#0b0b0b';
      ctx.lineWidth = 2;
      const a = t.first_pile, b = t.first_pile + t.n_piles - 1;
      const [x0, y0] = this.toScreen(x[a], y[a]);
      const [x1, y1] = this.toScreen(x[b], y[b]);
      ctx.beginPath(); ctx.moveTo(x0, y0); ctx.lineTo(x1, y1); ctx.stroke();
      const [sx, sy] = this.toScreen(x[this.selected], y[this.selected]);
      ctx.beginPath(); ctx.arc(sx, sy, this.radius() + 4, 0, 2 * Math.PI); ctx.stroke();
    }

    // Scale bar.
    const target = 120 / v.scale;
    const pow = 10 ** Math.floor(Math.log10(target));
    const len = [1, 2, 5, 10].map((k) => k * pow).filter((m) => m <= target).pop() || pow;
    const px = len * v.scale;
    const bx = this.w - px - 16, by = this.h - 18;
    ctx.strokeStyle = css('--ink-2') || '#52514e';
    ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.moveTo(bx, by - 4); ctx.lineTo(bx, by); ctx.lineTo(bx + px, by); ctx.lineTo(bx + px, by - 4); ctx.stroke();
    ctx.fillStyle = css('--ink-2') || '#52514e';
    ctx.font = '12px system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'bottom';
    ctx.fillText(len >= 1000 ? `${len / 1000} km` : `${len} m`, bx + px / 2, by - 5);
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    ctx.font = '600 13px system-ui, sans-serif';
    ctx.fillText('N ↑', 14, 12);
  }

  /** Piles, one batch per colour (faded outside the chosen block). */
  paintPiles(visible, focus) {
    const { ctx } = this;
    const { x, y } = this.data;
    const p = this.state.piles;
    const r = this.radius();
    const round = r >= 2.5;
    this.classes.forEach((c, k) => {
      if (this.hidden.has(k) || !this.members[k].length) return;
      for (const pass of focus ? [false, true] : [true]) {
        ctx.globalAlpha = pass ? 1 : 0.22;
        ctx.fillStyle = c.color;
        ctx.beginPath();
        for (const i of this.members[k]) {
          if (!visible(i) || (focus && (p.block[i] === focus) !== pass)) continue;
          const [sx, sy] = this.toScreen(x[i], y[i]);
          if (round) { ctx.moveTo(sx + r, sy); ctx.arc(sx, sy, r, 0, 2 * Math.PI); } else ctx.rect(sx - r, sy - r, 2 * r, 2 * r);
        }
        ctx.fill();
      }
    });
    ctx.globalAlpha = 1;
  }

  // ------------------------------------------------------------------ pointer

  /** Pile dot radius (px): grows with the zoom, up to a readable size. */
  radius() {
    return Math.min(7, Math.max(1.1, 0.9 * this.view.scale));
  }

  pileAt(sx, sy) {
    if (!this.data) return -1;
    const [wx, wy] = this.toWorld(sx, sy);
    const rad = Math.max(8, this.radius() + 4) / this.view.scale;
    const { x, y, grid } = this.data;
    let best = -1, bd = rad * rad;
    for (let gx = Math.floor((wx - rad) / 10); gx <= Math.floor((wx + rad) / 10); gx++) {
      for (let gy = Math.floor((wy - rad) / 10); gy <= Math.floor((wy + rad) / 10); gy++) {
        for (const i of grid.get(`${gx},${gy}`) || []) {
          if (this.hidden.has(this.cls[i])) continue;
          const d = (x[i] - wx) ** 2 + (y[i] - wy) ** 2;
          if (d < bd) { bd = d; best = i; }
        }
      }
    }
    return best;
  }

  zoomAt(sx, sy, factor) {
    const [wx, wy] = this.toWorld(sx, sy);
    const v = this.view;
    v.scale = Math.min(200, Math.max(0.005, v.scale * factor));
    v.cx = wx - (sx - this.w / 2) / v.scale;
    v.cy = wy + (sy - this.h / 2) / v.scale;
    this.draw();
  }

  zoom(factor) {
    this.zoomAt(this.w / 2, this.h / 2, factor);
  }

  bindPointer() {
    const c = this.canvas;
    const pointers = new Map();
    let moved = 0, pinch = null;
    const local = (e) => { const r = c.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    c.addEventListener('pointerdown', (e) => {
      c.setPointerCapture(e.pointerId);
      pointers.set(e.pointerId, local(e));
      moved = 0;
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]) };
      }
    });
    c.addEventListener('pointermove', (e) => {
      const pos = local(e);
      if (!pointers.has(e.pointerId)) { this.hover(pos); return; }
      const prev = pointers.get(e.pointerId);
      pointers.set(e.pointerId, pos);
      if (pointers.size === 2 && pinch) {
        const [a, b] = [...pointers.values()];
        const d = Math.hypot(a[0] - b[0], a[1] - b[1]);
        this.zoomAt((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, d / pinch.d);
        pinch.d = d;
        moved += 10;
        return;
      }
      const dx = pos[0] - prev[0], dy = pos[1] - prev[1];
      moved += Math.abs(dx) + Math.abs(dy);
      this.view.cx -= dx / this.view.scale;
      this.view.cy += dy / this.view.scale;
      this.tip.hidden = true;
      this.draw();
    });
    const end = (e) => {
      if (pointers.size === 1 && moved < 5 && e.type === 'pointerup') {
        const i = this.pileAt(...local(e));
        this.selected = i;
        this.draw();
        if (i >= 0) this.onPick(i);
      }
      pointers.delete(e.pointerId);
      if (pointers.size < 2) pinch = null;
    };
    c.addEventListener('pointerup', end);
    c.addEventListener('pointercancel', end);
    c.addEventListener('pointerleave', () => { this.tip.hidden = true; });
    c.addEventListener('wheel', (e) => {
      e.preventDefault();
      this.zoomAt(...local(e), Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0015)));
    }, { passive: false });
    c.addEventListener('dblclick', (e) => this.zoomAt(...local(e), 2));
  }

  hover([sx, sy]) {
    const i = this.pileAt(sx, sy);
    this.canvas.style.cursor = i >= 0 ? 'pointer' : 'grab';
    if (i < 0) { this.tip.hidden = true; return; }
    const p = this.state.piles;
    const t = this.state.trackers[this.data.trackerOf[i]];
    const c = this.classes[this.cls[i]];
    const detail = {
      coverage: p.zs[i] != null ? `surveyed ${p.zdate[i] ?? ''}` : c.label,
      date: p.zs[i] != null ? `surveyed ${p.zdate[i] ?? ''}` : 'not surveyed',
      status: STATUS[t.status]?.[1] ?? t.status,
      reveal: p.rev[i] != null ? `reveal ${p.rev[i].toFixed(2)} m` : 'no result',
      diff: p.zs[i] != null && p.zl[i] != null ? `survey − LIDAR ${(p.zs[i] - p.zl[i]).toFixed(2)} m` : 'not surveyed',
      installers: c.label,
    }[this.mode];
    this.tip.innerHTML = `<b>${esc(p.id[i])}</b><div class="row"><span>Tracker</span><span>${esc(t.tracker)}</span></div><div class="row"><span>Block</span><span>${esc(p.block[i] || '–')}</span></div><div class="row"><span><span class="sw-dot" style="display:inline-block;background:${c.color}"></span></span><span>${esc(detail)}</span></div>`;
    this.tip.hidden = false;
    const tw = this.tip.offsetWidth, th = this.tip.offsetHeight;
    this.tip.style.left = `${Math.min(this.w - tw - 8, sx + 14)}px`;
    this.tip.style.top = `${Math.max(8, Math.min(this.h - th - 8, sy + 14))}px`;
  }

  /** The map as a PNG file (Blob), with its title and legend drawn in. */
  async image() {
    const out = document.createElement('canvas');
    out.width = this.canvas.width;
    out.height = this.canvas.height;
    const g = out.getContext('2d');
    g.drawImage(this.canvas, 0, 0);
    g.scale(this.dpr, this.dpr);
    const rows = this.classes.filter((c, k) => !c.ramp && c.count && !this.hidden.has(k));
    const h = 28 + 18 * rows.length;
    g.fillStyle = css('--surface') || '#fff';
    g.globalAlpha = 0.92;
    g.fillRect(8, 8, 300, h);
    g.globalAlpha = 1;
    g.fillStyle = css('--ink') || '#000';
    g.font = '600 13px system-ui, sans-serif';
    g.textBaseline = 'middle';
    g.fillText(`${MODES[this.mode]}${this.focusBlock ? ` · block ${this.focusBlock}` : ''} · ${new Date().toISOString().slice(0, 10)}`, 16, 22);
    g.font = '12px system-ui, sans-serif';
    rows.forEach((c, k) => {
      g.fillStyle = c.color;
      g.fillRect(16, 36 + 18 * k, 10, 10);
      g.fillStyle = css('--ink') || '#000';
      g.fillText(`${c.label}: ${c.count.toLocaleString()}`, 32, 41 + 18 * k);
    });
    return new Promise((resolve) => out.toBlob(resolve, 'image/png'));
  }
}
