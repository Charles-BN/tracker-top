// Shared database: keeps this computer's database in step with one copy,
// tracker-top.db, kept in a shared folder, and writes the installers' files
// there. No server is involved: every computer works on its own copy and
// exchanges the whole file. Two ways to reach the shared folder:
//
// * a folder on this computer that a sync program keeps in step with the
//   cloud: Google Drive for desktop (G:\My Drive\...), OneDrive, a network
//   drive. Uses the browser's folder access (Edge, Chrome).
// * Google Drive itself, online, signing in with Google (no install). Needs a
//   Google Cloud "OAuth client ID" for this web address, set up once.
//
// The program decides what to do with a copy (take it, merge it, or report a
// conflict: see crates/top-db/src/sync.rs); this module moves the files.

const DB_FILE = 'tracker-top.db';
const DRIVE = 'https://www.googleapis.com/drive/v3';
const DRIVE_UPLOAD = 'https://www.googleapis.com/upload/drive/v3';
const DRIVE_SCOPE = 'https://www.googleapis.com/auth/drive';
const CHECK_EVERY_MS = { folder: 30_000, drive: 60_000 };

// ------------------------------------------------------------------ settings of this computer

function kv(mode, fn) {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open('tracker-top-sync', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onerror = () => reject(r.error);
    r.onsuccess = () => {
      const db = r.result;
      try {
        const tx = db.transaction('kv', mode);
        const req = fn(tx.objectStore('kv'));
        tx.oncomplete = () => { db.close(); resolve(req?.result); };
        tx.onerror = () => { db.close(); reject(tx.error); };
      } catch (e) {
        db.close();
        reject(e); // e.g. a value the browser cannot store
      }
    };
  });
}
const kvGet = (k) => kv('readonly', (s) => s.get(k)).catch(() => undefined);
const kvSet = (k, v) => kv('readwrite', (s) => s.put(v, k)).catch(() => undefined);

// ------------------------------------------------------------------ folder on this computer

export const folderAccessSupported = () => typeof window.showDirectoryPicker === 'function';

class FolderTarget {
  constructor(handle) {
    this.kind = 'folder';
    this.handle = handle;
  }

  get label() { return `folder “${this.handle.name}” on this computer`; }

  async permission(ask = false) {
    const opts = { mode: 'readwrite' };
    let p = await this.handle.queryPermission?.(opts) ?? 'granted';
    if (p !== 'granted' && ask) p = await this.handle.requestPermission(opts);
    return p;
  }

  async dir(parts, create) {
    let d = this.handle;
    for (const part of parts) d = await d.getDirectoryHandle(part, { create });
    return d;
  }

  async fileHandle(path, create) {
    const parts = path.split('/').filter(Boolean);
    const name = parts.pop();
    try {
      return await (await this.dir(parts, create)).getFileHandle(name, { create });
    } catch (e) {
      if (!create && (e.name === 'NotFoundError' || e.name === 'TypeMismatchError')) return null;
      throw e;
    }
  }

  async stat(path) {
    const fh = await this.fileHandle(path, false);
    if (!fh) return null;
    const f = await fh.getFile();
    return { stamp: `${f.lastModified}-${f.size}` };
  }

  async read(path) {
    const fh = await this.fileHandle(path, false);
    return new Uint8Array(await (await fh.getFile()).arrayBuffer());
  }

  async write(path, bytes) {
    const fh = await this.fileHandle(path, true);
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close(); // the browser swaps the new content in at once
    return this.stat(path);
  }
}

// ------------------------------------------------------------------ Google Drive online

/** Folder ID from a Drive folder link (or the ID itself). */
export function driveFolderId(link) {
  const s = String(link || '').trim();
  const m = s.match(/\/folders\/([\w-]{10,})/) || s.match(/[?&]id=([\w-]{10,})/) || s.match(/^([\w-]{10,})$/);
  return m ? m[1] : null;
}

let gisLoading = null;
function loadGoogleSignIn() {
  if (window.google?.accounts?.oauth2) return Promise.resolve();
  gisLoading ??= new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = 'https://accounts.google.com/gsi/client';
    s.async = true;
    s.onload = () => resolve();
    s.onerror = () => { gisLoading = null; reject(new Error('cannot load Google sign-in (no internet connection?)')); };
    document.head.appendChild(s);
  });
  return gisLoading;
}

class DriveTarget {
  constructor({ clientId, folderId, folderName }) {
    this.kind = 'drive';
    this.clientId = clientId;
    this.folderId = folderId;
    this.folderName = folderName || 'shared folder';
    this.ids = new Map(); // path -> file or folder id
    this.token = null;
    try {
      const t = JSON.parse(sessionStorage.getItem('tracker-top-drive-token') || 'null');
      if (t && t.clientId === clientId && t.expires > Date.now() + 60_000) this.token = t;
    } catch { /* not available */ }
  }

  get label() { return `Google Drive folder “${this.folderName}”`; }

  /** 'granted' while signed in, else 'prompt' (a click on Connect signs in). */
  async permission(ask = false) {
    if (this.token && this.token.expires > Date.now() + 60_000) return 'granted';
    if (!ask) return 'prompt';
    await this.signIn();
    return 'granted';
  }

  async signIn() {
    await loadGoogleSignIn();
    const token = await new Promise((resolve, reject) => {
      const client = window.google.accounts.oauth2.initTokenClient({
        client_id: this.clientId,
        scope: DRIVE_SCOPE,
        callback: (r) => (r.error ? reject(new Error(r.error_description || r.error)) : resolve(r)),
        error_callback: (e) => reject(new Error(e?.message || e?.type || 'Google sign-in was closed')),
      });
      client.requestAccessToken({ prompt: '' });
    });
    this.token = { access: token.access_token, expires: Date.now() + (token.expires_in || 3600) * 1000, clientId: this.clientId };
    try { sessionStorage.setItem('tracker-top-drive-token', JSON.stringify(this.token)); } catch { /* not kept */ }
  }

  async call(url, opts = {}) {
    if (!this.token || this.token.expires < Date.now() + 30_000) {
      const e = new Error('Google Drive: sign in again (click Connect)');
      e.signIn = true;
      throw e;
    }
    const res = await fetch(url, { ...opts, headers: { Authorization: `Bearer ${this.token.access}`, ...(opts.headers || {}) } });
    if (res.status === 401) {
      this.token = null;
      const e = new Error('Google Drive: sign in again (click Connect)');
      e.signIn = true;
      throw e;
    }
    if (!res.ok) {
      let msg = res.statusText;
      try { msg = (await res.json()).error?.message || msg; } catch { /* not JSON */ }
      throw new Error(`Google Drive: ${msg} (${res.status})`);
    }
    return res;
  }

  async find(name, parent, folder = false) {
    const q = [`name = '${name.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'`, `'${parent}' in parents`, 'trashed = false',
      folder ? "mimeType = 'application/vnd.google-apps.folder'" : "mimeType != 'application/vnd.google-apps.folder'"].join(' and ');
    const url = `${DRIVE}/files?q=${encodeURIComponent(q)}&fields=files(id,md5Checksum,version,modifiedTime)&orderBy=modifiedTime desc&pageSize=10&supportsAllDrives=true&includeItemsFromAllDrives=true`;
    return (await (await this.call(url)).json()).files?.[0] ?? null;
  }

  async folder(parts, create) {
    let parent = this.folderId;
    let path = '';
    for (const part of parts) {
      path += `${part}/`;
      let id = this.ids.get(path);
      if (!id) {
        const f = await this.find(part, parent, true);
        if (f) id = f.id;
        else if (!create) return null;
        else {
          const res = await this.call(`${DRIVE}/files?supportsAllDrives=true&fields=id`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: part, parents: [parent], mimeType: 'application/vnd.google-apps.folder' }),
          });
          id = (await res.json()).id;
        }
        this.ids.set(path, id);
      }
      parent = id;
    }
    return parent;
  }

  async file(path, create = false) {
    const parts = path.split('/').filter(Boolean);
    const name = parts.pop();
    const parent = await this.folder(parts, create);
    return parent ? { parent, name, meta: await this.find(name, parent) } : null;
  }

  async stat(path) {
    const f = await this.file(path);
    if (!f?.meta) return null;
    this.ids.set(path, f.meta.id);
    return { stamp: f.meta.md5Checksum || `${f.meta.version}`, id: f.meta.id };
  }

  async read(path) {
    const id = this.ids.get(path) ?? (await this.stat(path))?.id;
    const res = await this.call(`${DRIVE}/files/${id}?alt=media&supportsAllDrives=true`);
    return new Uint8Array(await res.arrayBuffer());
  }

  async write(path, bytes) {
    const f = await this.file(path, true);
    const existing = f.meta?.id;
    // Resumable upload: works for any size (simple uploads stop at 5 MB).
    const start = await this.call(
      existing
        ? `${DRIVE_UPLOAD}/files/${existing}?uploadType=resumable&supportsAllDrives=true&fields=id,md5Checksum,version`
        : `${DRIVE_UPLOAD}/files?uploadType=resumable&supportsAllDrives=true&fields=id,md5Checksum,version`,
      {
        method: existing ? 'PATCH' : 'POST',
        headers: {
          'Content-Type': 'application/json; charset=UTF-8',
          'X-Upload-Content-Type': 'application/octet-stream',
          'X-Upload-Content-Length': String(bytes.byteLength),
        },
        body: JSON.stringify(existing ? {} : { name: f.name, parents: [f.parent] }),
      },
    );
    const session = start.headers.get('Location');
    if (!session) throw new Error('Google Drive did not accept the upload');
    const res = await this.call(session, { method: 'PUT', body: bytes });
    const meta = await res.json();
    this.ids.set(path, meta.id);
    return { stamp: meta.md5Checksum || `${meta.version}`, id: meta.id };
  }
}

// ------------------------------------------------------------------ the sync loop

/**
 * @param {object} io  { json(method, url, body?), bytes(method, url, body?) -> {bytes, filename},
 *                       changed(report), exported(text), state() }
 */
export function createSync(io) {
  const state = {
    target: null,     // FolderTarget | DriveTarget
    mode: null,       // 'folder' | 'drive' | null
    name: '',         // this computer's name, shown as "saved by"
    phase: 'off',     // off | needs-permission | idle | working | error | conflict
    message: '',
    lastCheck: null,
    lastReport: null,
    status: null,     // GET api/sync/status
    drive: { clientId: '', folderId: '', folderName: '' },
  };
  let lastStamp = null;
  let timer = null;
  let running = null;
  let again = false;
  const listeners = new Set();
  const notify = () => listeners.forEach((fn) => { try { fn(state); } catch { /* ignore */ } });

  async function remember() {
    await kvSet('config', {
      mode: state.mode,
      name: state.name,
      handle: state.mode === 'folder' ? state.target?.handle : undefined,
      drive: state.drive,
      lastStamp,
    });
  }

  async function load() {
    const c = (await kvGet('config')) || {};
    state.name = c.name || '';
    state.drive = { clientId: '', folderId: '', folderName: '', ...(c.drive || {}) };
    lastStamp = c.lastStamp ?? null;
    if (c.mode === 'folder' && c.handle) setTarget(new FolderTarget(c.handle), 'folder');
    else if (c.mode === 'drive' && c.drive?.clientId && c.drive?.folderId) setTarget(new DriveTarget(c.drive), 'drive');
    if (state.target) {
      const p = await state.target.permission(false).catch(() => 'prompt');
      if (p === 'granted') start();
      else { state.phase = 'needs-permission'; notify(); }
    }
  }

  function setTarget(target, mode) {
    state.target = target;
    state.mode = mode;
  }

  function start() {
    state.phase = 'idle';
    state.message = '';
    clearInterval(timer);
    timer = setInterval(() => sync('timer'), CHECK_EVERY_MS[state.mode] || 60_000);
    notify();
    return sync('start');
  }

  function stop() {
    clearInterval(timer);
    timer = null;
  }

  async function connectFolder(name) {
    const handle = await window.showDirectoryPicker({ id: 'tracker-top-shared', mode: 'readwrite' });
    state.name = name.trim();
    setTarget(new FolderTarget(handle), 'folder');
    lastStamp = null;
    if ((await state.target.permission(true)) !== 'granted') throw new Error('the browser was not allowed to write in this folder');
    await remember();
    return start();
  }

  async function connectDrive(name, clientId, link, folderName) {
    const folderId = driveFolderId(link);
    if (!clientId.trim()) throw new Error('enter the Google OAuth client ID (see the set-up steps)');
    if (!folderId) throw new Error('paste the link of the Google Drive folder (it contains /folders/...)');
    state.name = name.trim();
    state.drive = { clientId: clientId.trim(), folderId, folderName: folderName.trim() || 'shared folder' };
    setTarget(new DriveTarget(state.drive), 'drive');
    lastStamp = null;
    await state.target.permission(true);
    await remember();
    return start();
  }

  /** After a reload: the browser asks again before the folder (or Drive) can be used. */
  async function reconnect() {
    if (!state.target) return;
    if ((await state.target.permission(true)) === 'granted') return start();
    state.phase = 'needs-permission';
    notify();
  }

  async function disconnect() {
    stop();
    state.target = null;
    state.mode = null;
    state.phase = 'off';
    state.message = '';
    lastStamp = null;
    await remember();
    notify();
  }

  async function rename(name) {
    state.name = name.trim();
    await remember();
    notify();
  }

  const by = () => encodeURIComponent(state.name || 'unnamed computer');

  /** One exchange with the shared folder. Runs again if asked meanwhile. */
  function sync(reason = 'manual') {
    if (!state.target || state.phase === 'needs-permission' || state.phase === 'conflict') return Promise.resolve();
    if (running) { again = true; return running; }
    const run = async () => {
      state.phase = 'working';
      notify();
      try {
        await exchange();
        await sendInstallerFiles();
        state.phase = 'idle';
        state.message = '';
      } catch (e) {
        if (e.signIn) { state.phase = 'needs-permission'; state.message = e.message; } else if (state.phase !== 'conflict') { state.phase = 'error'; state.message = e.message || String(e); }
      } finally {
        state.lastCheck = new Date();
        try { state.status = await io.json('GET', 'api/sync/status'); } catch { /* keep the last one */ }
        notify();
      }
    };
    const locked = navigator.locks?.request
      ? () => navigator.locks.request('tracker-top-sync', run)
      : run;
    running = locked().finally(() => {
      running = null;
      if (again) { again = false; setTimeout(() => sync('again'), 500); }
    });
    return running;
  }

  async function exchange() {
    const t = state.target;
    for (let attempt = 0; attempt < 4; attempt++) {
      const st = await t.stat(DB_FILE);
      if (st && st.stamp !== lastStamp) {
        const bytes = await t.read(DB_FILE);
        const rep = await io.json('POST', `api/sync/incoming?by=${by()}`, bytes, true);
        lastStamp = st.stamp;
        await remember();
        state.lastReport = rep;
        if (rep.action === 'conflict') {
          state.phase = 'conflict';
          state.message = rep.message;
          throw new Error(rep.message);
        }
        if (rep.action !== 'none') io.changed(rep);
      }
      const status = await io.json('GET', 'api/sync/status');
      state.status = status;
      if (!status.dirty) return;
      if (!st && status.imports === 0) return; // nothing worth creating the shared file for
      const snap = await io.bytes('POST', `api/sync/snapshot?by=${by()}`);
      const now = await t.stat(DB_FILE);
      if (now && now.stamp !== lastStamp) continue; // saved elsewhere meanwhile: take it in first
      const written = await t.write(DB_FILE, snap.bytes);
      lastStamp = written?.stamp ?? null;
      await remember();
      await io.json('POST', 'api/sync/saved');
      return;
    }
    throw new Error('the shared database keeps changing; trying again shortly');
  }

  async function sendInstallerFiles() {
    const { settings } = await io.json('GET', 'api/summary');
    if (!settings.auto_export) return;
    const blocks = (await io.json('GET', 'api/blocks')).filter((b) => b.needs_export);
    let sent = 0;
    for (const b of blocks) {
      const file = await io.bytes('GET', `api/export/block.xlsx?block=${encodeURIComponent(b.block)}`);
      const name = file.filename || `Block ${b.block} - pile heights rev ${b.next_revision}.xlsx`;
      const folder = settings.auto_export_folder.split('/').map((s) => s.trim()).filter(Boolean).join('/');
      await state.target.write(`${folder}/${name}`, file.bytes);
      const q = new URLSearchParams({ block: b.block, sha: b.content_sha, file: name, by: state.name || 'unnamed computer' });
      await io.json('POST', `api/export/block_done?${q}`);
      io.exported(`Block ${b.block || '(none)'} complete: “${name}” written to the installers' folder (${folder}).`);
      sent++;
    }
    if (sent) again = true; // the export log changed: share it
  }

  /** Conflict: keep this computer's database as a file in the folder, then use the shared one. */
  async function useShared() {
    const t = state.target;
    const mine = await io.bytes('POST', `api/sync/snapshot?by=${by()}`);
    const stampName = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-');
    await t.write(`tracker-top (copy from ${state.name || 'this computer'} ${stampName}).db`, mine.bytes);
    const st = await t.stat(DB_FILE);
    await io.json('POST', 'api/sync/adopt', await t.read(DB_FILE), true);
    lastStamp = st?.stamp ?? null;
    await remember();
    io.changed({ action: 'adopted', message: 'Now using the shared database. This computer\'s copy was kept in the folder.' });
    return start();
  }

  /** Conflict: this computer's database replaces the shared one (after keeping it aside). */
  async function replaceShared() {
    const t = state.target;
    const theirs = await t.read(DB_FILE);
    const stampName = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-');
    await t.write(`tracker-top (replaced ${stampName}).db`, theirs);
    const snap = await io.bytes('POST', `api/sync/snapshot?by=${by()}`);
    const written = await t.write(DB_FILE, snap.bytes);
    lastStamp = written?.stamp ?? null;
    await io.json('POST', 'api/sync/saved');
    await remember();
    return start();
  }

  window.addEventListener('focus', () => { if (state.phase === 'idle') sync('focus'); });

  return {
    state,
    load,
    sync,
    changed: () => { if (state.target) setTimeout(() => sync('change'), 800); },
    connectFolder,
    connectDrive,
    reconnect,
    disconnect,
    rename,
    useShared,
    replaceShared,
    onUpdate: (fn) => listeners.add(fn),
  };
}
