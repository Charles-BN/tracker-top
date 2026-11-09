// Tracker profile: elevation along the tracker, one axis (metres).
// Ground (survey or LIDAR), ground after cut/fill, the allowed reveal window
// and the optimised top of pile, with a per-pile hover tooltip.

const NS = 'http://www.w3.org/2000/svg';

function el(name, attrs = {}, parent) {
  const e = document.createElementNS(NS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (parent) parent.appendChild(e);
  return e;
}

function niceTicks(min, max, count) {
  const span = max - min || 1;
  const raw = span / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => span / s <= count) ?? raw;
  const out = [];
  for (let v = Math.ceil(min / step) * step; v <= max + 1e-9; v += step) out.push(+v.toFixed(6));
  return { ticks: out, step };
}

const fmt = (v, d = 3) => {
  if (v == null || Number.isNaN(v)) return '–';
  const s = v.toFixed(d);
  return /^-0\.?0*$/.test(s) ? s.slice(1) : s;
};

/**
 * rows: [{ id, s, z, ng, top, rev, ang, cf, zsrc }] ordered along the tracker.
 * settings: { reveal_nominal, delta_min, delta_max, max_angle_deg }
 */
export function renderProfile(container, rows, settings) {
  // Calculated values: 2 decimals when rounded to the centimetre.
  const dc = (settings.round_step ?? 0) >= 0.0095 ? 2 : 3;
  container.innerHTML = '';
  container.classList.add('profile');
  const hasResult = rows.some((r) => r.top != null);
  const hasCut = rows.some((r) => r.cf != null && Math.abs(r.cf) > 0.0005);
  const revMin = settings.reveal_nominal + settings.delta_min;
  const revMax = settings.reveal_nominal + settings.delta_max;
  const hasFlood = rows.some((r) => r.flvl != null);
  const raised = rows.some((r) => r.mta);

  const legend = document.createElement('div');
  legend.className = 'legend';
  legend.style.margin = '0 0 6px';
  const items = [];
  if (hasResult) items.push('<span class="item"><span class="ln" style="border-color:var(--s1)"></span>Top of pile (optimised)</span>');
  items.push('<span class="item"><span class="ln" style="border-color:var(--ink-2)"></span>Ground</span>');
  if (hasCut) items.push('<span class="item"><span class="ln dash" style="border-color:var(--s2)"></span>Ground after cut/fill</span>');
  if (hasFlood) items.push('<span class="item"><span class="ln dash" style="border-color:var(--s3)"></span>Flood water level</span>');
  items.push(`<span class="item"><span class="sw" style="background:var(--band)"></span>Allowed reveal ${revMin.toFixed(2)}–${revMax.toFixed(2)} m${raised ? ', raised for flood' : ''}</span>`);
  legend.innerHTML = items.join('');
  container.appendChild(legend);

  const W = Math.max(320, container.clientWidth || 600);
  const H = 260;
  const m = { l: 58, r: 14, t: 10, b: 34 };
  const svg = el('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img', 'aria-label': 'Tracker elevation profile' }, container);

  const base = (r) => (r.ng ?? r.z);
  const ys = [];
  for (const r of rows) {
    if (r.z != null) ys.push(r.z);
    if (r.ng != null) ys.push(r.ng);
    if (r.top != null) ys.push(r.top);
    if (base(r) != null) ys.push(base(r) + revMax);
    if (r.flvl != null) ys.push(r.flvl);
    if (r.mintop != null) ys.push(r.mintop);
  }
  const sMax = Math.max(...rows.map((r) => r.s), 1);
  let yMin = Math.min(...ys), yMax = Math.max(...ys);
  const pad = Math.max(0.1, (yMax - yMin) * 0.08);
  yMin -= pad; yMax += pad;
  const x = (s) => m.l + (s / sMax) * (W - m.l - m.r);
  const y = (v) => m.t + (1 - (v - yMin) / (yMax - yMin)) * (H - m.t - m.b);

  // Grid & axes (recessive).
  const { ticks: yt } = niceTicks(yMin, yMax, 5);
  for (const v of yt) {
    el('line', { x1: m.l, x2: W - m.r, y1: y(v), y2: y(v), class: 'gridline' }, svg);
    el('text', { x: m.l - 8, y: y(v) + 4, 'text-anchor': 'end', class: 'tick' }, svg).textContent = v.toFixed(yt.length > 1 && yt[1] - yt[0] < 0.1 ? 2 : 1);
  }
  const { ticks: xt } = niceTicks(0, sMax, 8);
  for (const v of xt) {
    el('text', { x: x(v), y: H - m.b + 18, 'text-anchor': 'middle', class: 'tick' }, svg).textContent = `${v}`;
  }
  el('line', { x1: m.l, x2: W - m.r, y1: H - m.b, y2: H - m.b, class: 'axisline' }, svg);
  el('text', { x: W - m.r, y: H - 4, 'text-anchor': 'end', class: 'tick' }, svg).textContent = 'distance along tracker (m)';
  const yl = el('text', { transform: `translate(12 ${(H - m.b + m.t) / 2}) rotate(-90)`, 'text-anchor': 'middle', class: 'tick' }, svg);
  yl.textContent = 'elevation (m)';

  const withBase = rows.filter((r) => base(r) != null);
  // Allowed band for the top: the reveal window, its bottom raised to the
  // flood minimum where there is one.
  const low = (r) => Math.max(base(r) + revMin, r.mintop ?? -Infinity);
  if (withBase.length > 1) {
    const up = withBase.map((r) => `${x(r.s)},${y(Math.max(base(r) + revMax, low(r)))}`);
    const lo = withBase.slice().reverse().map((r) => `${x(r.s)},${y(low(r))}`);
    el('polygon', { points: [...up, ...lo].join(' '), fill: 'var(--band)', stroke: 'none' }, svg);
  }
  const line = (pts, attrs) => {
    if (pts.length > 1) el('polyline', { points: pts.join(' '), fill: 'none', 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round', ...attrs }, svg);
  };
  // Pile shafts.
  for (const r of rows) {
    if (r.top != null && base(r) != null) {
      el('line', { x1: x(r.s), x2: x(r.s), y1: y(base(r)), y2: y(r.top), stroke: 'var(--muted)', 'stroke-width': 1 }, svg);
    }
  }
  line(rows.filter((r) => r.z != null).map((r) => `${x(r.s)},${y(r.z)}`), { stroke: 'var(--ink-2)' });
  if (hasFlood) line(rows.filter((r) => r.flvl != null).map((r) => `${x(r.s)},${y(r.flvl)}`), { stroke: 'var(--s3)', 'stroke-dasharray': '2 4' });
  if (hasCut) line(rows.filter((r) => r.ng != null).map((r) => `${x(r.s)},${y(r.ng)}`), { stroke: 'var(--s2)', 'stroke-dasharray': '5 4' });
  if (hasResult) {
    line(rows.filter((r) => r.top != null).map((r) => `${x(r.s)},${y(r.top)}`), { stroke: 'var(--s1)' });
    for (const r of rows) {
      if (r.top == null) continue;
      const over = r.ang != null && Math.abs(r.ang) > settings.max_angle_deg + 0.001;
      el('circle', { cx: x(r.s), cy: y(r.top), r: 4, fill: over ? 'var(--critical)' : 'var(--s1)', stroke: 'var(--surface)', 'stroke-width': 2 }, svg);
    }
  }

  // Hover: crosshair + tooltip on the nearest pile.
  const cross = el('line', { y1: m.t, y2: H - m.b, stroke: 'var(--axis)', 'stroke-width': 1, visibility: 'hidden' }, svg);
  const tip = document.createElement('div');
  tip.className = 'tooltip';
  tip.hidden = true;
  container.appendChild(tip);
  const hit = el('rect', { x: m.l, y: m.t, width: W - m.l - m.r, height: H - m.t - m.b, fill: 'transparent' }, svg);
  const pick = (evt) => {
    const box = svg.getBoundingClientRect();
    const px = ((evt.clientX - box.left) / box.width) * W;
    let best = rows[0], bd = Infinity;
    for (const r of rows) { const d = Math.abs(x(r.s) - px); if (d < bd) { bd = d; best = r; } }
    cross.setAttribute('x1', x(best.s)); cross.setAttribute('x2', x(best.s));
    cross.setAttribute('visibility', 'visible');
    const src = best.zsrc === 'survey' ? 'survey' : best.zsrc === 'lidar' ? 'LIDAR' : '';
    tip.innerHTML = `<b>${best.id}</b>
      <div class="row"><span>Ground${src ? ` (${src})` : ''}</span><span class="num">${fmt(best.z)}</span></div>
      ${best.zsrc === 'survey' && best.zl != null ? `<div class="row"><span>LIDAR ground</span><span class="num">${fmt(best.zl)}</span></div>` : ''}
      ${hasCut ? `<div class="row"><span>Cut / fill</span><span class="num">${fmt(best.cf, dc)}</span></div>` : ''}
      ${best.flvl != null ? `<div class="row"><span>Flood water level</span><span class="num">${fmt(best.flvl, dc)}</span></div><div class="row"><span>Lowest allowed top</span><span class="num">${fmt(best.mintop, dc)}</span></div>` : ''}
      <div class="row"><span>Top of pile${best.mta ? ' (raised for flood)' : ''}</span><span class="num">${fmt(best.top, dc)}</span></div>
      <div class="row"><span>Reveal</span><span class="num">${fmt(best.rev, dc)}</span></div>
      <div class="row"><span>Angle change</span><span class="num">${fmt(best.ang, 3)}°</span></div>
      <div class="row"><span>Distance</span><span class="num">${best.s.toFixed(2)} m</span></div>`;
    tip.hidden = false;
    const cx = (x(best.s) / W) * box.width;
    const left = cx + 14 + tip.offsetWidth > box.width ? cx - 14 - tip.offsetWidth : cx + 14;
    tip.style.left = `${left}px`;
    tip.style.top = `${legend.offsetHeight + 8}px`;
  };
  hit.addEventListener('pointermove', pick);
  hit.addEventListener('pointerleave', () => { tip.hidden = true; cross.setAttribute('visibility', 'hidden'); });
}
