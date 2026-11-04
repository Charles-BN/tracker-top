// Background worker of the browser version: holds the tracker-top engine
// (WebAssembly) and its database, so long calculations never freeze the page.
// The database is kept in this browser's IndexedDB and saved after every change.

import init, { Engine } from './pkg/top_web.js';

const DB_NAME = 'tracker-top';
const STORE = 'files';

function openIdb() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB_NAME, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE);
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function idb(mode, fn) {
  const db = await openIdb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, mode);
    const req = fn(tx.objectStore(STORE));
    tx.oncomplete = () => { db.close(); resolve(req?.result); };
    tx.onerror = () => { db.close(); reject(tx.error); };
    tx.onabort = () => { db.close(); reject(tx.error); };
  });
}

const load = (key) => idb('readonly', (s) => s.get(key));
const store = (key, value) => idb('readwrite', (s) => s.put(value, key));

let engine = null;
let notice = null;

async function save() {
  await store('database', engine.save());
}

const ready = (async () => {
  await init();
  const saved = await load('database');
  try {
    engine = new Engine(saved ?? undefined);
  } catch (err) {
    // Keep the unreadable copy aside rather than losing it, and start empty.
    await store(`unreadable-${Date.now()}`, saved);
    notice = `The stored database could not be opened (${err.message ?? err}); a copy was kept aside and an empty database started.`;
    engine = new Engine(undefined);
  }
  if (engine.refresh()) await save();
})();

function reply(id, msg, transfer = []) {
  self.postMessage({ id, ...msg }, transfer);
}

self.onmessage = async (event) => {
  const { id, op } = event.data;
  try {
    await ready;
    if (op === 'request') {
      const { method, path, query, body, watch } = event.data;
      const r = engine.request(method, path, query, body ?? new Uint8Array(), watch ?? 'null');
      const out = { status: r.status, contentType: r.content_type, filename: r.filename ?? null, body: r.take_body() };
      const changed = r.changed;
      r.free();
      if (changed) await save();
      reply(id, out, [out.body.buffer]);
    } else if (op === 'restore') {
      const next = new Engine(new Uint8Array(event.data.bytes));
      next.refresh();
      engine.free();
      engine = next;
      await save();
      reply(id, { status: 200 });
    } else if (op === 'reset') {
      engine.free();
      engine = new Engine(undefined);
      await save();
      reply(id, { status: 200 });
    } else if (op === 'sha') {
      reply(id, { status: 200, sha: Engine.sha256(new Uint8Array(event.data.bytes)) });
    } else if (op === 'notice') {
      reply(id, { status: 200, notice });
      notice = null;
    } else {
      reply(id, { status: 400, error: `unknown operation ${op}` });
    }
  } catch (err) {
    reply(id, { status: 500, error: String(err?.message ?? err) });
  }
};
