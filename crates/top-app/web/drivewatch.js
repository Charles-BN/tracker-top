// Watch a Google Drive folder online: while this page is open, every new
// survey file in the folder (or its subfolders) is downloaded and imported,
// like the watch folder on this computer, but with nothing installed: the
// page signs in with Google (the same sign-in as the shared database on
// Google Drive online) and reads the folder through the Drive API.
//
// CSV / TXT / Excel files are read as they are; Google Sheets are exported
// as Excel. A file already imported (same content) is skipped; a file whose
// content changes is imported again.

import { DriveTarget, driveFolderId } from './sync.js';

const DRIVE = 'https://www.googleapis.com/drive/v3';
const EXTENSIONS = ['csv', 'txt', 'xlsx', 'xlsm', 'xls', 'ods', 'prn'];
const SHEET = 'application/vnd.google-apps.spreadsheet';
const FOLDER = 'application/vnd.google-apps.folder';
const XLSX = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
const MAX_DEPTH = 4;
const INTERVAL_MS = 60_000;
const KEY = 'tracker-top.drive-watch';

/** Status shown on the page (same shape as the other watch folders). */
export const driveWatch = { folder: null, link: null, last_scan: null, error: null, files_seen: 0, events: [], needs_signin: false };

let target = null;
let timer = null;
let seen = new Set();
let io = null; // { api(method, url, body, raw), imported() }

const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* not kept */ } };
const stamp = () => {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
};
const ext = (name) => name.split('.').pop().toLowerCase();

/** Restore the watch of this browser, if any. `clientId`: the Google OAuth client ID. */
export function initDriveWatch(handlers, clientId) {
  io = handlers;
  const cfg = load(KEY, null);
  if (!cfg?.folderId || !clientId) return;
  seen = new Set(load(`${KEY}.seen`, []));
  target = new DriveTarget({ clientId, folderId: cfg.folderId, folderName: cfg.folderName });
  Object.assign(driveWatch, { folder: cfg.folderName, link: cfg.link });
  start();
}

/** Start watching the Drive folder of `link` (a folder link or ID); asks Google to sign in. */
export async function watchDriveFolder(link, clientId) {
  const folderId = driveFolderId(link);
  if (!folderId) throw new Error('Paste the link of a Google Drive folder (…/drive/folders/…).');
  if (!clientId) throw new Error('Google Drive online needs the OAuth client ID (Settings > Database > Shared database).');
  const t = new DriveTarget({ clientId, folderId, folderName: 'Drive folder' });
  await t.permission(true);
  const meta = await (await t.call(`${DRIVE}/files/${folderId}?fields=name,mimeType&supportsAllDrives=true`)).json();
  if (meta.mimeType !== FOLDER) throw new Error(`“${meta.name}” is not a folder.`);
  t.folderName = meta.name;
  target = t;
  seen = new Set();
  save(KEY, { folderId, folderName: meta.name, link });
  save(`${KEY}.seen`, []);
  Object.assign(driveWatch, { folder: meta.name, link, events: [], error: null, files_seen: 0 });
  start();
}

/** Sign in again (Google asks after an hour, and after a reload). */
export async function resumeDriveWatch() {
  if (!target) return;
  await target.permission(true);
  start();
}

export function stopDriveWatch() {
  clearInterval(timer);
  timer = null;
  target = null;
  try { localStorage.removeItem(KEY); localStorage.removeItem(`${KEY}.seen`); } catch { /* nothing kept */ }
  Object.assign(driveWatch, { folder: null, link: null, last_scan: null, error: null, files_seen: 0, events: [], needs_signin: false });
}

function start() {
  clearInterval(timer);
  scan();
  timer = setInterval(scan, INTERVAL_MS);
}

/** Every file in the folder and its subfolders: [path, file]. */
async function listAll(folderId, prefix, depth, out) {
  let pageToken = '';
  do {
    const q = encodeURIComponent(`'${folderId}' in parents and trashed = false`);
    const url = `${DRIVE}/files?q=${q}&fields=nextPageToken,files(id,name,mimeType,md5Checksum,modifiedTime,size)&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true${pageToken ? `&pageToken=${pageToken}` : ''}`;
    const res = await (await target.call(url)).json();
    for (const f of res.files || []) {
      if (f.name.startsWith('.') || f.name.startsWith('~$')) continue;
      const path = prefix ? `${prefix}/${f.name}` : f.name;
      if (f.mimeType === FOLDER) {
        if (depth < MAX_DEPTH) await listAll(f.id, path, depth + 1, out);
      } else if (f.mimeType === SHEET || EXTENSIONS.includes(ext(f.name))) {
        out.push([path, f]);
      }
    }
    pageToken = res.nextPageToken || '';
  } while (pageToken);
  return out;
}

async function sha256(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return [...d].map((b) => b.toString(16).padStart(2, '0')).join('');
}

let scanning = false;
async function scan() {
  if (!target || scanning) return;
  if ((await target.permission(false)) !== 'granted') {
    driveWatch.needs_signin = true;
    driveWatch.error = 'Sign in to Google Drive again to keep watching (Google asks after an hour, and after a reload).';
    return;
  }
  scanning = true;
  let imported = false;
  try {
    const files = await listAll(target.folderId, '', 0, []);
    for (const [path, f] of files) {
      const key = `${f.id}|${f.md5Checksum || f.modifiedTime}`;
      if (seen.has(key)) continue;
      const sheet = f.mimeType === SHEET;
      const url = sheet
        ? `${DRIVE}/files/${f.id}/export?mimeType=${encodeURIComponent(XLSX)}`
        : `${DRIVE}/files/${f.id}?alt=media&supportsAllDrives=true`;
      const bytes = await (await target.call(url)).arrayBuffer();
      seen.add(key);
      const sha = await sha256(bytes);
      if ((await io.api('GET', `api/file_seen?kind=survey&sha=${sha}`)).seen) continue;
      const name = sheet ? `${f.name}.xlsx` : f.name;
      const q = new URLSearchParams({ kind: 'survey', name, origin: 'Google Drive watch' });
      let event;
      try {
        const res = await io.api('POST', `api/import?${q}`, bytes, true);
        const rep = res.report;
        event = {
          at: stamp(),
          file: path,
          accepted: rep.accepted,
          import_id: rep.import_id,
          message: rep.accepted
            ? `${rep.rows_used} piles updated (${rep.rows_rejected} rows skipped), survey date ${rep.survey_date}`
            : (rep.findings.find((x) => x.level === 'error')?.message ?? 'not imported'),
        };
      } catch (err) {
        event = { at: stamp(), file: path, accepted: false, import_id: null, message: err.message || 'not imported' };
      }
      imported ||= event.accepted;
      driveWatch.events.unshift(event);
      driveWatch.events.length = Math.min(driveWatch.events.length, 50);
    }
    driveWatch.error = null;
    driveWatch.needs_signin = false;
  } catch (err) {
    if (err.signIn) {
      driveWatch.needs_signin = true;
      driveWatch.error = 'Sign in to Google Drive again to keep watching.';
    } else {
      driveWatch.error = `cannot read the Drive folder: ${err.message ?? err}`;
    }
  } finally {
    driveWatch.last_scan = stamp();
    driveWatch.files_seen = seen.size;
    scanning = false;
    save(`${KEY}.seen`, [...seen]);
    if (imported) io.imported();
  }
}
