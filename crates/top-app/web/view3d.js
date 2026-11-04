// 3D view of the site: ground mesh triangulated through the pile ground
// elevations, each pile drawn from its (new) ground to its top, trackers as
// lines through the tops of their piles.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/OrbitControls.js';

// Reference palette (see the README): status colours are reserved for status.
const STATUS = {
  Compliant: { color: '#0ca30c', label: 'Compliant' },
  Graded: { color: '#fab219', label: 'Graded (needs cut/fill)' },
  NonCompliant: { color: '#d03b3b', label: 'Non-compliant' },
  Skipped: { color: '#ec835a', label: 'Skipped (< 3 piles)' },
  InvalidInput: { color: '#ec835a', label: 'Invalid input' },
  AwaitingSurvey: { color: '#898781', label: 'Awaiting elevations' },
  NotRun: { color: '#898781', label: 'Not optimised yet' },
};
const CAT = {
  light: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#6250d6'],
  dark: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9'],
};
const OTHER = '#898781';
const BLUE = ['#cde2fb', '#9ec5f4', '#6da7ec', '#3987e5', '#256abf', '#184f95', '#0d366b'];
// Ground: one low-chroma sand -> brown hue, light -> dark, so it reads as
// terrain without competing with the pile colours drawn on top of it.
const TERRAIN = ['#f3efe6', '#e2dacb', '#cfc3ab', '#b8a888', '#9c8a69', '#7d6c4f', '#5c4f39'];
const DIV_NEG = ['#86b6ef', '#2a78d6', '#184f95'];
const DIV_POS = ['#f0a3a2', '#e34948', '#a32c2b'];
const FRENCH = { rouge: 'red', bleu: 'blue', vert: 'green', jaune: 'yellow', noir: 'black', blanc: 'white', gris: 'grey', rose: 'pink', marron: 'brown', violet: 'violet', orange: 'orange', cyan: 'cyan', beige: 'beige' };

export function isDark() {
  const t = document.documentElement.dataset.theme;
  if (t === 'dark') return true;
  if (t === 'light') return false;
  return matchMedia('(prefers-color-scheme: dark)').matches;
}

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function hexRgb(hex) {
  const c = new THREE.Color(hex);
  return [c.r, c.g, c.b];
}

function rampColor(stops, t) {
  t = Math.min(1, Math.max(0, t));
  const f = t * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(f));
  const a = new THREE.Color(stops[i]), b = new THREE.Color(stops[i + 1]);
  a.lerp(b, f - i);
  return [a.r, a.g, a.b];
}

function divColor(t, mid) {
  // t in [-1, 1]
  if (t < 0) return rampColor([mid, ...DIV_NEG], -t);
  return rampColor([mid, ...DIV_POS], t);
}

const colorParser = document.createElement('canvas').getContext('2d');
function cssColor(name) {
  if (!name) return null;
  const key = name.trim().toLowerCase();
  const v = FRENCH[key] || key;
  colorParser.fillStyle = '#010203';
  colorParser.fillStyle = v;
  const out = colorParser.fillStyle;
  return out === '#010203' && v !== '#010203' ? null : out;
}

function dotTexture(ring = false) {
  const c = document.createElement('canvas');
  c.width = c.height = 64;
  const g = c.getContext('2d');
  g.beginPath();
  if (ring) {
    g.arc(32, 32, 24, 0, Math.PI * 2);
    g.lineWidth = 10;
    g.strokeStyle = '#fff';
    g.stroke();
  } else {
    g.arc(32, 32, 28, 0, Math.PI * 2);
    g.fillStyle = '#fff';
    g.fill();
  }
  return new THREE.CanvasTexture(c);
}

export class View3D {
  constructor(container, legendEl, onPick) {
    this.container = container;
    this.legendEl = legendEl;
    this.onPick = onPick;
    this.opts = { colorBy: 'status', ground: 'elevation', wire: false, water: true, ex: 3, block: '' };
    this.hidden = new Set();
    this.triCache = new Map();
    this.active = false;
    this.needsRender = true;

    this.renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    container.prepend(this.renderer.domElement);
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(45, 1, 0.5, 500000);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.12;
    this.controls.maxPolarAngle = Math.PI * 0.495;
    this.controls.addEventListener('change', () => (this.needsRender = true));
    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x777777, 2.0));
    const sun = new THREE.DirectionalLight(0xffffff, 1.4);
    sun.position.set(0.6, 1, 0.3);
    this.scene.add(sun);
    this.root = new THREE.Group();
    this.scene.add(this.root);
    this.dot = dotTexture();
    this.ring = dotTexture(true);

    new ResizeObserver(() => this.resize()).observe(container);
    let down = null;
    const canvas = this.renderer.domElement;
    canvas.addEventListener('pointerdown', (e) => (down = [e.clientX, e.clientY]));
    canvas.addEventListener('pointerup', (e) => {
      if (down && Math.hypot(e.clientX - down[0], e.clientY - down[1]) < 5) this.pick(e);
      down = null;
    });
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => this.build());
    const loop = () => {
      requestAnimationFrame(loop);
      if (!this.active) return;
      const moved = this.controls.update();
      if (moved || this.needsRender) {
        this.renderer.render(this.scene, this.camera);
        this.needsRender = false;
      }
    };
    loop();
  }

  setActive(on) {
    this.active = on;
    if (on) { this.resize(); this.needsRender = true; }
  }

  resize() {
    const w = this.container.clientWidth, h = this.container.clientHeight;
    if (!w || !h) return;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.needsRender = true;
  }

  setData(data, settings) {
    this.data = data;
    this.settings = settings;
    const p = data.piles;
    const n = p.id.length;
    this.pileTracker = new Int32Array(n).fill(-1);
    data.trackers.forEach((t, ti) => {
      for (let i = t.first_pile; i < t.first_pile + t.n_piles; i++) this.pileTracker[i] = ti;
    });
    this.types = [...new Set(p.type)].sort();
    // Site colour of each pile type, from the master list's Color column. Used
    // when every pile of a type has the same, valid colour and types differ.
    const byType = new Map();
    p.type.forEach((t, i) => {
      if (!byType.has(t)) byType.set(t, new Set());
      byType.get(t).add(p.color[i]);
    });
    const site = new Map();
    for (const [t, set] of byType) {
      const c = set.size === 1 ? cssColor([...set][0]) : null;
      if (c) site.set(t, { css: c, name: [...set][0] });
    }
    const distinct = new Set([...site.values()].map((v) => v.css)).size === site.size;
    this.typeColors = site.size === byType.size && distinct ? site : null;
    this.triCache.clear();
    const first = !this.built;
    this.build();
    if (first) this.fit();
  }

  setOptions(o) {
    const blockChanged = 'block' in o && o.block !== this.opts.block;
    const modeChanged = 'colorBy' in o && o.colorBy !== this.opts.colorBy;
    Object.assign(this.opts, o);
    if (modeChanged) this.hidden.clear();
    this.build();
    if (blockChanged) this.fit();
  }

  groundOf(i) {
    const p = this.data.piles;
    return p.ng[i] ?? p.z[i];
  }

  /** Indices of piles shown (block filter). */
  subset() {
    const p = this.data.piles;
    const out = [];
    for (let i = 0; i < p.id.length; i++) if (!this.opts.block || p.block[i] === this.opts.block) out.push(i);
    return out;
  }

  pileColor(i, ctx) {
    const p = this.data.piles, s = this.settings;
    const t = this.data.trackers[this.pileTracker[i]];
    const dark = ctx.dark;
    switch (this.opts.colorBy) {
      case 'status': {
        const k = t?.status ?? 'NotRun';
        return { key: k, rgb: hexRgb((STATUS[k] ?? STATUS.NotRun).color) };
      }
      case 'angle': {
        if (p.ang[i] == null) return { key: 'none', rgb: hexRgb(OTHER) };
        const a = Math.abs(p.ang[i]);
        if (a > s.max_angle_deg + 0.001) return { key: 'over', rgb: hexRgb(STATUS.NonCompliant.color) };
        const ramp = dark ? BLUE.slice().reverse() : BLUE;
        return { key: 'ok', rgb: rampColor(ramp.slice(1), a / s.max_angle_deg) };
      }
      case 'reveal': {
        if (p.rev[i] == null) return { key: 'none', rgb: hexRgb(OTHER) };
        const span = Math.max(Math.abs(s.delta_min), Math.abs(s.delta_max)) || 0.1;
        return { key: 'v', rgb: divColor((p.rev[i] - s.reveal_nominal) / span, ctx.mid) };
      }
      case 'cutfill': {
        if (p.cf[i] == null) return { key: 'none', rgb: hexRgb(OTHER) };
        return { key: 'v', rgb: divColor(p.cf[i] / ctx.cfSpan, ctx.mid) };
      }
      case 'source': {
        const k = p.zsrc[i] ?? 'none';
        const pal = dark ? CAT.dark : CAT.light;
        return { key: k, rgb: hexRgb(k === 'survey' ? pal[0] : k === 'lidar' ? pal[1] : OTHER) };
      }
      case 'type': {
        if (this.typeColors) return { key: p.type[i], rgb: hexRgb(this.typeColors.get(p.type[i]).css) };
        const idx = this.types.indexOf(p.type[i]);
        const pal = dark ? CAT.dark : CAT.light;
        const k = idx >= 0 && idx < pal.length ? p.type[i] : 'Other';
        return { key: k, rgb: hexRgb(k === 'Other' ? OTHER : pal[idx]) };
      }
      case 'flood': {
        const pal = dark ? CAT.dark : CAT.light;
        if (p.flood[i] == null) return { key: 'none', rgb: hexRgb(OTHER) };
        if (p.mta[i]) return { key: 'raised', rgb: hexRgb(pal[6]) };
        return { key: 'clear', rgb: hexRgb(pal[2]) };
      }
    }
    return { key: '', rgb: [0.5, 0.5, 0.5] };
  }

  triangulate(idx, coords) {
    const key = this.opts.block;
    if (this.triCache.has(key)) return this.triCache.get(key);
    const del = new globalThis.Delaunator(coords);
    const tri = del.triangles;
    const lens = [];
    const L = (a, b) => Math.hypot(coords[2 * a] - coords[2 * b], coords[2 * a + 1] - coords[2 * b + 1]);
    for (let t = 0; t < tri.length; t += 3) {
      const a = tri[t], b = tri[t + 1], c = tri[t + 2];
      lens.push(Math.max(L(a, b), L(b, c), L(c, a)));
    }
    const sorted = Float64Array.from(lens).sort();
    const limit = 2.5 * (sorted[Math.floor(sorted.length / 2)] || 1);
    const keep = [];
    for (let t = 0, k = 0; t < tri.length; t += 3, k++) if (lens[k] <= limit) keep.push(tri[t], tri[t + 1], tri[t + 2]);
    const out = { index: keep };
    this.triCache.set(key, out);
    return out;
  }

  clear() {
    for (const o of [...this.root.children]) {
      this.root.remove(o);
      o.geometry?.dispose();
      o.material?.dispose();
    }
  }

  build() {
    if (!this.data) return;
    this.built = true;
    this.clear();
    const p = this.data.piles, s = this.settings;
    const ex = this.opts.ex;
    const dark = isDark();
    this.scene.background = new THREE.Color(css('--surface') || (dark ? '#1a1a19' : '#fcfcfb'));
    const idx = this.subset().filter((i) => this.groundOf(i) != null);
    this.shown = idx;
    if (!idx.length) { this.renderLegend({}, {}); this.needsRender = true; return; }

    let e0 = Infinity, n0 = Infinity, z0 = Infinity, cfMax = 0;
    for (const i of idx) {
      e0 = Math.min(e0, p.e[i]); n0 = Math.min(n0, p.n[i]); z0 = Math.min(z0, this.groundOf(i), p.z[i] ?? Infinity);
      if (p.cf[i] != null) cfMax = Math.max(cfMax, Math.abs(p.cf[i]));
    }
    this.origin = { e0, n0, z0 };
    const X = (i) => p.e[i] - e0;
    const Z = (i) => -(p.n[i] - n0);
    const Y = (v) => (v - z0) * ex;
    const ctx = { dark, mid: dark ? '#383835' : '#e1e0d9', cfSpan: Math.min(0.5, Math.max(0.05, cfMax)) };
    this.ctx = ctx;
    const dim = hexRgb(dark ? '#383835' : '#d6d5cf');

    // Pile colours (and legend counts).
    const colors = new Map();
    const counts = {};
    for (const i of idx) {
      const c = this.pileColor(i, ctx);
      counts[c.key] = (counts[c.key] ?? 0) + 1;
      colors.set(i, this.hidden.has(c.key) ? dim : c.rgb);
    }

    // Ground mesh.
    if (this.opts.ground !== 'none') {
      const coords = new Float64Array(idx.length * 2);
      idx.forEach((i, k) => { coords[2 * k] = X(i); coords[2 * k + 1] = Z(i); });
      const { index } = this.triangulate(idx, coords);
      const pos = new Float32Array(idx.length * 3);
      const col = new Float32Array(idx.length * 3);
      let gMin = Infinity, gMax = -Infinity;
      for (const i of idx) { gMin = Math.min(gMin, this.groundOf(i)); gMax = Math.max(gMax, this.groundOf(i)); }
      this.groundRange = [gMin, gMax];
      const ramp = dark ? TERRAIN.slice().reverse() : TERRAIN;
      idx.forEach((i, k) => {
        pos.set([X(i), Y(this.groundOf(i)), Z(i)], 3 * k);
        const c = this.opts.ground === 'cutfill'
          ? divColor((p.cf[i] ?? 0) / ctx.cfSpan, ctx.mid)
          : rampColor(ramp.slice(0, 6), (this.groundOf(i) - gMin) / ((gMax - gMin) || 1));
        col.set(c, 3 * k);
      });
      const g = new THREE.BufferGeometry();
      g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
      g.setAttribute('color', new THREE.BufferAttribute(col, 3));
      g.setIndex(index);
      g.computeVertexNormals();
      const mesh = new THREE.Mesh(g, new THREE.MeshLambertMaterial({
        vertexColors: true, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: 1, polygonOffsetUnits: 1,
        transparent: true, opacity: 0.92,
      }));
      this.root.add(mesh);
      if (this.opts.wire) {
        const w = new THREE.LineSegments(new THREE.WireframeGeometry(g), new THREE.LineBasicMaterial({ color: dark ? 0x52514e : 0x898781, transparent: true, opacity: 0.45 }));
        this.root.add(w);
      }
    }

    // Flood water: translucent surface at the flood water level, over the
    // triangles whose three piles all have a flood value above their ground.
    this.hasWater = false;
    if (this.opts.water && idx.some((i) => p.flvl[i] != null)) {
      const coords = new Float64Array(idx.length * 2);
      idx.forEach((i, k) => { coords[2 * k] = X(i); coords[2 * k + 1] = Z(i); });
      const { index } = this.triangulate(idx, coords);
      const wet = idx.map((i) => p.flvl[i] != null && p.flvl[i] > this.groundOf(i) + 0.005);
      const keep = [];
      for (let t = 0; t < index.length; t += 3) {
        if (wet[index[t]] && wet[index[t + 1]] && wet[index[t + 2]]) keep.push(index[t], index[t + 1], index[t + 2]);
      }
      if (keep.length) {
        const pos = new Float32Array(idx.length * 3);
        idx.forEach((i, k) => pos.set([X(i), Y(p.flvl[i] ?? this.groundOf(i)), Z(i)], 3 * k));
        const g = new THREE.BufferGeometry();
        g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
        g.setIndex(keep);
        g.computeVertexNormals();
        const water = new THREE.Mesh(g, new THREE.MeshLambertMaterial({
          color: dark ? 0x3987e5 : 0x2a78d6, transparent: true, opacity: 0.35, side: THREE.DoubleSide, depthWrite: false,
        }));
        water.renderOrder = 2;
        this.root.add(water);
        this.hasWater = true;
      }
    }

    // Piles (shaft from ground to top), tops (points, pickable), trackers (lines).
    const shaft = [], shaftCol = [], tops = [], topCol = [];
    this.pointPile = [];
    for (const i of idx) {
      const base = this.groundOf(i);
      const top = p.top[i] ?? (p.z[i] != null ? p.z[i] + s.reveal_nominal : null);
      if (top == null) continue;
      const c = colors.get(i);
      shaft.push(X(i), Y(base), Z(i), X(i), Y(top), Z(i));
      shaftCol.push(...c, ...c);
      tops.push(X(i), Y(top), Z(i));
      topCol.push(...c);
      this.pointPile.push(i);
    }
    const lines = [], lineCol = [];
    const shownSet = new Set(idx);
    for (const t of this.data.trackers) {
      let prev = -1;
      for (let i = t.first_pile; i < t.first_pile + t.n_piles; i++) {
        if (!shownSet.has(i) || p.top[i] == null) { prev = -1; continue; }
        if (prev >= 0) {
          lines.push(X(prev), Y(p.top[prev]), Z(prev), X(i), Y(p.top[i]), Z(i));
          lineCol.push(...colors.get(prev), ...colors.get(i));
        }
        prev = i;
      }
    }
    const seg = (arr, carr, mat) => {
      const g = new THREE.BufferGeometry();
      g.setAttribute('position', new THREE.Float32BufferAttribute(arr, 3));
      g.setAttribute('color', new THREE.Float32BufferAttribute(carr, 3));
      return new THREE.LineSegments(g, mat);
    };
    if (shaft.length) this.root.add(seg(shaft, shaftCol, new THREE.LineBasicMaterial({ vertexColors: true })));
    if (lines.length) this.root.add(seg(lines, lineCol, new THREE.LineBasicMaterial({ vertexColors: true })));
    if (tops.length) {
      const g = new THREE.BufferGeometry();
      g.setAttribute('position', new THREE.Float32BufferAttribute(tops, 3));
      g.setAttribute('color', new THREE.Float32BufferAttribute(topCol, 3));
      this.points = new THREE.Points(g, new THREE.PointsMaterial({ size: 7, sizeAttenuation: false, vertexColors: true, map: this.dot, alphaTest: 0.5 }));
      this.root.add(this.points);
    } else {
      this.points = null;
    }
    this.marker = null;
    if (this.selected != null) this.mark(this.selected);
    this.renderLegend(counts, ctx);
    this.needsRender = true;
  }

  renderLegend(counts, ctx) {
    const s = this.settings, dark = isDark();
    const mode = this.opts.colorBy;
    const fmt = (v, d = 2) => (v == null ? '–' : v.toFixed(d));
    let html = '';
    const items = (list) => list
      .filter(([k]) => counts[k])
      .map(([k, color, label]) => `<div class="item${this.hidden.has(k) ? ' off' : ''}" data-key="${encodeURIComponent(k)}" title="Click to hide or show"><span class="sw" style="background:${color}"></span>${label} <span class="muted">${counts[k].toLocaleString()}</span></div>`)
      .join('');
    const ramp = (stops, a, b, title) => `<div class="title">${title}</div><div class="ramp" style="background:linear-gradient(90deg,${stops.join(',')})"></div><div class="ramp-labels"><span>${a}</span><span>${b}</span></div>`;
    if (mode === 'status') {
      html = '<div class="title">Tracker status</div>' + items(Object.entries(STATUS).filter(([k]) => k !== 'InvalidInput').map(([k, v]) => [k, v.color, v.label]).concat([['InvalidInput', STATUS.InvalidInput.color, STATUS.InvalidInput.label]]));
    } else if (mode === 'angle') {
      const r = dark ? BLUE.slice().reverse().slice(1) : BLUE.slice(1);
      html = ramp(r, '0°', `${s.max_angle_deg}°`, 'Angle change at pile (absolute)') +
        `<div style="margin-top:6px">${items([['over', STATUS.NonCompliant.color, `Above ${s.max_angle_deg}° limit`], ['none', OTHER, 'No result yet']])}</div>`;
    } else if (mode === 'reveal') {
      const span = Math.max(Math.abs(s.delta_min), Math.abs(s.delta_max));
      html = ramp([DIV_NEG[2], DIV_NEG[0], ctx.mid, DIV_POS[0], DIV_POS[2]], fmt(s.reveal_nominal - span), fmt(s.reveal_nominal + span), `Reveal (m), nominal ${fmt(s.reveal_nominal)}`) +
        `<div style="margin-top:6px">${items([['none', OTHER, 'No result yet']])}</div>`;
    } else if (mode === 'cutfill') {
      html = ramp([DIV_NEG[2], DIV_NEG[0], ctx.mid, DIV_POS[0], DIV_POS[2]], `cut ${fmt(-ctx.cfSpan)}`, `fill +${fmt(ctx.cfSpan)}`, 'Cut / fill (m)') +
        `<div style="margin-top:6px">${items([['none', OTHER, 'No result yet']])}</div>`;
    } else if (mode === 'source') {
      const pal = dark ? CAT.dark : CAT.light;
      html = '<div class="title">Ground elevation source</div>' + items([['survey', pal[0], 'Field survey'], ['lidar', pal[1], 'LIDAR / design'], ['none', OTHER, 'None']]);
    } else if (mode === 'type') {
      if (this.typeColors) {
        const list = this.types.map((t) => [t, this.typeColors.get(t).css, `${t || '(no type)'} <span class="muted">${this.typeColors.get(t).name.toLowerCase()}</span>`]);
        html = '<div class="title">Pile type (site colours)</div>' + items(list);
      } else {
        const pal = dark ? CAT.dark : CAT.light;
        const list = this.types.slice(0, pal.length).map((t, k) => [t, pal[k], t || '(no type)']);
        list.push(['Other', OTHER, 'Other types']);
        html = '<div class="title">Pile type</div>' + items(list);
      }
    } else if (mode === 'flood') {
      const pal = dark ? CAT.dark : CAT.light;
      html = '<div class="title">Flood</div>' + items([['raised', pal[6], 'Top raised above the flood'], ['clear', pal[2], 'Already above the flood'], ['none', OTHER, 'No flood value']]);
      if (!counts.raised && !counts.clear) html += '<div class="muted" style="margin-top:4px">No flood values imported yet.</div>';
    }
    if (this.hasWater) {
      html += `<div class="item" style="margin-top:6px;cursor:default"><span class="sw" style="background:${dark ? '#3987e5' : '#2a78d6'};opacity:0.5"></span>Flood water surface</div>`;
    }
    if (this.opts.ground !== 'none' && this.groundRange) {
      const [a, b] = this.groundRange;
      html += this.opts.ground === 'elevation'
        ? `<div style="margin-top:10px">${ramp((dark ? TERRAIN.slice().reverse() : TERRAIN).slice(0, 6), fmt(a), fmt(b), 'Ground elevation (m)')}</div>`
        : '';
    }
    this.legendEl.innerHTML = html;
    this.legendEl.querySelectorAll('.item[data-key]').forEach((el) => {
      el.addEventListener('click', () => {
        const k = decodeURIComponent(el.dataset.key);
        if (this.hidden.has(k)) this.hidden.delete(k); else this.hidden.add(k);
        this.build();
      });
    });
  }

  fit(indices) {
    const p = this.data?.piles;
    const idx = indices ?? this.shown;
    if (!p || !idx?.length) return;
    const box = new THREE.Box3();
    const v = new THREE.Vector3();
    for (const i of idx) {
      const g = this.groundOf(i);
      if (g == null) continue;
      v.set(p.e[i] - this.origin.e0, (g - this.origin.z0) * this.opts.ex, -(p.n[i] - this.origin.n0));
      box.expandByPoint(v);
    }
    const c = box.getCenter(new THREE.Vector3());
    const size = Math.max(box.getSize(new THREE.Vector3()).length(), 80);
    this.controls.target.copy(c);
    this.camera.position.set(c.x + size * 0.35, c.y + size * 0.55, c.z + size * 0.75);
    this.camera.near = Math.max(0.1, size / 5000);
    this.camera.far = size * 50;
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.needsRender = true;
  }

  focusTracker(id) {
    const t = this.data?.trackers.find((t) => t.tracker === id);
    if (!t) return false;
    const block = this.data.piles.block[t.first_pile];
    if (this.opts.block && this.opts.block !== block) {
      this.opts.block = '';
      this.build();
    }
    const idx = [];
    for (let i = t.first_pile; i < t.first_pile + t.n_piles; i++) idx.push(i);
    this.fit(idx);
    this.select(t.first_pile);
    return true;
  }

  pick(evt) {
    if (!this.points) return;
    const r = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(((evt.clientX - r.left) / r.width) * 2 - 1, -((evt.clientY - r.top) / r.height) * 2 + 1);
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, this.camera);
    const dist = this.camera.position.distanceTo(this.controls.target);
    ray.params.Points.threshold = (dist * Math.tan((this.camera.fov * Math.PI) / 360) * 2 / r.height) * 8;
    const hits = ray.intersectObject(this.points);
    if (!hits.length) return;
    hits.sort((a, b) => a.distanceToRay - b.distanceToRay || a.distance - b.distance);
    this.select(this.pointPile[hits[0].index]);
  }

  select(i) {
    this.selected = i;
    this.mark(i);
    this.onPick?.(i);
    this.needsRender = true;
  }

  mark(i) {
    if (this.marker) { this.root.remove(this.marker); this.marker.geometry.dispose(); this.marker.material.dispose(); this.marker = null; }
    const p = this.data.piles;
    if (i == null || !this.origin || this.groundOf(i) == null) return;
    const top = p.top[i] ?? p.z[i] + this.settings.reveal_nominal;
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.Float32BufferAttribute([p.e[i] - this.origin.e0, (top - this.origin.z0) * this.opts.ex, -(p.n[i] - this.origin.n0)], 3));
    this.marker = new THREE.Points(g, new THREE.PointsMaterial({ size: 20, sizeAttenuation: false, color: css('--ink') || '#000', map: this.ring, alphaTest: 0.5, depthTest: false }));
    this.marker.renderOrder = 10;
    this.root.add(this.marker);
  }
}
