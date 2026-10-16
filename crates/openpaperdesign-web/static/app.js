// Page glue for openpaperdesign-web: loads the active page of the served project, forwards
// DOM input to the upstream editor (wasm) and saves with revision checks (If-Match). The
// editor lifecycle (gating, fallback, saving) is in session.js.
import init, { start } from './openpaperdesign_web.js';
import { Session } from './session.js';
import { discardStep, frameStep, saveStep } from './page.js';

const SAVE_POLL_MS = 300;
// How often to look for changes made elsewhere (an agent over MCP, another tab).
const REVISION_POLL_MS = 1000;

// State for people and for automated checks (read-only from outside).
const state = { pageId: null };
window.__opd = state;

const $ = (id) => document.getElementById(id);
// Long text is cut with an ellipsis; the full text is in the tooltip.
const setText = (el, s) => { el.textContent = s; el.title = s; };
const status = (s, error = false) => { setText($('status'), s); $('status').classList.toggle('error', error); };
const renderer = (s, error = false) => { setText($('renderer'), s); $('renderer').classList.toggle('error', error); };

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
  return fetch(path, { ...opts, headers, cache: 'no-store' });
}

function isMac() {
  const p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
  return /mac|iphone|ipad/i.test(p);
}

function fail(msg) {
  state.ready = false;
  state.error = msg;
  status(msg, true);
  renderer(`Not running: ${msg}`, true);
  console.error(msg);
}

async function main() {
  if (!TOKEN) return fail('Missing token: open the URL printed by openpaperdesign-serve.');
  await init();
  const pres = await api('/api/project');
  if (!pres.ok) return fail(`Could not load the project (${pres.status}).`);
  const project = await pres.json();
  const meta = project.pages.find((p) => p.id === project.active_page) || project.pages[0];
  const pageUrl = `/api/pages/${encodeURIComponent(meta.id)}`;
  const fetchPage = async () => {
    const res = await api(pageUrl);
    if (!res.ok) throw new Error(`Could not load the page (${res.status}).`);
    return { revision: res.headers.get('ETag'), text: await res.text() };
  };
  state.pageId = meta.id;
  $('title').textContent = `${project.name} — ${meta.name}`;
  document.title = `${project.name} — openpaperdesign`;

  const seed = crypto.getRandomValues(new Uint32Array(2));
  const forceWebgl = new URLSearchParams(location.search).get('backend') === 'webgl';
  const rectOf = (c) => c.getBoundingClientRect();

  const session = new Session({
    boot: (canvas, page, webgl) => {
      const r = rectOf(canvas);
      return start(canvas, page.text, meta.id, meta.name, seed[0], seed[1], isMac(), webgl,
        r.width, r.height, devicePixelRatio);
    },
    fetchPage,
    replaceCanvas: (old) => {
      const fresh = old.cloneNode(false);
      old.replaceWith(fresh);
      wire(fresh);
      return fresh;
    },
    observeResize: (canvas, onResize) => {
      const ro = new ResizeObserver(onResize);
      ro.observe(canvas);
      return ro;
    },
    resize: (ed, canvas) => {
      const r = rectOf(canvas);
      ed.resize(r.width, r.height, devicePixelRatio);
    },
    put: async (snapshot, revision) => {
      const res = await api(pageUrl, {
        method: 'PUT', body: snapshot,
        headers: { 'If-Match': revision, 'Content-Type': 'application/json' },
      });
      return { ok: res.ok, status: res.status, revision: res.headers.get('ETag') };
    },
    revision: async () => {
      const res = await api(`${pageUrl}/revision`);
      if (!res.ok) throw new Error(`revision ${res.status}`);
      return res.headers.get('ETag');
    },
    log: (m) => console.error(m),
  }, state);
  window.__opdSession = session;

  // ---- input: only reaches the editor while editing is allowed (see session.js).
  const mods = (ed, e) => ed.modifiers(e.shiftKey, e.ctrlKey, e.altKey, e.metaKey);
  const local = (c, e) => { const r = rectOf(c); return [e.clientX - r.left, e.clientY - r.top]; };
  function wire(c) {
    c.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      session.edit((ed) => {
        c.focus();
        c.setPointerCapture(e.pointerId);
        mods(ed, e);
        ed.pointer_down(...local(c, e), e.button, e.timeStamp);
      });
    });
    c.addEventListener('pointermove', (e) => session.edit((ed) => {
      mods(ed, e);
      ed.pointer_move(...local(c, e), e.timeStamp);
    }));
    const up = (e) => session.edit((ed) => { mods(ed, e); ed.pointer_up(...local(c, e), e.button, e.timeStamp); });
    c.addEventListener('pointerup', up);
    c.addEventListener('pointercancel', up);
    c.addEventListener('contextmenu', (e) => e.preventDefault());
    c.addEventListener('wheel', (e) => {
      e.preventDefault();
      session.edit((ed) => {
        mods(ed, e);
        const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? rectOf(c).height : 1;
        ed.wheel(e.deltaX * k, e.deltaY * k, ...local(c, e));
      });
    }, { passive: false });
    c.addEventListener('keydown', (e) => {
      if (session.edit((ed) => { mods(ed, e); return ed.key_down(e.key, e.code, e.repeat); })) {
        e.preventDefault();
      }
    });
    c.addEventListener('keyup', (e) => session.edit((ed) => { mods(ed, e); ed.key_up(e.key, e.code); }));
    c.addEventListener('blur', () => session.withEditor((ed) => ed.modifiers(false, false, false, false)));
  }

  const canvas = $('canvas');
  wire(canvas);
  await session.start(canvas, await fetchPage(), forceWebgl);
  window.__opdEditor = session.ed;
  const showRenderer = () => renderer(state.fallback
    ? `WebGPU failed: ${state.fallback} — using WebGL (${state.backend})`
    : `Renderer: ${state.backend}`, Boolean(state.fallback));
  showRenderer();
  status('Starting renderer…');

  // ---- toolbar
  for (const b of document.querySelectorAll('[data-tool]')) {
    b.addEventListener('click', () => session.edit((ed) => { ed.set_tool(b.dataset.tool); session.canvas.focus(); }));
  }
  for (const b of document.querySelectorAll('[data-action]')) {
    b.addEventListener('click', () => session.edit((ed) => { ed[b.dataset.action](); session.canvas.focus(); }));
  }
  const syncToolbar = () => session.withEditor((ed) => {
    const t = ed.tool();
    for (const b of document.querySelectorAll('[data-tool]')) {
      b.setAttribute('aria-pressed', String(b.dataset.tool === t));
    }
  });

  // What the page shows (see page.js). Ready only once the renderer validated, presented a
  // frame, and has no GPU problem.
  const ui = {
    ready: () => {
      window.__opdEditor = session.ed;
      showRenderer();
      status('Ready');
      session.canvas.focus();
      // Tell this page's own server that the page loaded and the renderer is Ready, so the
      // home view can tell an actual load from a navigation request. It carries nothing else.
      api('/api/ready', { method: 'POST' }).catch(() => {});
    },
    // Terminal: shown once, in red, and the frame loop stops.
    fatal: (msg) => {
      status(msg, true);
      renderer(`Renderer failed: ${msg}`, true);
      console.error(msg);
    },
    status: (text) => status(text),
    tick: syncToolbar,
    // The conflict banner: affected layers and an explicit discard.
    // The conflict banner. The unsaved state stays on screen until the person discards it;
    // the button reloads the server's version in place (no navigation, so the leave-page
    // guard for unsaved edits stays in force for ordinary navigation).
    conflict: () => {
      const c = $('conflict');
      c.replaceChildren();
      c.append('This page was changed elsewhere (for example by an agent) since your unsaved '
        + 'change. Nothing was overwritten, and your change is not saved. ');
      const b = document.createElement('button');
      b.textContent = 'Reload (discard my unsaved change)';
      b.addEventListener('click', () => discardStep(session, ui));
      c.append(b);
      c.classList.add('show');
    },
    hideConflict: () => {
      $('conflict').classList.remove('show');
      $('conflict').replaceChildren();
    },
  };
  // ---- saving: one request at a time, always with the revision we last saw.
  setInterval(() => saveStep(session, ui), SAVE_POLL_MS);
  // ---- changes made elsewhere (e.g. an agent): reload in place when nothing is unsaved here.
  setInterval(async () => {
    try {
      const r = await session.poll();
      if (state.error) return; // a terminal error stays on screen
      if (r === 'reloaded') status('Updated with changes made elsewhere');
      else if (r === 'behind') status('Changed elsewhere; saving your edit will report a conflict');
    } catch {
      // The server is unreachable; the save loop reports that.
    }
  }, REVISION_POLL_MS);
  window.addEventListener('beforeunload', (e) => {
    if (session.ed && session.ed.dirty()) e.preventDefault();
  });

  const frame = (t) => {
    if (frameStep(session, ui, t)) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

main().catch((e) => fail(`Failed to start: ${e && e.message ? e.message : e}`));
