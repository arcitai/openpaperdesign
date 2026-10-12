// Page glue for openpaperdesign-web: loads the active page of the served project, forwards
// DOM input to the upstream editor (wasm) and saves with revision checks (If-Match).
import init, { start } from './openpaperdesign_web.js';

const SAVE_POLL_MS = 300;

// State for people and for automated checks (read-only from outside).
const state = { ready: false, backend: null, pageId: null, revision: null, saves: 0,
  saving: false, conflict: false, error: null };
window.__opd = state;

const $ = (id) => document.getElementById(id);
const status = (s) => { $('status').textContent = s; };

function token() {
  // The URL fragment carries the per-launch token. Keep it for reloads in this tab only and
  // take it out of the address bar.
  const fromHash = new URLSearchParams(location.hash.slice(1)).get('token');
  if (fromHash) {
    sessionStorage.setItem('opd-token', fromHash);
    history.replaceState(null, '', location.pathname + location.search);
  }
  return fromHash || sessionStorage.getItem('opd-token');
}

const TOKEN = token();

async function api(path, opts = {}) {
  const headers = new Headers(opts.headers || {});
  headers.set('Authorization', `Bearer ${TOKEN}`);
  const res = await fetch(path, { ...opts, headers, cache: 'no-store' });
  return res;
}

function isMac() {
  const p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
  return /mac|iphone|ipad/i.test(p);
}

function fail(msg) {
  state.error = msg;
  status(msg);
  console.error(msg);
}

async function main() {
  if (!TOKEN) return fail('Missing token: open the URL printed by openpaperdesign-serve.');
  await init();
  const pres = await api('/api/project');
  if (!pres.ok) return fail(`Could not load the project (${pres.status}).`);
  const project = await pres.json();
  const page = project.pages.find((p) => p.id === project.active_page) || project.pages[0];
  const res = await api(`/api/pages/${encodeURIComponent(page.id)}`);
  if (!res.ok) return fail(`Could not load the page (${res.status}).`);
  state.pageId = page.id;
  state.revision = res.headers.get('ETag');
  const text = await res.text();
  $('title').textContent = `${project.name} — ${page.name}`;
  document.title = `${project.name} — openpaperdesign`;

  const canvas = $('canvas');
  const rect = () => canvas.getBoundingClientRect();
  const seed = crypto.getRandomValues(new Uint32Array(2));
  const forceWebgl = new URLSearchParams(location.search).get('backend') === 'webgl';
  const r0 = rect();
  const ed = await start(canvas, text, page.id, page.name, seed[0], seed[1], isMac(), forceWebgl,
    r0.width, r0.height, devicePixelRatio);
  state.backend = ed.backend();
  window.__opdEditor = ed;

  // ---- input
  const mods = (e) => ed.modifiers(e.shiftKey, e.ctrlKey, e.altKey, e.metaKey);
  const local = (e) => { const r = rect(); return [e.clientX - r.left, e.clientY - r.top]; };
  canvas.addEventListener('pointerdown', (e) => {
    canvas.focus();
    canvas.setPointerCapture(e.pointerId);
    mods(e);
    ed.pointer_down(...local(e), e.button, e.timeStamp);
    e.preventDefault();
  });
  canvas.addEventListener('pointermove', (e) => { mods(e); ed.pointer_move(...local(e), e.timeStamp); });
  const up = (e) => { mods(e); ed.pointer_up(...local(e), e.button, e.timeStamp); };
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);
  canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    mods(e);
    const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? rect().height : 1;
    ed.wheel(e.deltaX * k, e.deltaY * k, ...local(e));
  }, { passive: false });
  canvas.addEventListener('keydown', (e) => {
    mods(e);
    if (ed.key_down(e.key, e.code, e.repeat)) e.preventDefault();
  });
  canvas.addEventListener('keyup', (e) => { mods(e); ed.key_up(e.key, e.code); });
  canvas.addEventListener('blur', () => ed.modifiers(false, false, false, false));
  new ResizeObserver(() => { const r = rect(); ed.resize(r.width, r.height, devicePixelRatio); })
    .observe(canvas);

  // ---- toolbar
  for (const b of document.querySelectorAll('[data-tool]')) {
    b.addEventListener('click', () => { ed.set_tool(b.dataset.tool); canvas.focus(); });
  }
  for (const b of document.querySelectorAll('[data-action]')) {
    b.addEventListener('click', () => { ed[b.dataset.action](); canvas.focus(); });
  }
  const syncToolbar = () => {
    const t = ed.tool();
    for (const b of document.querySelectorAll('[data-tool]')) {
      b.setAttribute('aria-pressed', String(b.dataset.tool === t));
    }
  };

  // ---- saving: one request at a time, always with the revision we last saw.
  const save = async () => {
    if (state.saving || state.conflict) return;
    const snapshot = ed.save_snapshot();
    if (snapshot === undefined) return;
    state.saving = true;
    status('Saving…');
    try {
      const res = await api(`/api/pages/${encodeURIComponent(state.pageId)}`, {
        method: 'PUT', body: snapshot,
        headers: { 'If-Match': state.revision, 'Content-Type': 'application/json' },
      });
      if (res.ok) {
        state.revision = res.headers.get('ETag');
        state.saves += 1;
        ed.save_succeeded();
        status('Saved');
      } else if (res.status === 409) {
        ed.save_failed();
        state.conflict = true;
        status('Not saved: conflict');
        const c = $('conflict');
        c.textContent = 'This page was changed on disk by something else since it was loaded. '
          + 'Nothing was overwritten, and edits made here since then are not saved. '
          + 'Reload to continue from the version on disk.';
        c.classList.add('show');
      } else {
        ed.save_failed();
        status(`Save failed (${res.status}); retrying`);
      }
    } catch (err) {
      ed.save_failed();
      status('Save failed (server unreachable); retrying');
    } finally {
      state.saving = false;
    }
  };
  setInterval(save, SAVE_POLL_MS);
  window.addEventListener('beforeunload', (e) => { if (ed.dirty()) e.preventDefault(); });

  const frame = (t) => {
    ed.frame(t);
    syncToolbar();
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  state.ready = true;
  status(`Ready · ${state.backend}`);
  $('footer').append(` · Renderer: ${state.backend}`);
  canvas.focus();
}

main().catch((e) => fail(`Failed to start: ${e && e.message ? e.message : e}`));
