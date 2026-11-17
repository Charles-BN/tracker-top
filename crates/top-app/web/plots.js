// Plots tab. First plot: survey progress over time (piles with a field-survey
// elevation, by survey date) and a provisional curve to the last pile, at the
// pace of the recent weeks, with the range given by the slowest and fastest
// week of that period.

const DAY = 86_400_000;
const NS = 'http://www.w3.org/2000/svg';

const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const int = (n) => Math.round(n).toLocaleString();
const pct = (a, b) => (b ? `${((100 * a) / b).toFixed(1)} %` : '–');
const dayOf = (iso) => Math.round(Date.parse(`${iso}T00:00:00Z`) / DAY);
const dateText = (d, opts = { day: 'numeric', month: 'short', year: 'numeric' }) =>
  new Date(d * DAY).toLocaleDateString(undefined, { ...opts, timeZone: 'UTC' });
const todayDay = () => Math.floor(Date.now() / DAY);

// ------------------------------------------------------------------ model

/**
 * Survey progress of the chosen blocks (`block` '' = all) and its projection.
 * `windowDays`: how far back the recent pace is measured (0 = whole survey).
 * `target`: optional finish date (day number) to compare with.
 */
export function progressModel(data, block, windowDays, target) {
  const pick = (m) => (block ? m[block] || 0 : Object.values(m).reduce((a, b) => a + b, 0));
  const total = pick(data.block_piles);
  const points = [];
  let cum = 0;
  for (const d of data.days) {
    const n = pick(d.by_block);
    if (!n) continue;
    cum += n;
    points.push({ day: dayOf(d.date), n, cum });
  }
  const m = { total, points, surveyed: cum, remaining: Math.max(0, total - cum), weeks: [] };
  if (!points.length) return m;

  const first = points[0].day;
  const last = points[points.length - 1].day;
  m.first = first;
  m.last = last;
  m.span = last - first + 1; // days of survey, first and last included
  const cumAt = (day) => { let c = 0; for (const p of points) { if (p.day > day) break; c = p.cum; } return c; };
  m.cumAt = cumAt;

  // Weeks ending on the last survey day, going back; the oldest one can be partial.
  for (let end = last; end >= first; end -= 7) {
    const start = end - 6;
    m.weeks.unshift({ start, end, n: cumAt(end) - cumAt(start - 1), partial: start < first });
  }

  // Recent pace: piles per calendar day over the window (weekends and stops included).
  const w = windowDays > 0 ? Math.min(windowDays, m.span) : m.span;
  m.windowUsed = w;
  m.pace = (cum - cumAt(last - w)) / w;
  const inWindow = m.weeks.filter((k) => !k.partial && k.end - k.start + 1 === 7 && k.start > last - w);
  if (inWindow.length >= 2) {
    m.slow = Math.min(...inWindow.map((k) => k.n)) / 7;
    m.fast = Math.max(...inWindow.map((k) => k.n)) / 7;
  }
  m.weeksInWindow = inWindow.length;
  const finishAt = (pace) => (m.remaining === 0 ? last : pace > 0 ? last + Math.ceil(m.remaining / pace) : null);
  m.finish = finishAt(m.pace);
  if (m.slow !== undefined) { m.finishSlow = finishAt(m.slow); m.finishFast = finishAt(m.fast); }
  if (target && m.remaining > 0) {
    m.target = target;
    m.needed = target > last ? m.remaining / (target - last) : null;
  }
  return m;
}

// ------------------------------------------------------------------ drawing

function el(tag, attrs = {}, parent) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v !== undefined && v !== null) e.setAttribute(k, v);
  parent?.appendChild(e);
  return e;
}

/** Calendar ticks for a range of days: weekly, fortnightly or monthly. */
function timeTicks(d0, d1, width) {
  const span = d1 - d0;
  const maxTicks = Math.max(2, Math.floor(width / 80));
  const ticks = [];
  if (span / 7 <= maxTicks) {
    // Mondays.
    let d = d0 + ((8 - new Date(d0 * DAY).getUTCDay()) % 7);
    for (; d <= d1; d += 7) ticks.push(d);
    return { ticks, fmt: (d) => dateText(d, { day: 'numeric', month: 'short' }) };
  }
  const step = [1, 2, 3, 6, 12].find((s) => span / (30.4 * s) <= maxTicks) || 12;
  const t = new Date(d0 * DAY);
  let y = t.getUTCFullYear();
  let mo = t.getUTCMonth() + 1;
  for (;;) {
    if (mo > 11) { y += Math.floor(mo / 12); mo %= 12; }
    const d = Date.UTC(y, mo, 1) / DAY;
    if (d > d1) break;
    if (mo % step === 0 || step === 1) ticks.push(d);
    mo += 1;
  }
  return { ticks, fmt: (d) => dateText(d, { month: 'short', year: 'numeric' }) };
}

function niceStep(max, n) {
  const raw = max / n;
  const p = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 2.5, 5, 10].map((k) => k * p).find((s) => max / s <= n) || 10 * p;
}

function tooltip(host) {
  let tip = host.querySelector('.plot-tip');
  if (!tip) { tip = document.createElement('div'); tip.className = 'plot-tip'; tip.hidden = true; host.appendChild(tip); }
  return {
    show(html, x, y) {
      tip.innerHTML = html;
      tip.hidden = false;
      const w = tip.offsetWidth;
      const hw = host.clientWidth;
      tip.style.left = `${Math.max(0, Math.min(hw - w, x + 14 > hw - w ? x - w - 14 : x + 14))}px`;
      tip.style.top = `${Math.max(0, y - tip.offsetHeight - 10)}px`;
    },
    hide() { tip.hidden = true; },
  };
}

/** Cumulative piles surveyed, with the provisional curve and its range. */
export function drawProgress(host, m) {
  host.querySelector('svg')?.remove();
  const W = Math.max(300, host.clientWidth);
  const H = Math.round(Math.min(420, Math.max(260, W * 0.42)));
  const pad = { l: 58, r: 16, t: 14, b: 30 };
  const svg = el('svg', { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'img', 'aria-label': 'Piles surveyed over time, with the provisional curve' });
  host.prepend(svg);
  if (!m.points.length) return;

  const today = todayDay();
  let end = Math.max(m.last + 7, today);
  const ends = [m.finish, m.finishFast, m.finishSlow, m.target].filter((d) => d);
  if (ends.length) end = Math.max(end, ...ends);
  // Keep the survey readable: the projection may run off the right edge.
  const cap = m.last + Math.max(120, 3 * m.span);
  const clipped = end > cap;
  end = Math.min(end, cap) + 3;
  const start = m.first - 3;
  const yMax = Math.max(m.total, m.surveyed) || 1;
  const x = (d) => pad.l + ((d - start) / (end - start)) * (W - pad.l - pad.r);
  const y = (v) => pad.t + (1 - v / yMax) * (H - pad.t - pad.b);

  // Grid and axes.
  const g = el('g', { class: 'plot-grid' }, svg);
  const step = niceStep(yMax, 5);
  for (let v = 0; v <= yMax + 1e-9; v += step) {
    el('line', { x1: pad.l, x2: W - pad.r, y1: y(v), y2: y(v) }, g);
    el('text', { x: pad.l - 8, y: y(v) + 4, 'text-anchor': 'end', class: 'plot-tick' }, svg).textContent = int(v);
  }
  const { ticks, fmt } = timeTicks(start, end, W - pad.l - pad.r);
  for (const d of ticks) {
    el('line', { x1: x(d), x2: x(d), y1: H - pad.b, y2: H - pad.b + 4, class: 'plot-axis' }, svg);
    el('text', { x: x(d), y: H - pad.b + 18, 'text-anchor': 'middle', class: 'plot-tick' }, svg).textContent = fmt(d);
  }
  el('line', { x1: pad.l, x2: W - pad.r, y1: y(0), y2: y(0), class: 'plot-axis' }, svg);

  // All piles.
  el('line', { x1: pad.l, x2: W - pad.r, y1: y(m.total), y2: y(m.total), class: 'plot-total' }, svg);
  el('text', { x: pad.l + 6, y: y(m.total) - 6, class: 'plot-note' }, svg).textContent = `All piles: ${int(m.total)}`;

  // Today.
  if (today >= start && today <= end) {
    el('line', { x1: x(today), x2: x(today), y1: pad.t, y2: H - pad.b, class: 'plot-today' }, svg);
    el('text', { x: x(today) + 4, y: H - pad.b - 6, class: 'plot-note' }, svg).textContent = 'Today';
  }

  const proj = (pace, d) => Math.min(m.total, m.surveyed + pace * (d - m.last));
  const line = (pace) => {
    const fin = pace > 0 ? m.last + m.remaining / pace : Infinity;
    const pts = [[m.last, m.surveyed]];
    if (fin < end) pts.push([fin, m.total], [end, m.total]);
    else pts.push([end, proj(pace, end)]);
    return pts;
  };
  const path = (pts) => pts.map(([d, v], i) => `${i ? 'L' : 'M'}${x(d).toFixed(1)},${y(v).toFixed(1)}`).join('');

  if (m.remaining > 0) {
    // Range of the recent weeks.
    if (m.slow !== undefined) {
      const hi = line(m.fast);
      const lo = line(m.slow);
      el('path', { d: `${path(hi)}${path(lo.slice().reverse()).replace(/^M/, 'L')}Z`, class: 'plot-band' }, svg);
    }
    // Target pace.
    if (m.target && m.target > m.last) {
      el('path', { d: path([[m.last, m.surveyed], [Math.min(m.target, end), proj(m.needed, Math.min(m.target, end))]]), class: 'plot-target' }, svg);
      if (m.target <= end) el('circle', { cx: x(m.target), cy: y(m.total), r: 4, class: 'plot-target-dot' }, svg);
    }
    if (m.pace > 0) el('path', { d: path(line(m.pace)), class: 'plot-proj' }, svg);
  }

  // Survey so far.
  const pts = [[m.first - 1, 0]];
  for (const p of m.points) pts.push([p.day, p.cum]);
  el('path', { d: path(pts), class: 'plot-actual' }, svg);
  const lastPt = m.points[m.points.length - 1];
  el('circle', { cx: x(lastPt.day), cy: y(lastPt.cum), r: 4, class: 'plot-actual-dot' }, svg);

  // Provisional finish marker.
  if (m.remaining > 0 && m.finish && m.finish <= end) {
    el('circle', { cx: x(m.finish), cy: y(m.total), r: 4, class: 'plot-proj-dot' }, svg);
    const tx = x(m.finish);
    el('text', { x: tx, y: y(m.total) + 18, 'text-anchor': tx > W - 90 ? 'end' : 'middle', class: 'plot-label' }, svg).textContent = dateText(m.finish);
  }
  if (clipped) el('text', { x: W - pad.r, y: pad.t + 12, 'text-anchor': 'end', class: 'plot-note' }, svg).textContent = 'continues →';

  // Hover: a line and the values of that day.
  const cross = el('line', { y1: pad.t, y2: H - pad.b, class: 'plot-cross', visibility: 'hidden' }, svg);
  const dot = el('circle', { r: 5, class: 'plot-hover-dot', visibility: 'hidden' }, svg);
  const tip = tooltip(host);
  const hit = el('rect', { x: pad.l, y: pad.t, width: W - pad.l - pad.r, height: H - pad.t - pad.b, fill: 'transparent' }, svg);
  const move = (ev) => {
    const r = svg.getBoundingClientRect();
    const px = ((ev.clientX - r.left) / r.width) * W;
    const d = Math.round(start + ((px - pad.l) / (W - pad.l - pad.r)) * (end - start));
    if (d < m.first - 1) { leave(); return; }
    let html;
    let v;
    if (d <= m.last) {
      v = m.cumAt(d);
      const day = m.points.find((p) => p.day === d);
      html = `<b>${dateText(d)}</b><div>Surveyed <b>${int(v)}</b> · ${pct(v, m.total)}</div>${day ? `<div class="muted">+${int(day.n)} that day</div>` : ''}`;
    } else if (m.remaining > 0 && m.pace > 0) {
      v = proj(m.pace, d);
      const range = m.slow !== undefined ? `<div class="muted">Range ${int(proj(m.slow, d))} – ${int(proj(m.fast, d))}</div>` : '';
      html = `<b>${dateText(d)}</b><div>Provisional <b>${int(v)}</b> · ${pct(v, m.total)}</div>${range}`;
    } else { leave(); return; }
    cross.setAttribute('x1', x(d)); cross.setAttribute('x2', x(d)); cross.setAttribute('visibility', 'visible');
    dot.setAttribute('cx', x(d)); dot.setAttribute('cy', y(v)); dot.setAttribute('visibility', 'visible');
    tip.show(html, (x(d) / W) * r.width, (y(v) / H) * r.height);
  };
  const leave = () => { cross.setAttribute('visibility', 'hidden'); dot.setAttribute('visibility', 'hidden'); tip.hide(); };
  hit.addEventListener('pointermove', move);
  hit.addEventListener('pointerdown', move);
  hit.addEventListener('pointerleave', leave);
}

/** Piles surveyed per week, with the recent weekly pace. */
export function drawWeekly(host, m) {
  host.querySelector('svg')?.remove();
  const W = Math.max(300, host.clientWidth);
  const H = 200;
  const pad = { l: 58, r: 16, t: 14, b: 30 };
  const svg = el('svg', { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'img', 'aria-label': 'Piles surveyed per week' });
  host.prepend(svg);
  const weeks = m.weeks;
  if (!weeks.length) return;
  const yMax = Math.max(1, ...weeks.map((k) => k.n), m.pace * 7, m.needed ? m.needed * 7 : 0);
  const step = niceStep(yMax, 4);
  const top = Math.ceil(yMax / step) * step;
  const y = (v) => pad.t + (1 - v / top) * (H - pad.t - pad.b);
  const g = el('g', { class: 'plot-grid' }, svg);
  for (let v = 0; v <= top + 1e-9; v += step) {
    el('line', { x1: pad.l, x2: W - pad.r, y1: y(v), y2: y(v) }, g);
    el('text', { x: pad.l - 8, y: y(v) + 4, 'text-anchor': 'end', class: 'plot-tick' }, svg).textContent = int(v);
  }
  const slot = (W - pad.l - pad.r) / weeks.length;
  const bw = Math.max(3, Math.min(36, slot - 2));
  const tip = tooltip(host);
  const every = Math.ceil(70 / slot);
  weeks.forEach((k, i) => {
    const cx = pad.l + slot * (i + 0.5);
    const h = y(0) - y(k.n);
    if (k.n > 0) {
      const r = Math.min(4, bw / 2, h);
      const x0 = cx - bw / 2;
      el('path', {
        d: `M${x0},${y(0)}V${y(k.n) + r}Q${x0},${y(k.n)} ${x0 + r},${y(k.n)}H${x0 + bw - r}Q${x0 + bw},${y(k.n)} ${x0 + bw},${y(k.n) + r}V${y(0)}Z`,
        class: k.partial ? 'plot-bar partial' : 'plot-bar',
      }, svg);
    }
    if ((weeks.length - 1 - i) % every === 0) {
      el('text', { x: cx, y: H - pad.b + 18, 'text-anchor': 'middle', class: 'plot-tick' }, svg).textContent = dateText(k.end, { day: 'numeric', month: 'short' });
    }
    const hit = el('rect', { x: cx - slot / 2, y: pad.t, width: slot, height: H - pad.t - pad.b, fill: 'transparent' }, svg);
    const showTip = () => {
      const r = svg.getBoundingClientRect();
      tip.show(`<b>${dateText(Math.max(k.start, m.first), { day: 'numeric', month: 'short' })} – ${dateText(k.end)}</b><div><b>${int(k.n)}</b> piles surveyed${k.partial ? ' (part week)' : ''}</div>`, (cx / W) * r.width, (y(k.n) / H) * r.height);
    };
    hit.addEventListener('pointerenter', showTip);
    hit.addEventListener('pointerdown', showTip);
    hit.addEventListener('pointerleave', () => tip.hide());
  });
  el('line', { x1: pad.l, x2: W - pad.r, y1: y(0), y2: y(0), class: 'plot-axis' }, svg);
  const ref = (v, cls, text) => {
    el('line', { x1: pad.l, x2: W - pad.r, y1: y(v), y2: y(v), class: cls }, svg);
    el('text', { x: W - pad.r, y: y(v) - 5, 'text-anchor': 'end', class: 'plot-label' }, svg).textContent = text;
  };
  if (m.pace > 0) ref(m.pace * 7, 'plot-proj thin', `Recent pace ${int(m.pace * 7)} / week`);
  if (m.needed) ref(m.needed * 7, 'plot-target thin', `Needed ${int(m.needed * 7)} / week`);
}

// ------------------------------------------------------------------ the tab

const PREF = 'tracker-top.plots';
const loadPrefs = () => { try { return JSON.parse(localStorage.getItem(PREF)) || {}; } catch { return {}; } };
const savePrefs = (p) => { try { localStorage.setItem(PREF, JSON.stringify(p)); } catch { /* not kept */ } };

/** Fills the Plots tab (`root`) from GET api/progress. */
export function renderPlots(root, data) {
  const prefs = { block: '', window: 28, target: '', ...loadPrefs() };
  const blocks = Object.keys(data.block_piles).filter((b) => b).sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  if (prefs.block && !blocks.includes(prefs.block)) prefs.block = '';

  root.innerHTML = `
    <div class="card plot-card">
      <div class="plot-head">
        <div>
          <h2>Survey progress</h2>
          <p class="ink2">Piles with a field-survey elevation, by survey date, and a provisional curve to the last pile.</p>
        </div>
        <div class="plot-filters">
          <label>Blocks <select id="pl-block"><option value="">All blocks</option>${blocks.map((b) => `<option value="${esc(b)}">Block ${esc(b)}</option>`).join('')}</select></label>
          <label>Pace from <select id="pl-window">
            <option value="14">the last 2 weeks</option><option value="28">the last 4 weeks</option>
            <option value="56">the last 8 weeks</option><option value="0">the whole survey</option></select></label>
          <label>Target finish <input type="date" id="pl-target"></label>
        </div>
      </div>
      <div class="metrics plot-stats" id="pl-stats"></div>
      <div class="plot-legend" id="pl-legend"></div>
      <div class="plot" id="pl-progress"></div>
      <p class="muted plot-notes" id="pl-notes"></p>
      <h3 class="plot-sub">Piles surveyed per week</h3>
      <div class="plot" id="pl-weekly"></div>
      <details class="more"><summary>Table</summary><div class="table-wrap"><table id="pl-table"></table></div></details>
      <details class="more"><summary>How the provisional curve is made</summary>
        <p class="ink2">The curve continues from the last survey day at the <b>recent pace</b>: the piles surveyed over the chosen
        period (2, 4 or 8 weeks, or the whole survey) divided by its number of calendar days, so weekends, rain and stops count as
        they did. The shaded <b>range</b> uses the slowest and the fastest full week of that period (needs at least two full weeks).
        Each pile counts once, on its first survey date (the date found in the file name or its date column, else the import day);
        voided imports are left out. With a target date, the dashed line shows the pace needed to finish on that day.</p>
      </details>
    </div>`;

  const $ = (s) => root.querySelector(s);
  $('#pl-block').value = prefs.block;
  $('#pl-window').value = String(prefs.window);
  $('#pl-target').value = prefs.target || '';

  let m;
  const draw = () => {
    if (!root.isConnected || !root.offsetParent) return;
    drawProgress($('#pl-progress'), m);
    drawWeekly($('#pl-weekly'), m);
  };
  const update = () => {
    m = progressModel(data, prefs.block, +prefs.window, prefs.target ? dayOf(prefs.target) : null);
    const stat = (label, value, sub = '') => `<div class="metric"><span class="m-label">${label}</span><span class="m-value">${value}</span>${sub ? `<span class="m-sub">${sub}</span>` : ''}</div>`;
    if (!m.points.length) {
      $('#pl-stats').innerHTML = '';
      $('#pl-legend').innerHTML = '';
      $('#pl-progress').innerHTML = `<p class="muted empty">No field survey yet${prefs.block ? ` in block ${esc(prefs.block)}` : ''}: import a survey (Import tab) to see the progress.</p>`;
      $('#pl-weekly').innerHTML = '';
      $('#pl-notes').textContent = '';
      $('#pl-table').innerHTML = '';
      return;
    }
    $('#pl-progress').innerHTML = '';
    let finish;
    if (m.remaining === 0) finish = stat('Survey finished', dateText(m.last));
    else if (!m.finish) finish = stat('Provisional finish', '–', 'no piles surveyed in the period');
    else {
      const range = m.finishSlow !== undefined
        ? `range ${dateText(m.finishFast, { day: 'numeric', month: 'short' })} – ${m.finishSlow ? dateText(m.finishSlow) : 'no finish at the slowest week (none surveyed)'}`
        : 'range: needs two full weeks in the period';
      finish = stat('Provisional finish', dateText(m.finish), range);
    }
    $('#pl-stats').innerHTML = [
      stat('Surveyed', `${int(m.surveyed)} <small class="of">/ ${int(m.total)}</small>`, `${pct(m.surveyed, m.total)} · ${int(m.remaining)} to go`),
      stat('Recent pace', `${int(m.pace * 7)} <small class="of">piles / week</small>`, `over ${m.windowUsed} days to ${dateText(m.last, { day: 'numeric', month: 'short' })}`),
      finish,
      m.target ? stat('Needed for the target', m.needed ? `${int(m.needed * 7)} <small class="of">/ week</small>` : '–', m.needed ? `to finish on ${dateText(m.target)}` : 'the target is before the last survey') : '',
    ].join('');

    const sw = (cls, text) => `<span class="plot-key"><svg width="22" height="10" aria-hidden="true">${cls === 'plot-band' ? '<rect x="0" y="1" width="22" height="8" rx="2" class="plot-band"/>' : `<line x1="1" x2="21" y1="5" y2="5" class="${cls}"/>`}</svg>${text}</span>`;
    $('#pl-legend').innerHTML = [
      sw('plot-actual', 'Surveyed'),
      m.remaining > 0 && m.pace > 0 ? sw('plot-proj', 'Provisional (recent pace)') : '',
      m.remaining > 0 && m.slow !== undefined ? sw('plot-band', 'Range of the recent weeks') : '',
      m.target && m.remaining > 0 ? sw('plot-target', 'Pace for the target') : '',
    ].join('');

    const notes = [];
    if (m.span < 14) notes.push(`Only ${m.span} day${m.span === 1 ? '' : 's'} of survey so far: the provisional curve is a first guess.`);
    if (todayDay() - m.last > 7) notes.push(`The last survey is from ${dateText(m.last)}; the curve starts there. Import newer surveys to update it.`);
    $('#pl-notes').textContent = notes.join(' ');

    $('#pl-table').innerHTML = `<thead><tr><th>Week</th><th class="num">Piles surveyed</th><th class="num">Total surveyed</th><th class="num">Share</th></tr></thead><tbody>${
      m.weeks.slice().reverse().map((k) => `<tr><td>${dateText(Math.max(k.start, m.first), { day: 'numeric', month: 'short' })} – ${dateText(k.end)}</td><td class="num">${int(k.n)}</td><td class="num">${int(m.cumAt(k.end))}</td><td class="num">${pct(m.cumAt(k.end), m.total)}</td></tr>`).join('')}</tbody>`;
    draw();
  };

  const change = () => {
    prefs.block = $('#pl-block').value;
    prefs.window = +$('#pl-window').value;
    prefs.target = $('#pl-target').value;
    savePrefs(prefs);
    update();
  };
  ['#pl-block', '#pl-window', '#pl-target'].forEach((s) => $(s).addEventListener('change', change));
  update();

  // Redraw at the new width when the window or the panel changes size.
  let last = 0;
  const ro = new ResizeObserver(() => {
    const w = $('#pl-progress')?.clientWidth || 0;
    if (w && Math.abs(w - last) > 4) { last = w; if (m?.points.length) draw(); }
  });
  ro.observe($('#pl-progress'));
  return { redraw: () => m?.points.length && draw(), dispose: () => ro.disconnect() };
}
