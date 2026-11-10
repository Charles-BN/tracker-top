import { renderProfile } from './profile.js';
import { createSync, folderAccessSupported } from './sync.js';

const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const fmt = (v, d = 3) => {
  if (v == null || Number.isNaN(v)) return '–';
  const s = Number(v).toFixed(d);
  return /^-0\.?0*$/.test(s) ? s.slice(1) : s;
};
const int = (v) => (v ?? 0).toLocaleString();
// Decimals of calculated values (tops, reveal, cut/fill, set-out): 2 when the
// settings round them to the centimetre. Measured ground keeps 3.
const dc = () => ((cache.summary?.settings?.round_step ?? 0) >= 0.0095 ? 2 : 3);
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
const CONFIG = window.TRACKER_TOP_CONFIG || {};
let browser = null;
let sync = null; // shared database (sync.js)

/** API call answering JSON. Changes are passed on to the shared database. */
async function api(method, url, body, raw = false) {
  const data = await apiCall(method, url, body, raw);
  if (method !== 'GET' && !/api\/(sync\/|export\/block_done)/.test(url)) sync?.changed();
  return data;
}

/** API call answering a file: { bytes, filename }. */
async function apiRaw(method, url, body) {
  if (BROWSER) {
    const r = await browser.request(method, url, body, true);
    if (r.status >= 400) {
      let msg = `error ${r.status}`;
      try { msg = JSON.parse(new TextDecoder().decode(r.body)).error ?? msg; } catch { /* not JSON */ }
      throw new Error(msg);
    }
    return { bytes: new Uint8Array(r.body), filename: r.filename };
  }
  const res = await fetch(url, { method, body, headers: body ? { 'Content-Type': 'application/octet-stream' } : {} })
    .catch((e) => { throw unreachable(e); });
  if (!res.ok) {
    const text = await res.text();
    let msg = text || res.statusText;
    try { msg = JSON.parse(text).error ?? msg; } catch { /* not JSON */ }
    throw new Error(msg);
  }
  const cd = res.headers.get('Content-Disposition') || '';
  const m = cd.match(/filename\*=UTF-8''([^;]+)/i) || cd.match(/filename="([^"]+)"/i);
  return { bytes: new Uint8Array(await res.arrayBuffer()), filename: m ? decodeURIComponent(m[1]) : null };
}

/** Desktop program: the page lost its connection to the program. */
function unreachable(e) {
  return new Error(`The tracker-top program on this computer does not answer (${e.message}): is its black window still open? Start tracker-top.exe again, then reload this page.`);
}

/** Download a GET API file (exports). */
async function downloadUrl(url) {
  if (BROWSER) return browser.download(url);
  const a = document.createElement('a');
  a.href = url;
  a.download = '';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

async function apiCall(method, url, body, raw = false) {
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
  const res = await fetch(url, opts).catch((e) => { throw unreachable(e); });
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
  map?.setActive(tab === 'map');
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
  const { summary: s, watch: w0, watch_folder: wf, installers } = await loadSummary(true);
  const inst = Object.fromEntries((installers || []).map((b) => [b.block, b]));
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
      <td class="num">${b.reveal_mean == null ? '–' : fmt(b.reveal_mean, 3)}</td>
      <td>${installerCell(inst[b.block])}</td>
    </tr>`).join('');
  const events = (watch.events || []).slice(0, 4).map((e) => `<li>${e.accepted ? '<span class="pill ok">✓</span>' : '<span class="pill bad">✕</span>'} <b>${esc(e.file)}</b> <span class="muted">${esc(e.at)}</span><br><span class="ink2">${esc(e.message)}</span></li>`).join('');
  el.innerHTML = `
    <div class="tiles">
      <div class="tile"><div class="label">Piles in master list</div><div class="value">${int(s.piles)}</div><div class="sub">${int(s.trackers)} trackers · ${s.blocks.length} block${s.blocks.length === 1 ? '' : 's'}</div></div>
      <div class="tile"><div class="label">Piles surveyed</div><div class="value">${int(s.piles_surveyed)}</div><div class="sub">${pct(s.piles_surveyed, s.piles)}% of piles</div></div>
      <div class="tile" title="Top of pile above the ground elevation in use, over every pile with a result"><div class="label">Average reveal</div><div class="value">${s.reveal_mean == null ? '–' : `${fmt(s.reveal_mean, 3)} m`}</div><div class="sub">${s.reveal_mean == null ? 'no results yet' : `${int(s.piles_with_result)} piles · ${fmt(s.reveal_min, 2)} to ${fmt(s.reveal_max, 2)} m${s.reveal_mean_survey != null && s.reveal_mean_survey !== s.reveal_mean ? ` · surveyed ${fmt(s.reveal_mean_survey, 3)} m` : ''}`}</div></div>
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
        <thead><tr><th>Block</th><th class="num">Trackers</th><th class="num">Piles</th><th>Surveyed</th><th>Status</th><th class="num">Compliant</th><th class="num">Graded</th><th class="num">Non-compliant</th><th class="num">Awaiting</th><th class="num" title="Mean reveal of the block's piles with a result">Avg reveal (m)</th><th title="Excel file for the installers, written to the shared folder once the block is complete">Installers' file</th></tr></thead>
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
      <div class="card"><div class="card-head"><h3>Shared database</h3><a href="#settings">Settings</a></div><div id="ov-share"></div></div>
      <div class="card"><div class="card-head"><h3>Watch folder</h3><a href="#settings">Settings</a></div>
        ${watch_folder ? `<p class="mono" style="word-break:break-all">${esc(watch_folder)}</p><p class="ink2">${watch.error ? `<span style="color:var(--critical-text)">${esc(watch.error)}</span>` : `Last scan ${esc(watch.last_scan || 'pending')}`}</p>${events ? `<ul class="findings">${events}</ul>` : ''}` : '<p class="muted">Off. Set a folder in Settings to import new survey files automatically (for example a Trimble Connect Sync folder).</p>'}
      </div>
    </div>`;
  renderShareSummary();
};

function installerCell(b) {
  if (!b) return '';
  const last = b.last_export;
  if (b.needs_export) {
    const where = sync?.state.target ? 'written at the next sync' : 'connect a shared folder to send it';
    const settings = cache.summary?.settings;
    return settings && !settings.auto_export
      ? `<span class="muted" title="Turned off in Settings > Installers' files">ready${last ? ` (rev ${b.next_revision})` : ''}</span>`
      : `<span class="pill ok" title="${esc(where)}">ready${last ? ` · rev ${b.next_revision}` : ''}</span>`;
  }
  if (last) return `<span title="${esc(last.file_name)}, ${esc(last.exported_at)} by ${esc(last.exported_by)}">✓ rev ${last.revision} <span class="muted">${esc(last.exported_at.slice(0, 10))}</span></span>`;
  const waiting = b.surveyed < b.piles ? `${int(b.piles - b.surveyed)} piles not surveyed yet`
    : b.current < b.trackers ? `${int(b.trackers - b.current)} trackers to optimise`
      : b.waiting;
  return `<span class="muted"${b.ok < b.trackers ? ` title="${esc(b.waiting)}"` : ''}>${esc(waiting)}</span>`;
}

// ---------------------------------------------------------------- shared database

const utcText = (iso) => {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(+d) ? iso : d.toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
};

/** One line on the state of the shared database: [css class, text]. */
function shareState() {
  const st = sync?.state;
  if (!st?.target) return ['off', 'Not shared'];
  if (st.phase === 'needs-permission') return ['warn', st.mode === 'drive' ? 'Sign in to Google Drive' : 'Reconnect the shared folder'];
  if (st.phase === 'conflict') return ['bad', 'Conflict: choose which database to keep'];
  if (st.phase === 'error') return ['bad', 'Sync problem'];
  if (st.phase === 'working') return ['busy', 'Syncing…'];
  if (!st.status) return ['busy', 'Checking…'];
  if (st.status.dirty) return ['warn', 'Changes not in the shared copy yet'];
  return ['ok', 'In step with the shared copy'];
}

function renderSyncChip() {
  const chip = $('#sync-chip');
  const st = sync?.state;
  if (!st?.target) { chip.hidden = true; return; }
  const [cls, text] = shareState();
  chip.hidden = false;
  chip.className = `sync-chip ${cls}`;
  chip.textContent = cls === 'ok' ? '☁ In step' : `☁ ${text}`;
  chip.title = `${st.target.label}${st.lastCheck ? ` · checked ${st.lastCheck.toLocaleTimeString()}` : ''}${st.message ? `\n${st.message}` : ''}`;
}

$('#sync-chip').addEventListener('click', async () => {
  if (sync?.state.phase === 'needs-permission') {
    try { await sync.reconnect(); } catch (e) { toast(e.message, true); }
  } else {
    show('settings');
  }
});

function renderShareSummary() {
  const el = $('#ov-share');
  if (!el) return;
  const st = sync?.state;
  const [cls, text] = shareState();
  if (!st?.target) {
    el.innerHTML = '<p class="muted">Off. Share one database between computers and the browser version through Google Drive: Settings &gt; Shared database.</p>';
    return;
  }
  const status = st.status;
  el.innerHTML = `<p><span class="sync-dot ${cls}"></span><b>${esc(text)}</b></p>
    <p class="ink2" style="margin-top:4px">${esc(st.target.label)}<br>${status?.saved_by ? `Last saved by ${esc(status.saved_by)}, ${esc(utcText(status.saved_at))}` : ''}</p>
    ${st.message && cls !== 'ok' ? `<p class="ink2" style="margin-top:4px">${esc(st.message)}</p>` : ''}`;
}

function renderShareCard() {
  const st = sync.state;
  const [cls, text] = shareState();
  $('#share-pill').innerHTML = `<span class="sync-dot ${cls}"></span>${esc(text)}`;
  const body = $('#share-body');
  const nameField = `<label class="field" style="max-width:420px"><span>Name of this computer, shown to the others as who saved</span>
      <input type="text" id="share-name" value="${esc(st.name)}" placeholder="e.g. Site office laptop" maxlength="60"></label>`;
  if (!st.target) {
    const clientId = st.drive.clientId || CONFIG.googleClientId || '';
    body.innerHTML = `
      <p class="ink2">One database for everyone, kept in a shared folder: the desktop program and the browser version, on this computer or any other, all use it. Each computer keeps its own copy and exchanges it with the shared folder after every change, and every minute or so; changes made at the same time on two computers are merged. No server is involved.</p>
      <div style="margin-top:12px">${nameField}</div>
      <div class="grid cols-2" style="margin-top:14px">
        <div class="option">
          <h3>Folder on this computer</h3>
          <p class="ink2">With <b>Google Drive for desktop</b> installed, choose a folder in your Google Drive, for example <span class="mono">G:\My Drive\Tracker-top</span> or a shared drive. Also works with OneDrive or a network drive. Edge or Chrome.</p>
          <button type="button" class="btn primary" id="share-folder"${folderAccessSupported() ? '' : ' disabled title="Needs Microsoft Edge or Google Chrome"'}>Choose the shared folder…</button>
        </div>
        <div class="option">
          <h3>Google Drive online</h3>
          <p class="ink2">Nothing to install: sign in with Google. Paste the link of the shared Drive folder (open the folder in Drive and copy the address).${clientId ? '' : ' Needs a Google “OAuth client ID” for this web address, created once (README, Shared database).'}</p>
          <label class="field"><span>Drive folder link</span><input type="text" id="share-link" placeholder="https://drive.google.com/drive/folders/…" value="${st.drive.folderId ? `https://drive.google.com/drive/folders/${esc(st.drive.folderId)}` : ''}"></label>
          <label class="field"><span>Folder name, as shown here</span><input type="text" id="share-folder-name" placeholder="Tracker-top" value="${esc(st.drive.folderName)}"></label>
          <label class="field"${CONFIG.googleClientId ? ' hidden' : ''}><span>OAuth client ID</span><input type="text" id="share-client" value="${esc(clientId)}" placeholder="….apps.googleusercontent.com"></label>
          <button type="button" class="btn" id="share-drive">Connect to Google Drive</button>
        </div>
      </div>`;
    const name = () => {
      const n = $('#share-name').value.trim();
      if (!n) { $('#share-name').focus(); throw new Error('Give this computer a name first (shown as who saved).'); }
      return n;
    };
    $('#share-folder').addEventListener('click', async () => {
      try {
        await sync.connectFolder(name());
        toast(`Sharing through ${sync.state.target.label}.`);
      } catch (e) { if (e.name !== 'AbortError') toast(e.message, true); }
      renderShareCard();
    });
    $('#share-drive').addEventListener('click', async () => {
      try {
        await sync.connectDrive(name(), $('#share-client').value, $('#share-link').value, $('#share-folder-name').value);
        toast(`Sharing through ${sync.state.target.label}.`);
      } catch (e) { toast(e.message, true); }
      renderShareCard();
    });
    return;
  }
  const status = st.status;
  body.innerHTML = `
    <div class="kv">
      <div><span>Shared copy</span><b>${esc(st.target.label)}</b></div>
      <div><span>This computer</span><b>${esc(st.name || 'unnamed')}</b></div>
      <div><span>Last saved to the shared folder</span><b>${(() => {
        const by = status?.saved_by ?? st.lastReport?.saved_by;
        const at = status?.saved_by ? status.saved_at : st.lastReport?.saved_at;
        return by ? `${esc(by)}, ${esc(utcText(at))}` : '–';
      })()}</b></div>
      <div><span>Last check</span><b>${st.lastCheck ? st.lastCheck.toLocaleTimeString() : '–'}</b></div>
    </div>
    ${st.phase === 'conflict' ? `<div class="conflict">
        <p><b>The shared database and this computer's cannot be merged.</b></p>
        <p class="ink2" style="margin:4px 0 10px">${esc(st.message)}</p>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          <button type="button" class="btn primary" id="share-use">Use the shared database</button>
          <button type="button" class="btn danger" id="share-replace">Replace it with this computer's</button>
        </div>
        <p class="muted" style="margin-top:8px;font-size:12.5px">Either way, the database that is replaced is first kept as a copy in the shared folder.</p>
      </div>` : ''}
    ${st.phase === 'error' ? `<p style="color:var(--critical-text);margin-bottom:8px">${esc(st.message)}</p>` : ''}
    ${st.phase === 'needs-permission' ? `<p class="stale" style="margin-bottom:8px">${st.mode === 'drive' ? 'Sign in to Google Drive to continue (Google asks again after an hour or after a reload).' : 'The browser asks for your permission again before it uses the shared folder after a reload (choose “Allow on every visit” when offered).'}</p>` : ''}
    <div style="display:flex;gap:8px;flex-wrap:wrap;align-items:end">
      ${st.phase === 'needs-permission' ? '<button type="button" class="btn primary" id="share-reconnect">Reconnect</button>' : '<button type="button" class="btn" id="share-now">Sync now</button>'}
      ${nameField}
      <button type="button" class="btn danger" id="share-off">Stop sharing on this computer</button>
    </div>
    <p class="muted" style="margin-top:10px;font-size:12.5px">The shared file is <span class="mono">tracker-top.db</span> in that folder; don't open or edit it there. Google Drive keeps its earlier versions (in Drive: right-click the file &gt; File information &gt; Manage versions).</p>`;
  $('#share-now')?.addEventListener('click', () => sync.sync('manual'));
  $('#share-reconnect')?.addEventListener('click', async () => { try { await sync.reconnect(); } catch (e) { toast(e.message, true); } });
  $('#share-name').addEventListener('change', (e) => sync.rename(e.target.value));
  $('#share-off').addEventListener('click', () => openDialog('<h2>Stop sharing on this computer?</h2><p class="ink2" style="margin-top:8px">This computer keeps its copy of the database and stops exchanging it with the shared folder. The shared copy and the other computers are not affected.</p>',
    [['Cancel', null], ['Stop sharing', async () => { await sync.disconnect(); toast('No longer shared on this computer.'); }, 'danger']]));
  $('#share-use')?.addEventListener('click', () => openDialog('<h2>Use the shared database?</h2><p class="ink2" style="margin-top:8px">This computer\'s database is saved in the shared folder as a copy, then replaced by the shared one.</p>',
    [['Cancel', null], ['Use the shared one', async () => { await sync.useShared(); }, 'danger']]));
  $('#share-replace')?.addEventListener('click', () => openDialog('<h2>Replace the shared database?</h2><p class="ink2" style="margin-top:8px">The shared database is saved in the folder as a copy, then replaced by this computer\'s. The other computers then get this one (a computer with its own unsaved changes will report the same conflict).</p>',
    [['Cancel', null], ['Replace it', async () => { await sync.replaceShared(); toast('The shared database now is this computer\'s.'); }, 'danger']]));
}

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
    rows.push({ i, id: p.id[i], s, z: p.z[i], zs: p.zs[i], zl: p.zl[i], ng: p.ng[i], top: p.top[i], rev: p.rev[i], ang: p.ang[i], cf: p.cf[i], zsrc: p.zsrc[i], flvl: p.flvl[i], mintop: p.mintop[i], mta: p.mta[i] });
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
      <td class="num">${fmt(t.max_angle_deg, 3)}</td><td class="num">${fmt(t.max_cut_fill, dc())}</td></tr>`).join('');
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
    const d = dc();
    const shiftCell = `<td class="num" title="${p.se[i] == null ? '' : `Set-out E ${fmt(p.se[i], d)}  N ${fmt(p.sn[i], d)}`}"${big ? ' style="color:var(--warning-text);font-weight:600"' : ''}>${sh == null ? '–' : fmt(sh, d)}</td>`;
    const flood = p.flood[i] == null ? '–' : `${fmt(p.flood[i], 2)}${p.mta[i] ? ' <span class="raised" title="Top raised to stay above the flood level">▲</span>' : ''}`;
    const used = (k) => (p.zsrc[i] === k ? ' class="num used" title="Ground in use"' : ' class="num muted"');
    const diff = p.zs[i] != null && p.zl[i] != null ? p.zs[i] - p.zl[i] : null;
    return `<tr>
      <td>${esc(p.id[i])}</td><td><span class="type-cell">${swatch(p.color[i])}${esc(p.type[i])}</span></td>
      <td${used('survey')}>${p.zs[i] == null ? '–' : `<span title="Survey of ${esc(p.zdate[i] ?? '')}">${fmt(p.zs[i])}</span>`}${p.offset[i] != null && p.offset[i] > settings.offset_warn_m ? ` <span class="stale" title="Surveyed position is ${fmt(p.offset[i], 2)} m from design">⚠</span>` : ''}</td>
      <td${used('lidar')}>${fmt(p.zl[i])}</td>
      <td class="num">${diff == null ? '' : fmt(diff, d)}</td>
      <td class="num">${fmt(p.cf[i], d)}</td><td class="num"><b>${fmt(p.top[i], d)}</b></td>${shiftCell}<td class="num">${fmt(p.rev[i], d)}</td>
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
      <div><span>Max cut/fill</span><b>${fmt(t.max_cut_fill, dc())} m</b></div>
      ${t.anchor_pile ? `<div><span>Fixed pile (set-out)</span><b>${esc(t.anchor_pile.split('.').pop())}${t.anchor_motor ? ' · motor' : ' · middle'}</b></div>
      <div><span>Largest shift</span><b>${fmt(t.max_shift ?? 0, dc())} m</b></div>
      <div title="Largest difference between a span's length along the tube and its design length, from the (rounded) set-out positions and tops"><span>Span check</span><b>${t.span_error != null && t.span_error <= (settings.round_step || 0) + 0.0005 ? '✓ ' : ''}${((t.span_error ?? 0) * 1000).toFixed(1)} mm</b></div>` : ''}
      <div><span>Result of</span><b>${esc(t.run_at?.slice(0, 16) ?? '–')}</b></div>
      <div><span>Flood values</span><b>${t.n_flood ? `${t.n_flood} of ${t.n_piles} · ${t.n_flood_raised} raised` : 'none'}</b></div>
    </div>
    <div id="profile"></div>
    <div class="table-wrap" style="margin-top:12px"><table>
      <thead><tr><th>Pile</th><th>Type</th><th class="num" title="Latest field survey (hover for its date). Bold: the ground in use.">Survey Z</th><th class="num" title="LIDAR / design ground. Bold: the ground in use.">LIDAR Z</th><th class="num" title="Survey minus LIDAR (m)">Δ</th><th class="num">Cut/fill</th><th class="num">ToP</th><th class="num" title="Move along the tracker from the design position (m), to keep the torque-tube spans at their design length. Hover for the set-out coordinates.">Shift</th><th class="num">Reveal</th>${hasFlood ? '<th class="num" title="Flood value (▲: top raised above the flood level)">Flood</th>' : ''}<th class="num">Angle (°)</th></tr></thead>
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

/** Details of pile i, in the floating panel of the 3D view or the map. */
function showPileInfo(i, panel = '#v3-info') {
  const st = cache.state, p = st.piles, s = cache.summary.settings;
  const t = st.trackers.find((t) => i >= t.first_pile && i < t.first_pile + t.n_piles);
  const d = dc();
  const row = (k, v) => `<div class="row"><span>${k}</span><span class="num">${v}</span></div>`;
  const el = $(panel);
  const inUse = (k) => (p.zsrc[i] === k ? ' <span class="muted">in use</span>' : '');
  const diff = p.zs[i] != null && p.zl[i] != null ? p.zs[i] - p.zl[i] : null;
  el.hidden = false;
  el.innerHTML = `<div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline"><b>${esc(p.id[i])}</b><button class="btn small" data-close aria-label="Close">✕</button></div>
    <div style="margin:6px 0">${statusBadge(t?.status)}</div>
    ${row('Tracker', esc(p.tracker[i]))}
    ${row('Block', esc(p.block[i] || '–'))}
    ${row('Pile type', `<span class="type-cell">${swatch(p.color[i])}${esc(p.type[i] || '–')}</span>`)}
    ${p.ttype[i] ? row('Tracker type', esc(p.ttype[i])) : ''}
    ${row('Easting', fmt(p.e[i], 3))}
    ${row('Northing', fmt(p.n[i], 3))}
    ${p.se[i] != null ? row('Set-out E / N', `${fmt(p.se[i], d)} / ${fmt(p.sn[i], d)}`) + row('Shift along tracker', `${fmt(p.shift[i], d)} m`) : ''}
    ${row('Survey Z', p.zs[i] == null ? '<span class="muted">not surveyed</span>' : `${fmt(p.zs[i])} <span class="muted">${esc(p.zdate[i] ?? '')}</span>${inUse('survey')}`)}
    ${row('LIDAR Z', `${fmt(p.zl[i])}${inUse('lidar')}`)}
    ${diff != null ? row('Survey − LIDAR', `${fmt(diff, d)} m`) : ''}
    ${row('Cut / fill', fmt(p.cf[i], d))}
    ${row('Top of pile', `<b>${fmt(p.top[i], d)}</b>`)}
    ${row('Reveal', fmt(p.rev[i], d))}
    ${p.flood[i] != null ? row('Flood water level', `${fmt(p.flvl[i], d)} <span class="muted">value ${fmt(p.flood[i], 2)}</span>`) + row('Lowest allowed top', `${fmt(p.mintop[i], d)}${p.mta[i] ? ' <span class="muted">raised</span>' : ''}`) : ''}
    ${row('Angle change', `${fmt(p.ang[i], 3)}° <span class="muted">/ ${s.max_angle_deg}°</span>`)}
    <div style="display:flex;gap:6px;margin-top:8px">
      <button class="btn small" data-open style="flex:1">Open in Results</button>
      ${panel === '#v3-info' ? '' : '<button class="btn small" data-3d style="flex:1">Show in 3D</button>'}
    </div>`;
  el.querySelector('[data-close]').addEventListener('click', () => { el.hidden = true; });
  el.querySelector('[data-open]').addEventListener('click', async () => {
    rs.selected = p.tracker[i];
    $('#f-search').value = p.tracker[i];
    await show('results');
    renderDetail(t);
  });
  el.querySelector('[data-3d]')?.addEventListener('click', async () => {
    await show('view3d');
    if (view?.focusTracker(p.tracker[i])) $('#v3-find').value = p.tracker[i];
  });
}

// ---------------------------------------------------------------- map

let map = null;

renderers.map = async () => {
  const [state, sum] = await Promise.all([loadState(), loadSummary()]);
  if (!map) {
    const { Map2D } = await import('./map2d.js');
    map = new Map2D($('#m2-main'), $('#m2-legend'), (i) => showPileInfo(i, '#m2-info'));
    $('#m2-mode').addEventListener('change', (e) => map.setMode(e.target.value));
    $('#m2-block').addEventListener('change', (e) => map.setBlock(e.target.value));
    $('#m2-lines').addEventListener('change', (e) => map.setOptions({ lines: e.target.checked }));
    $('#m2-labels').addEventListener('change', (e) => map.setOptions({ labels: e.target.checked }));
    $('#m2-outlines').addEventListener('change', (e) => map.setOptions({ outlines: e.target.checked }));
    $('#m2-in').addEventListener('click', () => map.zoom(1.6));
    $('#m2-out').addEventListener('click', () => map.zoom(1 / 1.6));
    $('#m2-fit').addEventListener('click', () => { $('#m2-block').value = ''; map.setBlock(''); });
    $('#m2-find').addEventListener('change', (e) => { if (e.target.value && !map.find(e.target.value)) toast(`"${e.target.value}" not found.`, true); });
    $('#m2-save').addEventListener('click', async () => {
      const blob = await map.image();
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = `tracker-top map ${$('#m2-mode').selectedOptions[0].text} ${new Date().toISOString().slice(0, 10)}.png`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 60_000);
    });
  }
  map.setActive(current === 'map');
  const blocks = [...new Set(state.piles.block)].sort(natural);
  fillSelect($('#m2-block'), blocks.map((b) => [b, null]), 'All blocks', { '': '(no block)' });
  $('#v3-trackers').innerHTML = state.trackers.map((t) => `<option value="${esc(t.tracker)}">`).join('');
  if (!state.piles.id.length) { $('#m2-legend').innerHTML = '<span class="muted">No piles yet: import the master pile list.</span>'; return; }
  if (map.state !== state || map.summary !== sum) map.setData(state, sum);
  // Outlines depend only on the master list: fetched again when it grows.
  if (cache.outlinesFor !== sum.summary.piles) {
    cache.outlinesFor = sum.summary.piles;
    map.setOutlines(await api('GET', 'api/blocks/outlines'));
  }
};

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
    b.className = `btn ${cls === 'danger' || cls === 'primary' ? 'primary' : ''}`;
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

// ---------------------------------------------------------------- custom export

const SHEET_LABEL = { piles: 'Piles', trackers: 'Trackers', imports: 'Imports', settings: 'Settings' };
let exportChoice = null;
try { exportChoice = JSON.parse(localStorage.getItem('tracker-top-export') || 'null'); } catch { /* not kept */ }

async function openExportDialog() {
  const opt = await api('GET', 'api/export/options');
  const keys = opt.columns.map((c) => c.key);
  const blocks = opt.blocks.map((b) => b.block);
  let profiles = opt.profiles;
  const ch = {
    profile: 'Full', columns: keys, blocks, sheets: [...opt.sheets], format: 'xlsx',
    ...(exportChoice || {}),
  };
  ch.columns = ch.columns.filter((k) => keys.includes(k));
  ch.blocks = ch.blocks.filter((b) => blocks.includes(b));
  if (exportChoice && !exportChoice.blocks?.length) ch.blocks = blocks;
  const groups = [...new Set(opt.columns.map((c) => c.group))];
  const box = (name, value, label, checked, extra = '') => `<label class="check"><input type="checkbox" name="${name}" value="${esc(value)}"${checked ? ' checked' : ''}> ${label}${extra}</label>`;
  openDialog(`
    <h2>Custom export</h2>
    <div class="ex-top">
      <label class="field"><span>Profile (a saved choice of columns and sheets, shared)</span>
        <select id="ex-profile">${profiles.map((p) => `<option${p.name === ch.profile ? ' selected' : ''}>${esc(p.name)}</option>`).join('')}<option value="">(custom)</option></select></label>
      <button type="button" class="btn small" id="ex-save">Save as profile…</button>
      <button type="button" class="btn small danger" id="ex-delete">Delete profile</button>
      <span id="ex-save-row" hidden style="display:inline-flex;gap:6px;align-items:center"><input type="text" id="ex-name" placeholder="Profile name" maxlength="40"><button type="button" class="btn small primary" id="ex-save-ok">Save</button></span>
    </div>
    <div class="ex-grid">
      <section>
        <div class="ex-head"><h3>Columns <span class="muted" id="ex-ncols"></span></h3><span><a href="#" data-set="col:all">All</a> · <a href="#" data-set="col:none">None</a></span></div>
        <div class="ex-cols">${groups.map((g) => `<fieldset><legend>${esc(g)}</legend>${opt.columns.filter((c) => c.group === g).map((c) => box('col', c.key, esc(c.header), ch.columns.includes(c.key))).join('')}</fieldset>`).join('')}</div>
      </section>
      <section>
        <div class="ex-head"><h3>Blocks <span class="muted" id="ex-nblocks"></span></h3><span><a href="#" data-set="blk:all">All</a> · <a href="#" data-set="blk:none">None</a></span></div>
        <div class="ex-blocks">${opt.blocks.map((b) => box('blk', b.block, esc(b.block || '(no block)'), ch.blocks.includes(b.block), ` <span class="muted">${int(b.piles)} piles · ${pct(b.surveyed, b.piles)}% surveyed</span>`)).join('')}</div>
      </section>
      <section>
        <h3>Sheets</h3>
        <div class="ex-list">${opt.sheets.map((sh) => box('sheet', sh, SHEET_LABEL[sh] || sh, ch.sheets.includes(sh))).join('')}</div>
        <h3 style="margin-top:14px">Format</h3>
        <div class="ex-list">
          <label class="check"><input type="radio" name="ex-format" value="xlsx"${ch.format !== 'csv' ? ' checked' : ''}> Excel (.xlsx)</label>
          <label class="check"><input type="radio" name="ex-format" value="csv"${ch.format === 'csv' ? ' checked' : ''}> CSV, piles only</label>
        </div>
      </section>
    </div>`, [['Cancel', null], ['Export', () => runExport(opt), 'primary']]);

  const d = $('#dialog');
  const checked = (name) => $$(`input[name="${name}"]`, d).filter((i) => i.checked).map((i) => i.value);
  const count = () => {
    $('#ex-ncols').textContent = `${checked('col').length} of ${keys.length}`;
    $('#ex-nblocks').textContent = `${checked('blk').length} of ${blocks.length}`;
  };
  const applyProfile = (name) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    const cols = p.columns.length ? p.columns : keys;
    const sheets = p.sheets.length ? p.sheets : opt.sheets;
    $$('input[name="col"]', d).forEach((i) => { i.checked = cols.includes(i.value); });
    $$('input[name="sheet"]', d).forEach((i) => { i.checked = sheets.includes(i.value); });
    count();
  };
  $('#ex-profile').addEventListener('change', (e) => applyProfile(e.target.value));
  d.querySelectorAll('input[name="col"], input[name="sheet"]').forEach((i) => i.addEventListener('change', () => { $('#ex-profile').value = ''; count(); }));
  d.querySelectorAll('input[name="blk"]').forEach((i) => i.addEventListener('change', count));
  d.querySelectorAll('[data-set]').forEach((a) => a.addEventListener('click', (e) => {
    e.preventDefault();
    const [name, what] = a.dataset.set.split(':');
    $$(`input[name="${name}"]`, d).forEach((i) => { i.checked = what === 'all'; });
    if (name === 'col') $('#ex-profile').value = '';
    count();
  }));
  const saveProfiles = async (next) => {
    const res = await api('PUT', 'api/export/profiles', next);
    profiles = res.profiles;
    opt.profiles = profiles;
    $('#ex-profile').innerHTML = profiles.map((p) => `<option>${esc(p.name)}</option>`).join('') + '<option value="">(custom)</option>';
  };
  $('#ex-save').addEventListener('click', () => { $('#ex-save-row').hidden = false; $('#ex-name').value = $('#ex-profile').value; $('#ex-name').focus(); });
  $('#ex-save-ok').addEventListener('click', async () => {
    const name = $('#ex-name').value.trim();
    if (!name) return;
    const p = { name, columns: orderedColumns(profiles, $('#ex-profile').value, checked('col'), keys), sheets: checked('sheet') };
    try {
      await saveProfiles([...profiles.filter((x) => x.name.toLowerCase() !== name.toLowerCase()), p]);
      $('#ex-profile').value = name;
      $('#ex-save-row').hidden = true;
      toast(`Profile “${name}” saved.`);
    } catch (e) { toast(e.message, true); }
  });
  $('#ex-delete').addEventListener('click', async () => {
    const name = $('#ex-profile').value;
    if (!name) return;
    try {
      await saveProfiles(profiles.filter((x) => x.name !== name));
      $('#ex-profile').value = '';
      toast(`Profile “${name}” deleted.`);
    } catch (e) { toast(e.message, true); }
  });
  count();
}

/** Columns in the profile's order, then the other ticked ones in the usual order. */
function orderedColumns(profiles, profileName, ticked, keys) {
  const p = profiles.find((x) => x.name === profileName);
  const base = (p?.columns.length ? p.columns : keys).filter((k) => ticked.includes(k));
  return [...base, ...keys.filter((k) => ticked.includes(k) && !base.includes(k))];
}

async function runExport(opt) {
  const d = $('#dialog');
  const checked = (name) => $$(`input[name="${name}"]`, d).filter((i) => i.checked).map((i) => i.value);
  const keys = opt.columns.map((c) => c.key);
  const profile = $('#ex-profile').value;
  const cols = orderedColumns(opt.profiles, profile, checked('col'), keys);
  const blks = checked('blk');
  const sheets = checked('sheet');
  const format = d.querySelector('input[name="ex-format"]:checked').value;
  if (!cols.length) { toast('Choose at least one column.', true); return false; }
  if (!blks.length) { toast('Choose at least one block.', true); return false; }
  if (format === 'xlsx' && !sheets.length) { toast('Choose at least one sheet.', true); return false; }
  exportChoice = { profile, columns: cols, blocks: blks.length === opt.blocks.length ? [] : blks, sheets, format };
  try { localStorage.setItem('tracker-top-export', JSON.stringify(exportChoice)); } catch { /* not kept */ }
  const spec = {
    columns: cols.length === keys.length && cols.every((k, i) => k === keys[i]) ? [] : cols,
    blocks: blks.length === opt.blocks.length ? [] : blks,
    sheets: sheets.length === opt.sheets.length ? [] : sheets,
  };
  toast('Preparing the download…');
  await downloadUrl(`api/export.${format}?spec=${encodeURIComponent(JSON.stringify(spec))}`);
  return true;
}

$('#btn-custom').addEventListener('click', () => openExportDialog().catch((e) => toast(e.message, true)));

// ---------------------------------------------------------------- settings

const OPT_FIELDS = [
  ['reveal_nominal', 'Nominal reveal', 'm', 'Target height of the top of pile above ground.'],
  ['delta_min', 'Lowest reveal, offset from nominal', 'm', 'Negative. Lowest reveal = nominal + this.'],
  ['delta_max', 'Highest reveal, offset from nominal', 'm', 'Highest reveal = nominal + this.'],
  ['max_angle_deg', 'Max angle change between spans', '°', 'The NX Horizon XTR limit is 0.75°; 0.70° keeps a margin.'],
  ['max_cut_fill', 'Max cut/fill at a pile', 'm', 'Grading is never larger than this.'],
  ['grading_threshold', 'Grading counted above', 'm', 'A tracker needing more cut/fill than this is "Graded".'],
  ['round_step', 'Round tops, cut/fill and set-out to', 'm', '0.01 = the centimetre; 0 = exact values. Every limit is checked on the rounded values.'],
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
  const [{ settings, defaults, watch_folder }, sum, opts] = await Promise.all([api('GET', 'api/settings'), loadSummary(true), api('GET', 'api/export/options')]);
  const profileSelect = $('[name="auto_export_profile"]');
  const names = [...new Set([...opts.profiles.map((p) => p.name), settings.auto_export_profile])];
  profileSelect.innerHTML = names.map((n) => `<option>${esc(n)}</option>`).join('');
  $('#settings-opt').innerHTML = settingsInputs(OPT_FIELDS, settings);
  $('#settings-match').innerHTML = settingsInputs(MATCH_FIELDS, settings);
  const setFlood = (v) => {
    $('[name="flood_mode"]').value = v.flood_mode;
    $('[name="flood_pile_height_m"]').value = v.flood_pile_height_m;
    $('[name="flood_module_clearance_m"]').value = v.flood_module_clearance_m;
    $('[name="flood_freeboard_m"]').value = v.flood_freeboard_m;
    floodRule();
    $('[name="epsg"]').value = v.epsg ?? '';
    $('[name="fixed_spans"]').checked = !!v.fixed_spans;
    $('[name="shift_warn_m"]').value = v.shift_warn_m;
    $('[name="auto_export"]').checked = !!v.auto_export;
    $('[name="auto_export_folder"]').value = v.auto_export_folder;
    $('[name="auto_export_profile"]').value = v.auto_export_profile;
    $('[name="auto_export_require_ok"]').checked = !!v.auto_export_require_ok;
  };
  setFlood(settings);
  $('#watch-folder').value = watch_folder || '';
  renderDatabaseCard(sum);
  renderShareCard();
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
  for (const k of ['flood_pile_height_m', 'flood_module_clearance_m', 'flood_freeboard_m']) settings[k] = Number($(`[name="${k}"]`).value);
  const epsg = $('[name="epsg"]').value.trim();
  settings.epsg = epsg ? Number(epsg) : null;
  settings.fixed_spans = $('[name="fixed_spans"]').checked;
  settings.shift_warn_m = Number($('[name="shift_warn_m"]').value);
  settings.auto_export = $('[name="auto_export"]').checked;
  settings.auto_export_folder = $('[name="auto_export_folder"]').value.trim();
  settings.auto_export_profile = $('[name="auto_export_profile"]').value;
  settings.auto_export_require_ok = $('[name="auto_export_require_ok"]').checked;
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

// ---------------------------------------------------------------- updates

// Browser version: browsers and company proxies may keep the old site's files
// for a while after an update. version.json is read past every cache; when it
// names another build, offer to reload. The new address (?v=...) makes sure the
// page itself is fetched again.
let updateOffered = false;
async function checkForUpdate() {
  if (updateOffered) return;
  try {
    const res = await fetch(`version.json?t=${Date.now()}`, { cache: 'no-store' });
    if (!res.ok) return;
    const { build } = await res.json();
    if (!build || build === window.TRACKER_TOP_BUILD) return;
    updateOffered = true;
    const bar = document.createElement('div');
    bar.className = 'update-bar';
    bar.innerHTML = '<span>A new version of tracker-top is available.</span> <button class="btn small primary" type="button">Reload now</button>';
    bar.querySelector('button').addEventListener('click', () => {
      location.replace(`${location.pathname}?v=${encodeURIComponent(build)}${location.hash}`);
    });
    document.body.prepend(bar);
  } catch { /* offline: try again later */ }
}

/** The flood rule in words, from the three flood inputs. */
function floodRule() {
  const n = (k) => Number($(`[name="${k}"]`).value);
  const h = n('flood_pile_height_m'), c = n('flood_module_clearance_m'), f = n('flood_freeboard_m');
  if (![h, c, f].every(Number.isFinite)) return;
  const clear = f + h - c, free = c - f;
  const level = $('[name="flood_mode"]').value === 'elevation';
  $('#flood-rule').innerHTML = `Rule: <b>top of pile ≥ flood ${level ? 'level' : 'water level (ground + depth)'} + ${clear.toFixed(3)} m</b> (${f.toFixed(2)} m freeboard + ${(h - c).toFixed(3)} m from the top of pile to the lowest module edge). With a ${h.toFixed(3)} m pile, water up to ${free.toFixed(2)} m deep needs no raise; deeper, the pile is raised by depth − ${free.toFixed(2)} m (e.g. 0.45 m of water: +${Math.max(0, 0.45 - free).toFixed(2)} m, pile ${(h + Math.max(0, 0.45 - free)).toFixed(3)} m). The optimiser raises a pile only when its top would otherwise be lower.`;
}
['flood_mode', 'flood_pile_height_m', 'flood_module_clearance_m', 'flood_freeboard_m'].forEach((k) => $(`[name="${k}"]`).addEventListener('input', floodRule));

// ---------------------------------------------------------------- start

async function poll() {
  // Pick up watch-folder imports made in the background.
  try {
    const before = cache.summary;
    const after = await api('GET', 'api/summary');
    const sig = (s) => s && `${s.summary.imports}/${s.summary.last_run?.run_id}`;
    if (before && sig(before) !== sig(after)) {
      sync?.changed();
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
  sync = createSync({
    json: apiCall,
    bytes: apiRaw,
    changed: async (rep) => {
      const who = rep.saved_by ? ` (saved by ${rep.saved_by}, ${utcText(rep.saved_at)})` : '';
      if (rep.action === 'adopted') toast(rep.message && !rep.saved_by ? rep.message : `Updated from the shared database${who}.`);
      else if (rep.action === 'merged') toast(`Merged with the shared database${who}: ${rep.imports_added} import${rep.imports_added === 1 ? '' : 's'} from this computer added${rep.voids_applied ? `, ${rep.voids_applied} voided` : ''}.`);
      else if (rep.message) toast(rep.message);
      await refresh();
    },
    exported: (text) => toast(text),
  });
  sync.onUpdate(() => {
    renderSyncChip();
    renderShareSummary();
    if (current === 'settings') renderShareCard();
  });
  await show(location.hash.slice(1) || 'overview', true);
  sync.load().catch((e) => toast(`Shared database: ${e.message}`, true));
  setInterval(poll, 15000);
  if (window.TRACKER_TOP_BUILD) {
    checkForUpdate();
    setInterval(checkForUpdate, 5 * 60_000);
    window.addEventListener('focus', checkForUpdate);
  }
})();
