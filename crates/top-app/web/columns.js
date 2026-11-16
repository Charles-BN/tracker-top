// Tables whose columns the user chooses. Each table has an id and a list of
// columns { key, label, cls, tip, cell(row), hidden (off by default), fixed
// (cannot be hidden), when (shown only when true) }. The choice is kept in
// this browser (localStorage) per table.

const KEY = (id) => `tracker-top.columns.${id}`;

function load(id) {
  try {
    const v = JSON.parse(localStorage.getItem(KEY(id)));
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

function save(id, v) {
  try {
    localStorage.setItem(KEY(id), JSON.stringify(v));
  } catch { /* private window: the choice lasts until the page is closed */ }
}

const memory = {};
const prefs = (id) => (memory[id] ??= load(id));

/** Columns shown in table `id`: chosen ones (or the defaults), available ones only. */
export function visibleColumns(id, cols) {
  const p = prefs(id);
  return cols.filter((c) => c.when !== false && (c.fixed || (p[c.key] ?? !c.hidden)));
}

/** Header cells. `sort` = { key, dir } marks the sorted column. */
export function headCells(cols, sort) {
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[ch]);
  return cols.map((c) => `<th class="${c.cls || ''}${c.sort ? ' sortable' : ''}"${c.sort ? ` data-sort="${c.key}"` : ''}${c.tip ? ` title="${esc(c.tip)}"` : ''}>${c.label}${sort && sort.key === c.key ? ` <span class="arrow">${sort.dir > 0 ? '▲' : '▼'}</span>` : ''}</th>`).join('');
}

/** A "Columns" button that opens the chooser for table `id`; `redraw` runs after each change. */
export function columnsButton(id, cols, redraw) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'btn small ghost cols-btn';
  btn.innerHTML = '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M2 3h12M2 8h12M2 13h12" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><circle cx="5" cy="3" r="1.6" fill="var(--surface)" stroke="currentColor" stroke-width="1.4"/><circle cx="11" cy="8" r="1.6" fill="var(--surface)" stroke="currentColor" stroke-width="1.4"/><circle cx="7" cy="13" r="1.6" fill="var(--surface)" stroke="currentColor" stroke-width="1.4"/></svg> Columns';
  btn.setAttribute('aria-haspopup', 'true');
  btn.setAttribute('aria-expanded', 'false');
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    const open = document.querySelector('.cols-pop');
    open?.remove();
    if (open?.dataset.for === id) { btn.setAttribute('aria-expanded', 'false'); return; }
    openChooser(btn, id, typeof cols === 'function' ? cols() : cols, redraw);
  });
  return btn;
}

function openChooser(btn, id, cols, redraw) {
  const p = prefs(id);
  const pop = document.createElement('div');
  pop.className = 'cols-pop';
  pop.dataset.for = id;
  pop.setAttribute('role', 'dialog');
  pop.setAttribute('aria-label', 'Columns to show');
  const list = cols.filter((c) => c.when !== false && c.label);
  const groups = [...new Set(list.map((c) => c.group || ''))];
  pop.innerHTML = `<div class="cols-pop-head"><b>Columns</b><span><button type="button" class="linkish" data-all>All</button> · <button type="button" class="linkish" data-reset>Default</button></span></div>
    <div class="cols-pop-list">${groups.map((g) => `${g ? `<div class="cols-pop-group">${g}</div>` : ''}${list.filter((c) => (c.group || '') === g).map((c) => `<label class="check"><input type="checkbox" value="${c.key}"${c.fixed ? ' checked disabled' : (p[c.key] ?? !c.hidden) ? ' checked' : ''}> ${c.label}</label>`).join('')}`).join('')}</div>`;
  document.body.appendChild(pop);
  const r = btn.getBoundingClientRect();
  const w = pop.offsetWidth;
  pop.style.top = `${r.bottom + window.scrollY + 6}px`;
  pop.style.left = `${Math.max(8, Math.min(window.innerWidth - w - 8, r.right - w)) + window.scrollX}px`;
  btn.setAttribute('aria-expanded', 'true');
  const apply = () => { save(id, p); redraw(); };
  pop.addEventListener('change', (e) => {
    p[e.target.value] = e.target.checked;
    apply();
  });
  pop.querySelector('[data-all]').addEventListener('click', () => {
    for (const c of list) if (!c.fixed) p[c.key] = true;
    pop.querySelectorAll('input:not(:disabled)').forEach((i) => { i.checked = true; });
    apply();
  });
  pop.querySelector('[data-reset]').addEventListener('click', () => {
    for (const k of Object.keys(p)) delete p[k];
    pop.querySelectorAll('input:not(:disabled)').forEach((i) => { i.checked = !list.find((c) => c.key === i.value).hidden; });
    apply();
  });
  const close = (e) => {
    if (e.type === 'keydown' ? e.key !== 'Escape' : pop.contains(e.target) || e.target === btn) return;
    pop.remove();
    btn.setAttribute('aria-expanded', 'false');
    document.removeEventListener('click', close);
    document.removeEventListener('keydown', close);
  };
  setTimeout(() => {
    document.addEventListener('click', close);
    document.addEventListener('keydown', close);
  });
}
