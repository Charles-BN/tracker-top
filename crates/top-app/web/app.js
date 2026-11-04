import { renderProfile } from './profile.js';

const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const fmt = (v, d = 3) => {
  if (v == null || Number.isNaN(v)) return '–';
  const s = Number(v).toFixed(d);
  return /^-0\.?0*$/.test(s) ? s.slice(1) : s;
};
const int = (v) => (v ?? 0).toLocaleString();
const pct = (a, b) => (b ? Math.round((100 * a) / b) : 0);

const STATUS = {
  Compliant: ['good', 'Compliant'],
  Graded: ['warning', 'Graded (cut/fill)'],
  NonCompliant: ['critical', 'Non-compliant'],
  Skipped: ['serious', 'Skipped (< 3 piles)'],
  InvalidInput: ['serious', 'Invalid input'],
  AwaitingSurvey: ['neutral', 'Awaiting elevations'],
  NotRun: ['notrun', 'Not optimised'],
};
const STATUS_ORDER = Object.keys(STATUS);
const STATUS_FILL = { good: 'var(--good)', warning: 'var(--warning)', critical: 'var(--critical)', serious: 'var(--serious)', neutral: 'var(--neutral)', notrun: 'var(--notrun)' };
const statusBadge = (k) => {
  const [cls, label] = STATUS[k] ?? ['neutral', k];
  return `<span class="status st-${cls}"><span class="dot"></span>${esc(label)}</span>`;
};
const KIND = { master: 'Master pile list', survey: 'Field survey', lidar: 'LIDAR / design ground', flood: 'Flood values' };
// Site colour of a pile, as a CSS colour (the master list says e.g. "RED", "LIGHT BLUE").
const siteColor = (c) => String(c || '').trim().toLowerCase().replace(/\s+/g, '');
const swatch = (c) => (c ? `<span class="sw-dot" style="background:${esc(siteColor(c))}" title="${esc(c)}"></span>` : '');

// ---------------------------------------------------------------- API

// Browser version: the engine runs in the page (WebAssembly) instead of the
// desktop program; see browser.js. Set by the browser build's index.html.
const BROWSER = !!window.TRACKER_TOP_BROWSER;
let browser = null;

async function api(method, url, body, raw = false) {
  if (BROWSER) {
    const r = await browser.request(method, url, body, raw);
    const text = new TextDecoder().decode(r.body);
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { data = text; }
    if (r.status >= 400) throw new Error((data && data.error) || text || `error ${r.status}`);
    return data;
  }
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.body = raw ? body : JSON.stringify(body);
    opts.headers['Content-Type'] = raw ? 'application/octet-stream' : 'application/json';
  }
  const res = await fetch(url, opts);
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!res.ok) throw new Error((data && data.error) || text || res.statusText);
  return data;
}

function toast(msg, err = false) {
  const el = document.createElement('div');
  el.className = 'msg' + (err ? ' err' : '');
  el.textContent = msg;
  $('#toast').appendChild(el);
  setTimeout(() => el.remove(), err ? 9000 : 5000);
}

const cache = { summary: null, state: null };
async function loadSummary(force = false) {
  if (!cache.summary || force) cache.summary = await api('GET', 'api/summary');
  return cache.summary;
}
async function loadState(force = false) {
  if (!cache.state || force) cache.state = await api('GET', 'api/state');
  return cache.state;
}
async function refresh() {
  cache.state = null;
  await loadSummary(true);
  updateHeader();
  await show(current, true);
}

function runText(run) {
  if (!run) return '';
  if (!run.run_id) return run.message;
  const c = run.counts || {};
  const parts = STATUS_ORDER.filter((k) => c[k]).map((k) => `${c[k]} ${STATUS[k][1].toLowerCase()}`);
  return `${run.trackers_run} tracker${run.trackers_run === 1 ? '' : 's'} optimised${parts.length ? ': ' + parts.join(', ') : ''}.`;
}

function updateHeader() {
  const s = cache.summary;
  if (!s) return;
  $('#btn-gpkg').title = s.settings.epsg
    ? `GeoPackage for ArcGIS / QGIS (EPSG:${s.settings.epsg}): piles and trackers with all results`
    : 'GeoPackage for ArcGIS / QGIS. Set the coordinate system in Settings > ArcGIS export first.';
  $('#db-path').textContent = s.summary.db_path;
  $('#db-path').title = `Database: ${s.summary.db_path}`;
  const stale = s.summary.stale_trackers;
  const badge = $('#stale-count');
  badge.hidden = !stale;
  badge.textContent = stale;
  $('#btn-optimize').title = stale ? `${stale} trackers have newer elevations or settings than their last result. Shift+click re-optimises every tracker.` : 'Everything is up to date. Shift+click re-optimises every tracker.';
}

// ---------------------------------------------------------------- tabs

let current = 'overview';
const renderers = {};

async function show(tab, rerender = false) {
  if (!renderers[tab]) tab = 'overview';
  const changed = tab !== current;
  current = tab;
  if ($('#dialog').open) $('#dialog').close();
  $$('.tabs button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.tab === tab)));
  $$('.tab').forEach((s) => s.classList.toggle('active', s.id === `tab-${tab}`));
  if (location.hash !== `#${tab}`) history.replaceState(null, '', `#${tab}`);
  view?.setActive(tab === 'view3d');
  if (changed || rerender) {
    try { await renderers[tab](); } catch (e) { toast(e.message, true); }
  }
}

$$('.tabs button').forEach((b) => b.addEventListener('click', () => show(b.dataset.tab)));
window.addEventListener('hashchange', () => show(location.hash.slice(1) || 'overview'));

$('#btn-optimize').addEventListener('click', async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true;
  try {
    const run = await api('POST', 'api/optimize', { force: e.shiftKey });
    toast(runText(run));
    await refresh();
  } catch (err) {
    toast(err.message, true);
  } finally {
    btn.disabled = false;
  }
});

// ---------------------------------------------------------------- overview

function stackBar(counts, total, thin = false) {
  const segs = STATUS_ORDER.filter((k) => counts[k])
    .map((k) => `<span style="flex:${counts[k]};background:${STATUS_FILL[STATUS[k][0]]}" title="${esc(STATUS[k][1])}: ${int(counts[k])}"></span>`)
    .join('');
  return `<div class="stackbar${thin ? ' thin' : ''}" role="img" aria-label="Tracker status breakdown">${segs || ''}</div>`;
}

renderers.overview = async () => {
  const { summary: s, watch: w0, watch_folder: wf } = await loadSummary(true);
  const watch = BROWSER ? browser.watch : w0;
  const watch_folder = BROWSER ? browser.watch.folder : wf;
  const el = $('#tab-overview');
  if (!s.piles) {
    el.innerHTML = `<div class="card empty">
      <h2>Start with the master pile list</h2>
      <p>The database is empty. Load the original pile list once; it becomes the immutable core that every survey is matched against.</p>
      <ol class="steps">
        <li><b>Import the master pile list</b> (pile ID, Easting, Northing, pile type, colour). A ground elevation column such as <i>Z LIDAR</i> is kept as LIDAR ground.</li>
        <li><b>Import daily survey reports</b>, or point the watch folder at your Trimble Connect Sync folder in Settings.</li>
        <li>Trackers are <b>optimised automatically</b> once every pile has an elevation. Read the results, look at the 3D view, export to Excel.</li>
      </ol>
      <button class="btn primary" id="go-import">Go to Import</button>
    </div>`;
    $('#go-import').addEventListener('click', () => show('import'));
    return;
  }
  const st = s.tracker_status;
  const done = (st.Compliant || 0) + (st.Graded || 0);
  const lastRun = s.last_run;
  const ls = s.last_survey;
  const blocks = s.blocks.map((b) => `<tr>
      <td>${esc(b.block || '(none)')}</td>
      <td class="num">${int(b.trackers)}</td>
      <td class="num">${int(b.piles)}</td>
      <td class="bar-cell"><div style="display:flex;align-items:center;gap:8px"><div class="mini-bar" style="flex:1"><span style="width:${pct(b.surveyed, b.piles)}%"></span></div><span class="num" style="width:3.5em">${pct(b.surveyed, b.piles)}%</span></div></td>
      <td class="bar-cell">${stackBar(b.counts, b.trackers, true)}</td>
      ${['Compliant', 'Graded', 'NonCompliant', 'AwaitingSurvey'].map((k) => `<td class="num">${int(b.counts[k] || 0)}</td>`).join('')}
    </tr>`).join('');
  const events = (watch.events || []).slice(0, 4).map((e) => `<li>${e.accepted ? '<span class="pill ok">✓</span>' : '<span class="pill bad">✕</span>'} <b>${esc(e.file)}</b> <span class="muted">${esc(e.at)}</span><br><span class="ink2">${esc(e.message)}</span></li>`).join('');
  el.innerHTML = `
    <div class="tiles">
      <div class="tile"><div class="label">Piles in master list</div><div class="value">${int(s.piles)}</div><div class="sub">${int(s.trackers)} trackers · ${s.blocks.length} block${s.blocks.length === 1 ? '' : 's'}</div></div>
      <div class="tile"><div class="label">Piles surveyed</div><div class="value">${int(s.piles_surveyed)}</div><div class="sub">${pct(s.piles_surveyed, s.piles)}% of piles</div></div>
      <div class="tile"><div class="label">LIDAR ground only</div><div class="value">${int(s.piles_lidar_only)}</div><div class="sub">not surveyed yet</div></div>
      <div class="tile"><div class="label">No elevation</div><div class="value">${int(s.piles_no_elevation)}</div><div class="sub">trackers wait for these</div></div>
      <div class="tile"><div class="label">Trackers with a valid ToP</div><div class="value">${int(done)}</div><div class="sub">${pct(done, s.trackers)}% · ${int(st.Graded || 0)} need cut/fill</div></div>
      <div class="tile"><div class="label">Out of date</div><div class="value">${int(s.stale_trackers)}</div><div class="sub">${s.stale_trackers ? 'click Optimise to update' : 'all results current'}</div></div>
      <div class="tile"><div class="label">Piles with a flood value</div><div class="value">${int(s.piles_with_flood)}</div><div class="sub">${s.piles_with_flood ? `${int(s.piles_flood_raised)} raised above the flood` : 'import them when available'}</div></div>
    </div>
    <div class="card" style="margin-top:16px">
      <div class="card-head"><h2>Trackers by status</h2><span class="muted">${int(s.trackers)} trackers</span></div>
      ${stackBar(st, s.trackers)}
      <div class="legend">${STATUS_ORDER.filter((k) => st[k]).map((k) => `<span class="item"><span class="sw" style="background:${STATUS_FILL[STATUS[k][0]]}"></span>${esc(STATUS[k][1])} <b>${int(st[k])}</b></span>`).join('')}</div>
    </div>
    <div class="card">
      <div class="card-head"><h2>Progress by block</h2></div>
      <div class="table-wrap"><table>
        <thead><tr><th>Block</th><th class="num">Trackers</th><th class="num">Piles</th><th>Surveyed</th><th>Status</th><th class="num">Compliant</th><th class="num">Graded</th><th class="num">Non-compliant</th><th class="num">Awaiting</th></tr></thead>
        <tbody>${blocks}</tbody>
      </table></div>
    </div>
    <div class="grid cols-3" style="margin-top:16px">
      <div class="card"><div class="card-head"><h3>Last survey import</h3><a href="#history">History</a></div>
        ${ls ? `<p><b>${esc(ls.file_name)}</b></p><p class="ink2">Survey of ${esc(ls.survey_date || '–')} · imported ${esc(ls.imported_at)} (${esc(ls.origin)})<br>${int(ls.rows_used)} piles updated, ${int(ls.rows_rejected)} rows skipped</p>` : '<p class="muted">No field survey imported yet.</p>'}
      </div>
      <div class="card"><div class="card-head"><h3>Last optimisation</h3></div>
        ${lastRun ? `<p class="ink2">${esc(lastRun.created_at)} · ${esc(lastRun.trigger)}</p><p>${esc(runText(lastRun))}</p>` : '<p class="muted">Not run yet.</p>'}
      </div>
      <div class="card"><div class="card-head"><h3>Watch folder</h3><a href="#settings">Settings</a></div>
        ${watch_folder ? `<p class="mono" style="word-break:break-all">${esc(watch_folder)}</p><p class="ink2">${watch.error ? `<span style="color:var(--critical-text)">${esc(watch.error)}</span>` : `Last scan ${esc(watch.last_scan || 'pending')}`}</p>${events ? `<ul class="findings">${events}</ul>` : ''}` : '<p class="muted">Off. Set a folder in Settings to import new survey files automatically (for example a Trimble Connect Sync folder).</p>'}
      </div>
    </div>`;
};

// ---------------------------------------------------------------- import

function renderReport(rep, run) {
  const ok = rep.accepted;
  const lvl = { error: ['✕', 'lvl-error'], warning: ['!', 'lvl-warning'], info: ['i', 'lvl-info'] };
  const findings = (rep.findings || []).map((f) => {
    const [icon, cls] = lvl[f.level] || ['·', ''];
    const ex = f.examples?.length ? `<details><summary>${f.count > f.examples.length ? `First ${f.examples.length} of ${int(f.count)}` : 'Details'}</summary>${f.examples.map(esc).join('<br>')}</details>` : '';
    return `<li><span class="lvl ${cls}" aria-label="${f.level}">${icon}</span>${esc(f.message)}${f.count > 1 ? ` <span class="muted">× ${int(f.count)}</span>` : ''}${ex}</li>`;
  }).join('');
  const cols = Object.entries(rep.columns || {}).map(([k, v]) => `${esc(k)} ← “${esc(v)}”`).join(' · ');
  return `<div class="report">
    <div class="report-head">
      <span class="pill ${ok ? 'ok' : 'bad'}">${ok ? '✓ Imported' : '✕ Not imported'}</span>
      <b>${esc(rep.file_name)}</b>
      <span class="muted">${esc(KIND[rep.kind] || rep.kind)}${rep.import_id ? ` · import #${rep.import_id}` : ''}${rep.survey_date ? ` · dated ${esc(rep.survey_date)} (${esc(rep.survey_date_source)})` : ''}</span>
    </div>
    ${rep.rows_total ? `<div class="kv"><div><span>Rows read</span><b>${int(rep.rows_total)}</b></div><div><span>Used</span><b>${int(rep.rows_used)}</b></div><div><span>Skipped</span><b>${int(rep.rows_rejected)}</b></div></div>` : ''}
    ${cols ? `<div class="cols">Columns: ${cols}</div>` : ''}
    ${findings ? `<ul class="findings">${findings}</ul>` : ''}
    ${run ? `<p style="margin-top:10px"><b>Optimisation:</b> ${esc(runText(run))}</p>` : ''}
  </div>`;
}

const fileInput = $('#file-input');
let pendingKind = 'survey';
$$('.drop').forEach((d) => {
  const kind = d.dataset.kind;
  const open = () => { pendingKind = kind; fileInput.multiple = kind !== 'master'; fileInput.value = ''; fileInput.click(); };
  d.addEventListener('click', open);
  d.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } });
  d.addEventListener('dragover', (e) => { e.preventDefault(); d.classList.add('over'); });
  d.addEventListener('dragleave', () => d.classList.remove('over'));
  d.addEventListener('drop', (e) => {
    e.preventDefault();
    d.classList.remove('over');
    importFiles(kind, [...e.dataTransfer.files]);
  });
});
fileInput.addEventListener('change', () => importFiles(pendingKind, [...fileInput.files]));

async function importFiles(kind, files) {
  if (!files.length) return;
  const out = $('#import-results');
  const date = kind === 'master' ? '' : $('#survey-date').value;
  for (const file of files) {
    const slot = document.createElement('div');
    slot.className = 'report';
    slot.innerHTML = `<span class="spinner"></span> Importing <b>${esc(file.name)}</b>…`;
    out.prepend(slot);
    try {
      const body = await file.arrayBuffer();
      const q = new URLSearchParams({ kind, name: file.name, date });
      const res = await api('POST', `api/import?${q}`, body, true);
      slot.outerHTML = renderReport(res.report, res.run);
      if (res.report.accepted) toast(`${file.name}: imported. ${runText(res.run)}`);
      else toast(`${file.name}: not imported, see the report.`, true);
    } catch (e) {
      slot.innerHTML = `<span class="pill bad">✕ Failed</span> <b>${esc(file.name)}</b>: ${esc(e.message)}`;
    }
  }
  cache.state = null;
  await loadSummary(true);
  updateHeader();
  renderMasterLock();
}

function renderMasterLock() {
  const s = cache.summary?.summary;
  const el = $('#master-lock');
  if (!s || !s.piles) { el.hidden = true; return; }
  el.hidden = false;
  el.innerHTML = `<span aria-hidden="true">🔒</span><div><b>${int(s.piles)} piles loaded</b> in ${int(s.trackers)} trackers. The master list cannot be changed; a new file can only add piles that are not in it yet.</div>`;
  $('#flood-count').textContent = s.piles_with_flood ? `${int(s.piles_with_flood)} piles have a value` : 'not loaded yet';
}

renderers.import = async () => {
  await loadSummary();
  renderMasterLock();
};

// ---------------------------------------------------------------- results

const rs = { sort: 'tracker', dir: 1, limit: 400, selected: null };

function trackerRowsFor(state, t) {
  const p = state.piles;
  const rows = [];
  let s = 0;
  for (let i = t.first_pile; i < t.first_pile + t.n_piles; i++) {
    if (i > t.first_pile) s += Math.hypot(p.e[i] - p.e[i - 1], p.n[i] - p.n[i - 1]);
    rows.push({ i, id: p.id[i], s, z: p.z[i], ng: p.ng[i], top: p.top[i], rev: p.rev[i], ang: p.ang[i], cf: p.cf[i], zsrc: p.zsrc[i], flvl: p.flvl[i], mintop: p.mintop[i], mta: p.mta[i] });
  }
  return rows;
}

function fillSelect(sel, values, allLabel, labels = {}) {
  const prev = sel.value;
  sel.innerHTML = `<option value="">${allLabel}</option>` + values.map(([v, n]) => `<option value="${esc(v)}">${esc(labels[v] ?? v)}${n != null ? ` (${int(n)})` : ''}</option>`).join('');
  if ([...sel.options].some((o) => o.value === prev)) sel.value = prev;
}

const natural = (a, b) => String(a).localeCompare(String(b), undefined, { numeric: true });

renderers.results = async () => {
  const state = await loadState();
  const blocks = [...new Set(state.trackers.map((t) => t.block))].sort(natural);
  fillSelect($('#f-block'), blocks.map((b) => [b, state.trackers.filter((t) => t.block === b).length]), 'All blocks', { '': '(no block)' });
  const sc = {};
  state.trackers.forEach((t) => (sc[t.status] = (sc[t.status] || 0) + 1));
  fillSelect($('#f-status'), STATUS_ORDER.filter((k) => sc[k]).map((k) => [k, sc[k]]), 'All statuses', Object.fromEntries(STATUS_ORDER.map((k) => [k, STATUS[k][1]])));
  drawTrackerTable();
  if (rs.selected) {
    const t = state.trackers.find((t) => t.tracker === rs.selected);
    if (t) renderDetail(t);
  }
};

function filteredTrackers() {
  const state = cache.state;
  const q = $('#f-search').value.trim().toLowerCase();
  const block = $('#f-block').value, status = $('#f-status').value, staleOnly = $('#f-stale').checked, floodOnly = $('#f-flood').checked;
  let pileTrackers = null;
  if (q) {
    pileTrackers = new Set();
    state.piles.id.forEach((id, i) => { if (id.toLowerCase().includes(q)) pileTrackers.add(state.piles.tracker[i]); });
  }
  const list = state.trackers.filter((t) =>
    (!block || t.block === block) && (!status || t.status === status) && (!staleOnly || t.stale) && (!floodOnly || t.n_flood_raised > 0) &&
    (!q || t.tracker.toLowerCase().includes(q) || pileTrackers.has(t.tracker)));
  const key = {
    tracker: (t) => t.tracker, block: (t) => t.block, n_piles: (t) => t.n_piles, n_survey: (t) => t.n_survey / t.n_piles,
    status: (t) => STATUS_ORDER.indexOf(t.status), max_angle_deg: (t) => t.max_angle_deg ?? -1, max_cut_fill: (t) => t.max_cut_fill ?? -1, run_at: (t) => t.run_at ?? '',
  }[rs.sort];
  list.sort((a, b) => {
    const ka = key(a), kb = key(b);
    const c = typeof ka === 'number' ? ka - kb : natural(ka, kb);
    return c * rs.dir || natural(a.tracker, b.tracker);
  });
  return list;
}

function drawTrackerTable() {
  const list = filteredTrackers();
  $('#f-count').textContent = `${int(list.length)} tracker${list.length === 1 ? '' : 's'}`;
  const cols = [
    ['tracker', 'Tracker', ''], ['block', 'Block', ''], ['n_piles', 'Piles', 'num'], ['n_survey', 'Surveyed', 'num'],
    ['status', 'Status', ''], ['max_angle_deg', 'Angle (°)', 'num', 'Largest angle change between two spans'], ['max_cut_fill', 'Cut/fill (m)', 'num', 'Largest cut or fill at a pile'],
  ];
  const head = cols.map(([k, label, cls, tip]) => `<th class="sortable ${cls}" data-sort="${k}"${tip ? ` title="${tip}"` : ''}>${label}${rs.sort === k ? ` <span class="arrow">${rs.dir > 0 ? '▲' : '▼'}</span>` : ''}</th>`).join('');
  const rows = list.slice(0, rs.limit).map((t) => `<tr class="clickable${t.tracker === rs.selected ? ' selected' : ''}" data-t="${esc(t.tracker)}">
      <td>${esc(t.tracker)}</td><td>${esc(t.block)}</td><td class="num">${t.n_piles}</td>
      <td class="num">${t.n_survey}/${t.n_piles}</td>
      <td>${statusBadge(t.status)}${t.stale && t.run_id ? ' <span class="stale" title="Elevations or settings changed since this result">out of date</span>' : ''}</td>
      <td class="num">${fmt(t.max_angle_deg, 3)}</td><td class="num">${fmt(t.max_cut_fill, 3)}</td></tr>`).join('');
  const more = list.length > rs.limit ? `<tr><td colspan="7" style="text-align:center"><button class="btn small" id="show-more">Show ${int(Math.min(400, list.length - rs.limit))} more</button></td></tr>` : '';
  const table = $('#tracker-table');
  table.innerHTML = `<thead><tr>${head}</tr></thead><tbody>${rows || '<tr><td colspan="7" class="muted">No tracker matches these filters.</td></tr>'}${more}</tbody>`;
  $$('th[data-sort]', table).forEach((th) => th.addEventListener('click', () => {
    const k = th.dataset.sort;
    rs.dir = rs.sort === k ? -rs.dir : 1;
    rs.sort = k;
    drawTrackerTable();
  }));
  $$('tr[data-t]', table).forEach((tr) => tr.addEventListener('click', () => {
    rs.selected = tr.dataset.t;
    $$('tr.selected', table).forEach((r) => r.classList.remove('selected'));
    tr.classList.add('selected');
    renderDetail(cache.state.trackers.find((t) => t.tracker === rs.selected));
  }));
  $('#show-more')?.addEventListener('click', () => { rs.limit += 400; drawTrackerTable(); });
}

['#f-search', '#f-block', '#f-status', '#f-stale', '#f-flood'].forEach((s) => $(s).addEventListener('input', () => { rs.limit = 400; if (cache.state) drawTrackerTable(); }));

function renderDetail(t) {
  const state = cache.state, settings = cache.summary.settings;
  const p = state.piles;
  const rows = trackerRowsFor(state, t);
  const el = $('#tracker-detail');
  const hasFlood = t.n_flood > 0;
  const piles = rows.map((r) => {
    const i = r.i;
    const over = r.ang != null && Math.abs(r.ang) > settings.max_angle_deg + 0.001;
    const sh = p.shift[i];
    const big = sh != null && Math.abs(sh) > settings.shift_warn_m;
    const shiftCell = `<td class="num" title="${p.se[i] == null ? '' : `Set-out E ${fmt(p.se[i], 3)}  N ${fmt(p.sn[i], 3)}`}"${big ? ' style="color:var(--warning-text);font-weight:600"' : ''}>${sh == null ? '–' : (sh * 1000).toFixed(0)}</td>`;
    const flood = p.flood[i] == null ? '–' : `${fmt(p.flood[i], 2)}${p.mta[i] ? ' <span class="raised" title="Top raised to stay above the flood level">▲</span>' : ''}`;
    return `<tr>
      <td>${esc(p.id[i])}</td><td><span class="type-cell">${swatch(p.color[i])}${esc(p.type[i])}</span></td>
      <td class="num">${fmt(p.z[i])}</td>
      <td class="muted">${p.zsrc[i] === 'survey' ? `survey ${esc(p.zdate[i])}` : p.zsrc[i] === 'lidar' ? 'LIDAR' : 'none'}${p.offset[i] != null && p.offset[i] > settings.offset_warn_m ? ` <span class="stale" title="Surveyed position is ${fmt(p.offset[i], 2)} m from design">⚠ ${fmt(p.offset[i], 2)} m</span>` : ''}</td>
      <td class="num">${fmt(p.cf[i])}</td><td class="num"><b>${fmt(p.top[i])}</b></td>${shiftCell}<td class="num">${fmt(p.rev[i])}</td>
      ${hasFlood ? `<td class="num" title="${p.mintop[i] == null ? '' : `Lowest allowed top: ${fmt(p.mintop[i])}`}">${flood}</td>` : ''}
      <td class="num"${over ? ' style="color:var(--critical-text);font-weight:600"' : ''}>${fmt(p.ang[i], 3)}${p.outl[i] ? ' <span title="Elevation is an outlier within this tracker">*</span>' : ''}</td></tr>`;
  }).join('');
  el.innerHTML = `
    <div class="card-head"><h2>${esc(t.tracker)}</h2><div style="display:flex;gap:8px;align-items:center">${statusBadge(t.status)}<button class="btn small" id="detail-3d">Show in 3D</button></div></div>
    ${t.stale && t.run_id ? '<p class="stale" style="margin-bottom:8px">Out of date: elevations or settings changed since this result. Click Optimise.</p>' : ''}
    ${t.n_missing ? `<p class="ink2" style="margin-bottom:8px">${t.n_missing} pile${t.n_missing === 1 ? ' has' : 's have'} no elevation yet; the tracker is optimised once every pile has one.</p>` : ''}
    <div class="kv">
      <div><span>Block / section</span><b>${esc(t.block || '–')} / ${esc(t.section || '–')}</b></div>
      ${t.tracker_type ? `<div style="grid-column:span 2"><span>Tracker type</span><b>${esc(t.tracker_type)}</b></div>` : ''}
      <div><span>Piles surveyed</span><b>${t.n_survey} of ${t.n_piles}</b></div>
      <div><span>LIDAR only</span><b>${t.n_lidar}</b></div>
      <div><span>Max angle change</span><b>${fmt(t.max_angle_deg, 3)}°</b></div>
      <div><span>Max cut/fill</span><b>${fmt(t.max_cut_fill, 3)} m</b></div>
      ${t.anchor_pile ? `<div><span>Fixed pile (set-out)</span><b>${esc(t.anchor_pile.split('.').pop())}${t.anchor_motor ? ' · motor' : ' · middle'}</b></div>
      <div><span>Largest shift</span><b>${((t.max_shift ?? 0) * 1000).toFixed(0)} mm</b></div>
      <div title="Largest difference between a span's length along the tube and its design length"><span>Span check</span><b>${t.span_error != null && t.span_error < 0.0005 ? '✓ ' : ''}${((t.span_error ?? 0) * 1000).toFixed(2)} mm</b></div>` : ''}
      <div><span>Result of</span><b>${esc(t.run_at?.slice(0, 16) ?? '–')}</b></div>
      <div><span>Flood values</span><b>${t.n_flood ? `${t.n_flood} of ${t.n_piles} · ${t.n_flood_raised} raised` : 'none'}</b></div>
    </div>
    <div id="profile"></div>
    <div class="table-wrap" style="margin-top:12px"><table>
      <thead><tr><th>Pile</th><th>Type</th><th class="num">Ground Z</th><th>Source</th><th class="num">Cut/fill</th><th class="num">ToP</th><th class="num" title="Move along the tracker from the design position, to keep the torque-tube spans at their design length. Hover for the set-out coordinates.">Shift (mm)</th><th class="num">Reveal</th>${hasFlood ? '<th class="num" title="Flood value (▲: top raised above the flood level)">Flood</th>' : ''}<th class="num">Angle (°)</th></tr></thead>
      <tbody>${piles}</tbody></table></div>`;
  renderProfile($('#profile', el), rows, settings);
  $('#detail-3d').addEventListener('click', async () => {
    await show('view3d');
    if (view?.focusTracker(t.tracker)) $('#v3-find').value = t.tracker;
  });
}

// ---------------------------------------------------------------- 3D

let view = null;
let View3DClass = null;

renderers.view3d = async () => {
  const [state, sum] = await Promise.all([loadState(), loadSummary()]);
  if (!View3DClass) View3DClass = (await import('./view3d.js')).View3D;
  if (!view) {
    view = new View3DClass($('#v3-main'), $('#v3-legend'), showPileInfo);
    $('#v3-color').addEventListener('change', (e) => view.setOptions({ colorBy: e.target.value }));
    $('#v3-ground').addEventListener('change', (e) => view.setOptions({ ground: e.target.value }));
    $('#v3-wire').addEventListener('change', (e) => view.setOptions({ wire: e.target.checked }));
    $('#v3-water').addEventListener('change', (e) => view.setOptions({ water: e.target.checked }));
    $('#v3-ex').addEventListener('input', (e) => { $('#v3-ex-label').textContent = `${e.target.value}×`; });
    $('#v3-ex').addEventListener('change', (e) => { view.setOptions({ ex: +e.target.value }); view.fit(); });
    $('#v3-block').addEventListener('change', (e) => view.setOptions({ block: e.target.value }));
    $('#v3-fit').addEventListener('click', () => view.fit());
    $('#v3-find').addEventListener('change', (e) => { if (e.target.value && !view.focusTracker(e.target.value.trim())) toast(`Tracker "${e.target.value}" not found.`, true); });
    view.setActive(current === 'view3d');
  }
  const blocks = [...new Set(state.piles.block)].sort(natural);
  fillSelect($('#v3-block'), blocks.map((b) => [b, null]), 'All blocks', { '': '(no block)' });
  $('#v3-trackers').innerHTML = state.trackers.map((t) => `<option value="${esc(t.tracker)}">`).join('');
  if (!state.piles.id.length) { $('#v3-legend').innerHTML = '<span class="muted">No piles yet: import the master pile list.</span>'; return; }
  if (view.data !== state) view.setData(state, sum.settings);
};

function showPileInfo(i) {
  const st = cache.state, p = st.piles, s = cache.summary.settings;
  const t = st.trackers.find((t) => i >= t.first_pile && i < t.first_pile + t.n_piles);
  const row = (k, v) => `<div class="row"><span>${k}</span><span class="num">${v}</span></div>`;
  const el = $('#v3-info');
  el.hidden = false;
  el.innerHTML = `<div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline"><b>${esc(p.id[i])}</b><button class="btn small" id="v3-close" aria-label="Close">✕</button></div>
    <div style="margin:6px 0">${statusBadge(t?.status)}</div>
    ${row('Tracker', esc(p.tracker[i]))}
    ${row('Pile type', `<span class="type-cell">${swatch(p.color[i])}${esc(p.type[i] || '–')}</span>`)}
    ${p.ttype[i] ? row('Tracker type', esc(p.ttype[i])) : ''}
    ${row('Easting', fmt(p.e[i], 3))}
    ${row('Northing', fmt(p.n[i], 3))}
    ${p.se[i] != null ? row('Set-out E / N', `${fmt(p.se[i], 3)} / ${fmt(p.sn[i], 3)}`) + row('Shift along tracker', `${(p.shift[i] * 1000).toFixed(0)} mm`) : ''}
    ${row('Ground Z', `${fmt(p.z[i])} <span class="muted">${p.zsrc[i] === 'survey' ? 'survey ' + esc(p.zdate[i]) : p.zsrc[i] === 'lidar' ? 'LIDAR' : ''}</span>`)}
    ${row('Cut / fill', fmt(p.cf[i]))}
    ${row('Top of pile', `<b>${fmt(p.top[i])}</b>`)}
    ${row('Reveal', fmt(p.rev[i]))}
    ${p.flood[i] != null ? row('Flood water level', `${fmt(p.flvl[i])} <span class="muted">value ${fmt(p.flood[i], 2)}</span>`) + row('Lowest allowed top', `${fmt(p.mintop[i])}${p.mta[i] ? ' <span class="muted">raised</span>' : ''}`) : ''}
    ${row('Angle change', `${fmt(p.ang[i], 3)}° <span class="muted">/ ${s.max_angle_deg}°</span>`)}
    <button class="btn small" id="v3-open" style="margin-top:8px;width:100%">Open tracker in Results</button>`;
  $('#v3-close').addEventListener('click', () => { el.hidden = true; });
  $('#v3-open').addEventListener('click', async () => {
    rs.selected = p.tracker[i];
    $('#f-search').value = p.tracker[i];
    await show('results');
    renderDetail(t);
  });
}

// ---------------------------------------------------------------- history

renderers.history = async () => {
  const list = await api('GET', 'api/imports');
  const rows = list.map((r) => `<tr class="${r.voided_at ? 'void' : ''}">
    <td class="num">${r.import_id}</td><td>${esc(KIND[r.kind] || r.kind)}</td><td>${esc(r.file_name)}</td>
    <td>${esc(r.survey_date || '')}</td><td class="muted">${esc(r.imported_at)}</td><td class="muted">${esc(r.origin)}</td>
    <td class="num">${int(r.rows_total)}</td><td class="num">${int(r.rows_used)}</td><td class="num">${int(r.rows_rejected)}</td>
    <td class="keep">${r.voided_at ? `<span class="pill void" title="${esc(r.void_reason)}">Void · ${esc(r.voided_at.slice(0, 10))}</span>` : '<span class="pill ok">Active</span>'}</td>
    <td class="keep" style="white-space:nowrap"><button class="btn small" data-report="${r.import_id}">Report</button>
      ${r.kind !== 'master' && !r.voided_at ? `<button class="btn small danger" data-void="${r.import_id}">Void…</button>` : ''}</td></tr>`).join('');
  const table = $('#imports-table');
  table.innerHTML = `<thead><tr><th class="num">#</th><th>Type</th><th>File</th><th>Survey date</th><th>Imported</th><th>Origin</th><th class="num">Rows</th><th class="num">Used</th><th class="num">Skipped</th><th>Status</th><th></th></tr></thead>
    <tbody>${rows || '<tr><td colspan="11" class="muted">Nothing imported yet.</td></tr>'}</tbody>`;
  $$('[data-report]', table).forEach((b) => b.addEventListener('click', async () => {
    const r = await api('GET', `api/imports/${b.dataset.report}`);
    const rep = r.report || { file_name: r.file_name, kind: r.kind, accepted: true, findings: [] };
    rep.import_id = r.import_id;
    openDialog(`${renderReport(rep)}${r.voided_at ? `<p style="margin-top:12px" class="ink2"><b>Voided</b> on ${esc(r.voided_at)}: ${esc(r.void_reason)}</p>` : ''}`, [['Close', null]]);
  }));
  $$('[data-void]', table).forEach((b) => b.addEventListener('click', () => {
    const id = b.dataset.void;
    const row = list.find((r) => String(r.import_id) === id);
    openDialog(`<h2>Void import #${id}?</h2>
      <p class="ink2" style="margin:8px 0 12px">${esc(row.file_name)}: its ${int(row.rows_used)} elevations stop being used (the previous survey or LIDAR value comes back) and affected trackers are re-optimised. The file stays in the history and the action cannot be undone, but the file can be imported again.</p>
      <label class="field"><span>Reason (kept in the history)</span><textarea id="void-reason" rows="2" style="width:100%"></textarea></label>`,
    [['Cancel', null], ['Void import', async () => {
      const reason = $('#void-reason').value.trim();
      if (!reason) { toast('Give a reason.', true); return false; }
      const res = await api('POST', `api/imports/${id}/void`, { reason });
      toast(`Import #${id} voided. ${runText(res.run)}`);
      await refresh();
      return true;
    }, 'danger']]);
  }));
};

function openDialog(html, buttons) {
  const d = $('#dialog');
  $('#dialog-body').innerHTML = html;
  const actions = $('#dialog-actions');
  actions.innerHTML = '';
  for (const [label, fn, cls] of buttons) {
    const b = document.createElement('button');
    b.className = `btn ${cls === 'danger' ? 'primary' : ''}`;
    if (cls === 'danger') b.style.background = 'var(--critical)';
    if (cls === 'danger') b.style.borderColor = 'var(--critical)';
    b.textContent = label;
    b.addEventListener('click', async () => {
      if (!fn) return d.close();
      try { if ((await fn()) !== false) d.close(); } catch (e) { toast(e.message, true); }
    });
    actions.appendChild(b);
  }
  d.showModal();
}

// ---------------------------------------------------------------- settings

const OPT_FIELDS = [
  ['reveal_nominal', 'Nominal reveal', 'm', 'Target height of the top of pile above ground.'],
  ['delta_min', 'Lowest reveal, offset from nominal', 'm', 'Negative. Lowest reveal = nominal + this.'],
  ['delta_max', 'Highest reveal, offset from nominal', 'm', 'Highest reveal = nominal + this.'],
  ['max_angle_deg', 'Max angle change between spans', '°', 'The NX Horizon XTR limit is 0.75°; 0.70° keeps a margin.'],
  ['max_cut_fill', 'Max cut/fill at a pile', 'm', 'Grading is never larger than this.'],
  ['grading_threshold', 'Grading counted above', 'm', 'A tracker needing more cut/fill than this is "Graded".'],
];
const MATCH_FIELDS = [
  ['match_radius_m', 'Match by position within', 'm', 'A survey point with an unknown ID is matched to the nearest pile within this distance.'],
  ['offset_warn_m', 'Warn if off design position by more than', 'm', 'The point is kept and flagged in the import report.'],
  ['offset_reject_m', 'Reject if off design position by more than', 'm', 'Almost always a wrong point ID.'],
  ['jump_warn_m', 'Warn if the elevation changed by more than', 'm', 'Compared with the previous survey of the same pile.'],
];

function settingsInputs(fields, values) {
  return fields.map(([k, label, unit, help]) => `<label class="field"><span>${label} (${unit})</span>
    <input type="number" step="any" name="${k}" value="${values[k]}" required><small>${help}</small></label>`).join('');
}

function renderDatabaseCard(sum) {
  if (BROWSER) {
    $('#db-where').textContent = 'Stored in this browser';
    $('#db-help').textContent = 'Everything is stored in this browser, on this computer only: nothing is sent to a server. Download a copy regularly as a backup. Clearing this site\'s data in the browser, or using another browser or computer, starts from an empty database. A copy can be opened here (Open a database file) or in the desktop program (save it as tracker-top.db next to tracker-top.exe).';
  } else {
    $('#db-where').textContent = sum.summary.db_path;
    $('#db-help').textContent = 'Everything is in this one file. Download a copy as a backup (or copy the file while the program is closed). A copy can also be opened in the browser version (Settings > Database > Open a database file).';
  }
}

function renderBrowserWatch() {
  const w = browser.watch;
  const supported = browser.folderAccessSupported();
  $('#watch-choose').disabled = !supported;
  $('#watch-choose').textContent = w.folder ? 'Choose another folder…' : 'Choose a folder…';
  $('#watch-resume').hidden = !w.needs_permission;
  $('#watch-stop').hidden = !w.folder;
}

if (BROWSER) {
  $('#watch-choose').addEventListener('click', async () => {
    try { await browser.chooseFolder(); toast(`Watching ${browser.watch.folder}.`); } catch (e) { if (e.name !== 'AbortError') toast(e.message, true); }
    await renderers.settings();
  });
  $('#watch-resume').addEventListener('click', async () => { await browser.resumeWatching(); await renderers.settings(); });
  $('#watch-stop').addEventListener('click', async () => { await browser.stopWatching(); await renderers.settings(); });
  $('#db-open').addEventListener('click', () => { $('#db-file').value = ''; $('#db-file').click(); });
  $('#db-file').addEventListener('change', () => {
    const file = $('#db-file').files[0];
    if (!file) return;
    openDialog(`<h2>Open ${esc(file.name)}?</h2><p class="ink2" style="margin-top:8px">It replaces the database stored in this browser. Download a copy of the current one first if you may need it.</p>`,
      [['Cancel', null], ['Open it', async () => {
        await browser.restore(file);
        toast(`${file.name} opened.`);
        await refresh();
      }, 'danger']]);
  });
  $('#db-reset').addEventListener('click', () => {
    openDialog('<h2>Start a new, empty database?</h2><p class="ink2" style="margin-top:8px">Everything stored in this browser (master list, surveys, results) is deleted. Download a copy first if you may need it.</p>',
      [['Cancel', null], ['Delete and start again', async () => {
        await browser.reset();
        toast('Empty database started.');
        await refresh();
      }, 'danger']]);
  });
}
document.querySelectorAll('.browser-only').forEach((el) => { el.hidden = !BROWSER; });
document.querySelectorAll('.desktop-only').forEach((el) => { el.hidden = BROWSER; });

renderers.settings = async () => {
  const [{ settings, defaults, watch_folder }, sum] = await Promise.all([api('GET', 'api/settings'), loadSummary(true)]);
  $('#settings-opt').innerHTML = settingsInputs(OPT_FIELDS, settings);
  $('#settings-match').innerHTML = settingsInputs(MATCH_FIELDS, settings);
  const setFlood = (v) => {
    $('[name="flood_mode"]').value = v.flood_mode;
    $('[name="flood_clearance_m"]').value = v.flood_clearance_m;
    $('[name="epsg"]').value = v.epsg ?? '';
    $('[name="fixed_spans"]').checked = !!v.fixed_spans;
    $('[name="shift_warn_m"]').value = v.shift_warn_m;
  };
  setFlood(settings);
  $('#watch-folder').value = watch_folder || '';
  renderDatabaseCard(sum);
  $('#settings-defaults').onclick = () => {
    $('#settings-opt').innerHTML = settingsInputs(OPT_FIELDS, defaults);
    $('#settings-match').innerHTML = settingsInputs(MATCH_FIELDS, defaults);
    setFlood(defaults);
    toast('Defaults restored in the form. Click Save settings to apply them.');
  };
  const w = BROWSER ? browser.watch : sum.watch;
  if (BROWSER) renderBrowserWatch();
  const events = (w.events || []).map((e) => `<tr><td class="muted">${esc(e.at)}</td><td>${esc(e.file)}</td><td>${e.accepted ? '<span class="pill ok">✓ Imported</span>' : '<span class="pill bad">✕ Not imported</span>'}</td><td class="ink2">${esc(e.message)}</td></tr>`).join('');
  $('#watch-status').innerHTML = (BROWSER ? w.folder : watch_folder)
    ? `<p class="ink2">${w.error ? `<span style="color:var(--critical-text)">${esc(w.error)}</span>` : `Last scan: ${esc(w.last_scan || 'pending')} · ${int(w.files_seen)} files checked`}</p>
       ${events ? `<div class="table-wrap" style="margin-top:8px"><table><thead><tr><th>Time</th><th>File</th><th>Result</th><th>Details</th></tr></thead><tbody>${events}</tbody></table></div>` : `<p class="muted" style="margin-top:6px">No new file seen since ${BROWSER ? 'the page was opened' : 'the program started'}.</p>`}`
    : (BROWSER && !browser.folderAccessSupported()
      ? '<p class="muted">This browser cannot read folders: use Microsoft Edge or Google Chrome for the watch folder. Files can always be dropped on the Import page.</p>'
      : '');
};

$('#settings-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const settings = {};
  for (const [k] of [...OPT_FIELDS, ...MATCH_FIELDS]) settings[k] = Number($(`[name="${k}"]`).value);
  settings.flood_mode = $('[name="flood_mode"]').value;
  settings.flood_clearance_m = Number($('[name="flood_clearance_m"]').value);
  const epsg = $('[name="epsg"]').value.trim();
  settings.epsg = epsg ? Number(epsg) : null;
  settings.fixed_spans = $('[name="fixed_spans"]').checked;
  settings.shift_warn_m = Number($('[name="shift_warn_m"]').value);
  const btn = e.submitter;
  if (btn) btn.disabled = true;
  try {
    const res = await api('PUT', 'api/settings', { settings, watch_folder: BROWSER ? null : ($('#watch-folder').value.trim() || null) });
    toast(`Settings saved. ${runText(res.run)}`);
    await refresh();
  } catch (err) {
    toast(err.message, true);
  } finally {
    if (btn) btn.disabled = false;
  }
});

// ---------------------------------------------------------------- start

async function poll() {
  // Pick up watch-folder imports made in the background.
  try {
    const before = cache.summary;
    const after = await api('GET', 'api/summary');
    const sig = (s) => s && `${s.summary.imports}/${s.summary.last_run?.run_id}`;
    if (before && sig(before) !== sig(after)) {
      cache.summary = after;
      cache.state = null;
      updateHeader();
      const ev = (BROWSER ? browser.watch : after.watch).events?.[0];
      if (ev) toast(`Watch folder: ${ev.file}: ${ev.message}`, !ev.accepted);
      if (current !== 'import') await show(current, true);
    }
  } catch { /* server stopped */ }
}

(async () => {
  if (BROWSER) {
    document.body.classList.add('browser-mode');
    browser = await import('./browser.js');
    browser.keepStorage();
    browser.onWatchEvent(() => poll(true));
    await browser.restoreWatch();
    // Exports and the database copy are generated in the page, then saved.
    document.addEventListener('click', (e) => {
      const a = e.target.closest('a[href^="api/"]');
      if (!a) return;
      e.preventDefault();
      a.setAttribute('aria-busy', 'true');
      browser.download(a.getAttribute('href'))
        .catch((err) => toast(err.message, true))
        .finally(() => a.removeAttribute('aria-busy'));
      toast('Preparing the download…');
    });
  }
  try {
    await loadSummary();
    updateHeader();
    if (BROWSER) {
      const n = await browser.notice();
      if (n) toast(n, true);
    }
  } catch (e) {
    toast(`Cannot reach the program: ${e.message}`, true);
  }
  await show(location.hash.slice(1) || 'overview', true);
  setInterval(poll, 15000);
})();
