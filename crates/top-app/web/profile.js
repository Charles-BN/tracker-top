// Tracker profile: elevation along the tracker, one axis (metres).
// Survey ground and LIDAR ground, ground after cut/fill, the allowed reveal
// window and the optimised top of pile (motor piles marked), with a per-pile
// hover tooltip. Can be shown full screen, with pile numbers.

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

/** A motor pile: type letters ending in MP (HMP-20, SMP-31) or "motor" (as setout.rs). */
export function isMotorPile(type) {
  const up = String(type || '').trim().toUpperCase();
  return /^[A-Z]*MP(?![A-Z])/.test(up) || up.includes('MOTOR');
}

/**
 * rows: [{ id, s, z, zs, zl, ng, top, rev, ang, cf, zsrc, type }] ordered along the tracker.
 * settings: { reveal_nominal, delta_min, delta_max, max_angle_deg }
 * opts: { height, labels } (full-screen view: taller, pile numbers shown)
 */
export function renderProfile(container, rows, settings, opts = {}) {
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
  const hasSurvey = rows.some((r) => r.zs != null);
  const hasLidar = rows.some((r) => r.zl != null);
  const hasMotor = rows.some((r) => isMotorPile(r.type));
  if (hasResult) items.push('<span class="item"><span class="ln" style="border-color:var(--s1)"></span>Top of pile (optimised)</span>');
  if (hasMotor) items.push('<span class="item"><span class="motor-key"></span>Motor pile (M)</span>');
  if (hasSurvey) items.push('<span class="item"><span class="ln" style="border-color:var(--ink)"></span>Survey ground</span>');
  if (hasLidar) items.push('<span class="item"><span class="ln dash" style="border-color:var(--muted)"></span>LIDAR ground</span>');
  if (hasCut) items.push('<span class="item"><span class="ln dash" style="border-color:var(--s2)"></span>Ground after cut/fill</span>');
  if (hasFlood) items.push('<span class="item"><span class="ln dash" style="border-color:var(--s3)"></span>Flood water level</span>');
  items.push(`<span class="item"><span class="sw" style="background:var(--band)"></span>Allowed reveal ${revMin.toFixed(2)}–${revMax.toFixed(2)} m${raised ? ', raised for flood' : ''}</span>`);
  legend.innerHTML = items.join('');
  container.appendChild(legend);

  const W = Math.max(320, container.clientWidth || 600);
  const H = opts.height || 260;
  const m = { l: 58, r: 14, t: opts.labels ? 26 : 10, b: 34 };
  const svg = el('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img', 'aria-label': 'Tracker elevation profile' }, container);

  const base = (r) => (r.ng ?? r.z);
  const ys = [];
  for (const r of rows) {
    if (r.z != null) ys.push(r.z);
    if (r.zs != null) ys.push(r.zs);
    if (r.zl != null) ys.push(r.zl);
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
  // Pile shafts (motor piles thicker).
  for (const r of rows) {
    if (r.top != null && base(r) != null) {
      const motor = isMotorPile(r.type);
      el('line', { x1: x(r.s), x2: x(r.s), y1: y(base(r)), y2: y(r.top), stroke: motor ? 'var(--ink-2)' : 'var(--muted)', 'stroke-width': motor ? 2.5 : 1 }, svg);
    }
  }
  // LIDAR ground (dashed), then survey ground (solid, a dot at each surveyed
  // pile, broken where piles are not surveyed).
  if (hasLidar) line(rows.filter((r) => r.zl != null).map((r) => `${x(r.s)},${y(r.zl)}`), { stroke: 'var(--muted)', 'stroke-width': 1.5, 'stroke-dasharray': '6 4' });
  if (hasSurvey) {
    let run = [];
    const flush = () => { line(run, { stroke: 'var(--ink)' }); run = []; };
    for (const r of rows) {
      if (r.zs == null) { flush(); continue; }
      run.push(`${x(r.s)},${y(r.zs)}`);
    }
    flush();
    for (const r of rows) if (r.zs != null) el('circle', { cx: x(r.s), cy: y(r.zs), r: 2.5, fill: 'var(--ink)' }, svg);
  }
  if (!hasSurvey && !hasLidar) line(rows.filter((r) => r.z != null).map((r) => `${x(r.s)},${y(r.z)}`), { stroke: 'var(--ink-2)' });
  if (hasFlood) line(rows.filter((r) => r.flvl != null).map((r) => `${x(r.s)},${y(r.flvl)}`), { stroke: 'var(--s3)', 'stroke-dasharray': '2 4' });
  if (hasCut) line(rows.filter((r) => r.ng != null).map((r) => `${x(r.s)},${y(r.ng)}`), { stroke: 'var(--s2)', 'stroke-dasharray': '5 4' });
  if (hasResult) {
    line(rows.filter((r) => r.top != null).map((r) => `${x(r.s)},${y(r.top)}`), { stroke: 'var(--s1)' });
    for (const r of rows) {
      if (r.top == null) continue;
      const over = r.ang != null && Math.abs(r.ang) > settings.max_angle_deg + 0.001;
      const fill = over ? 'var(--critical)' : 'var(--s1)';
      if (isMotorPile(r.type)) {
        // Motor pile: a larger outlined square and an "M".
        el('rect', { x: x(r.s) - 6, y: y(r.top) - 6, width: 12, height: 12, rx: 2, fill: 'var(--surface)', stroke: fill, 'stroke-width': 3 }, svg);
        el('text', { x: x(r.s), y: y(r.top) - 11, 'text-anchor': 'middle', class: 'tick motor-label' }, svg).textContent = 'M';
      } else {
        el('circle', { cx: x(r.s), cy: y(r.top), r: 4, fill, stroke: 'var(--surface)', 'stroke-width': 2 }, svg);
      }
    }
  }
  // Full screen: pile numbers (last part of the ID) along the top.
  if (opts.labels) {
    for (const r of rows) {
      el('text', { x: x(r.s), y: 14, 'text-anchor': 'middle', class: 'tick' }, svg).textContent = String(r.id).split('.').pop();
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
    const used = (k) => (best.zsrc === k ? ' (in use)' : '');
    tip.innerHTML = `<b>${best.id}</b>${isMotorPile(best.type) ? ' <span class="muted">motor pile</span>' : ''}
      ${best.zs != null ? `<div class="row"><span>Survey ground${used('survey')}</span><span class="num">${fmt(best.zs)}</span></div>` : '<div class="row"><span>Survey ground</span><span class="num muted">not surveyed</span></div>'}
      ${best.zl != null ? `<div class="row"><span>LIDAR ground${used('lidar')}</span><span class="num">${fmt(best.zl)}</span></div>` : ''}
      ${best.zs != null && best.zl != null ? `<div class="row"><span>Survey − LIDAR</span><span class="num">${fmt(best.zs - best.zl, dc)}</span></div>` : ''}
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
