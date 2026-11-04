// Browser version: answers the page's API requests with the engine running in
// a background worker, instead of the desktop program. Also: downloads,
// database copy / restore, and a watch folder through the browser's folder
// access (Edge / Chrome), active while the page is open.

const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
let nextId = 1;
const pending = new Map();
worker.onmessage = (e) => {
  const p = pending.get(e.data.id);
  if (!p) return;
  pending.delete(e.data.id);
  p(e.data);
};
worker.onerror = (e) => console.error('tracker-top worker', e.message);

function call(msg, transfer = []) {
  const id = nextId++;
  return new Promise((resolve) => {
    pending.set(id, resolve);
    worker.postMessage({ id, ...msg }, transfer);
  });
}

// ------------------------------------------------------------------ requests

/** Same contract as fetch to the desktop program: { status, contentType, body, filename }. */
export async function request(method, url, body, raw = false) {
  const u = new URL(url, location.href);
  let bytes;
  if (body !== undefined && body !== null) {
    bytes = raw ? new Uint8Array(body) : new TextEncoder().encode(JSON.stringify(body));
  }
  const r = await call(
    { op: 'request', method, path: u.pathname.slice(u.pathname.indexOf('api/')), query: u.search.slice(1), body: bytes, watch: JSON.stringify(watch) },
    bytes ? [bytes.buffer] : [],
  );
  if (r.error) return { status: r.status, contentType: 'application/json', body: new TextEncoder().encode(JSON.stringify({ error: r.error })) };
  return r;
}

export async function download(url) {
  const r = await request('GET', url);
  if (r.status >= 400) {
    let msg = 'download failed';
    try { msg = JSON.parse(new TextDecoder().decode(r.body)).error ?? msg; } catch { /* not JSON */ }
    throw new Error(msg);
  }
  const blob = new Blob([r.body], { type: r.contentType });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = r.filename || 'download';
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 60_000);
}

export async function restore(file) {
  const bytes = await file.arrayBuffer();
  const r = await call({ op: 'restore', bytes }, [bytes]);
  if (r.error) throw new Error(r.error);
}

export async function reset() {
  const r = await call({ op: 'reset' });
  if (r.error) throw new Error(r.error);
}

/** Message about a stored database that could not be opened, once. */
export async function notice() {
  return (await call({ op: 'notice' })).notice;
}

/** Ask the browser not to clear this site's storage under pressure. */
export async function keepStorage() {
  try {
    return await navigator.storage?.persist?.();
  } catch {
    return false;
  }
}

// ------------------------------------------------------------------ watch folder

const EXTENSIONS = ['csv', 'txt', 'xlsx', 'xlsm', 'xls', 'ods', 'prn'];
const MAX_DEPTH = 4;
const SETTLE_MS = 5000;
const INTERVAL_MS = 30_000;

/** Status shown on the page, same shape as the desktop program's. */
export const watch = { folder: null, last_scan: null, error: null, files_seen: 0, events: [], needs_permission: false };
let handle = null;
let timer = null;
let seen = new Set();
const listeners = new Set();
export const onWatchEvent = (fn) => listeners.add(fn);

export const folderAccessSupported = () => typeof window.showDirectoryPicker === 'function';

// Small key-value store in the page's own IndexedDB (separate from the worker's).
function kv(mode, fn) {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open('tracker-top-page', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onerror = () => reject(r.error);
    r.onsuccess = () => {
      const db = r.result;
      const tx = db.transaction('kv', mode);
      const req = fn(tx.objectStore('kv'));
      tx.oncomplete = () => { db.close(); resolve(req?.result); };
      tx.onerror = () => { db.close(); reject(tx.error); };
    };
  });
}
const kvGet = (k) => kv('readonly', (s) => s.get(k));
const kvSet = (k, v) => kv('readwrite', (s) => s.put(v, k));
const kvDel = (k) => kv('readwrite', (s) => s.delete(k));

const stamp = () => {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
};

export async function restoreWatch() {
  try {
    handle = (await kvGet('watch-handle')) ?? null;
    seen = new Set((await kvGet('watch-seen')) ?? []);
  } catch {
    handle = null;
  }
  if (!handle) return;
  watch.folder = handle.name;
  const state = await handle.queryPermission?.({ mode: 'read' });
  if (state === 'granted') start();
  else {
    watch.needs_permission = true;
    watch.error = 'The browser needs your permission again to read this folder: click "Resume watching".';
  }
}

export async function chooseFolder() {
  handle = await window.showDirectoryPicker({ id: 'tracker-top-watch', mode: 'read' });
  await kvSet('watch-handle', handle);
  seen = new Set();
  await kvSet('watch-seen', []);
  watch.folder = handle.name;
  watch.events = [];
  start();
}

export async function resumeWatching() {
  if (!handle) return;
  if ((await handle.requestPermission({ mode: 'read' })) === 'granted') start();
}

export async function stopWatching() {
  clearInterval(timer);
  timer = null;
  handle = null;
  await kvDel('watch-handle');
  Object.assign(watch, { folder: null, last_scan: null, error: null, files_seen: 0, events: [], needs_permission: false });
}

function start() {
  watch.needs_permission = false;
  watch.error = null;
  clearInterval(timer);
  scan();
  timer = setInterval(scan, INTERVAL_MS);
}

async function* files(dir, prefix, depth) {
  for await (const entry of dir.values()) {
    if (entry.name.startsWith('.') || entry.name.startsWith('~$')) continue;
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.kind === 'directory' && depth < MAX_DEPTH) yield* files(entry, path, depth + 1);
    else if (entry.kind === 'file' && EXTENSIONS.includes(entry.name.split('.').pop().toLowerCase())) yield [path, entry];
  }
}

let scanning = false;
async function scan() {
  if (!handle || scanning) return;
  scanning = true;
  let imported = false;
  try {
    for await (const [path, entry] of files(handle, '', 0)) {
      const file = await entry.getFile();
      const key = `${path}|${file.size}|${file.lastModified}`;
      if (seen.has(key)) continue;
      if (Date.now() - file.lastModified < SETTLE_MS) continue; // still being written
      const bytes = await file.arrayBuffer();
      seen.add(key);
      const sha = (await call({ op: 'sha', bytes: bytes.slice(0) })).sha;
      const known = await request('GET', `api/file_seen?kind=survey&sha=${sha}`);
      if (JSON.parse(new TextDecoder().decode(known.body)).seen) continue;
      const q = new URLSearchParams({ kind: 'survey', name: file.name, origin: 'watch folder' });
      const r = await request('POST', `api/import?${q}`, bytes, true);
      const res = JSON.parse(new TextDecoder().decode(r.body));
      const rep = res.report;
      const event = rep
        ? {
          at: stamp(),
          file: path,
          accepted: rep.accepted,
          import_id: rep.import_id,
          message: rep.accepted
            ? `${rep.rows_used} piles updated (${rep.rows_rejected} rows skipped), survey date ${rep.survey_date}`
            : (rep.findings.find((f) => f.level === 'error')?.message ?? 'not imported'),
        }
        : { at: stamp(), file: path, accepted: false, import_id: null, message: res.error ?? 'not imported' };
      imported ||= event.accepted;
      watch.events.unshift(event);
      watch.events.length = Math.min(watch.events.length, 50);
    }
    watch.error = null;
  } catch (err) {
    watch.error = `cannot read the folder: ${err.message ?? err}`;
  } finally {
    watch.last_scan = stamp();
    watch.files_seen = seen.size;
    scanning = false;
    try { await kvSet('watch-seen', [...seen]); } catch { /* not critical */ }
    if (imported) listeners.forEach((fn) => fn());
  }
}
