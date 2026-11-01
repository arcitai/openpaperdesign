// Page glue for openpaperdesign-web: loads the whole served canvas (every page, the page list and
// the active page, `/api/canvas`) and mounts the editor on it (editor.js: DOM input to upstream's
// workspace, frame loop, saving and polling), with saves of the canvas revision-checked over HTTP
// (If-Match). The editor lifecycle (gating, fallback, saving) is in session.js. The home view's
// in-panel editor mounts the same editor (panel.js).
import init, { start } from './openpaperdesign_web.js';
import { Session } from './session.js';
import { claimStep, discardStep, frameStep, offerStep, saveStep } from './page.js';
import {
  opdBrowserEnv, opdIsMac, opdMountEditor, opdResolveDark, opdStatusNote, opdSystemDark,
} from './editor.js';

// State for people and for automated checks (read-only from outside).
const state = {};
window.__opd = state;

const $ = (id) => document.getElementById(id);
const scheme = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;
// Every status goes to the live region (assistive technology, checks). Stable healthy ones
// (Ready, Saved, Saving…) show nothing; loading shows as a small note until the next state; a
// save failure or conflict stays shown through the retries until a save is acknowledged (or
// the conflict is reloaded); anything else is a note (editor.js `opdStatusNote`, the same rule
// as the home view's editor).
const status = opdStatusNote($('status'), $('alert'), {
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (id) => clearTimeout(id),
});
// Diagnostics only (which renderer runs); a renderer failure is also a status error.
const renderer = (s) => { $('renderer').textContent = s; };

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
// This tab's own view of the canvas (issue #23, `POST /api/view`): sent with every request, so
// the server tells it apart from every other tab and panel of the same canvas. Only the newest
// view edits. Without one the tab doesn't start (R78-01: no request of it acts unidentified).
let VIEW = null;

async function api(path, opts = {}) {
  const headers = new Headers(opts.headers || {});
  headers.set('Authorization', `Bearer ${TOKEN}`);
  if (VIEW) headers.set('X-OPD-View', VIEW);
  return fetch(path, { ...opts, headers, cache: 'no-store' });
}

/** One handover action on offer or pending view `id`: its status, and its answer as `body`. */
async function offerCall(path, id, resolved) {
  // `resolved`: the kept recoveries the action says it resolves (run 194, R83).
  const payload = Array.isArray(resolved) && resolved.length ? { id, resolved } : { id };
  const res = await api(path, {
    method: 'POST', body: JSON.stringify(payload), headers: { 'Content-Type': 'application/json' },
  });
  let body = null;
  if (res.status === 200) {
    try { body = await res.json(); } catch { body = null; }
  }
  return { ok: res.ok, status: res.status, body };
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
  // This tab is the newest view: it takes the canvas's editing (any other tab or panel of it,
  // here or in another window, is read-only from now on), before its first canvas read.
  const vres = await api('/api/view', { method: 'POST' });
  const v = vres.ok ? await vres.json().catch(() => null) : null;
  if (!v || typeof v.view !== 'string') {
    const why = vres.ok ? 'an unreadable answer' : `${vres.status}`;
    return fail(`This tab couldn't be registered as a view of the canvas (${why}), so it can't `
      + 'edit or hand anything over. Reload it.');
  }
  VIEW = v.view;
  // As the page goes away, this view changes nothing any more (a late request of it is refused
  // as released), and the editing view stops waiting for it.
  window.addEventListener('pagehide', () => {
    fetch('/api/view/release', {
      method: 'POST', keepalive: true, cache: 'no-store',
      headers: { Authorization: `Bearer ${TOKEN}`, 'X-OPD-View': VIEW },
    }).catch(() => {});
  });
  // The canvas as one snapshot: its text goes to the editor as it came, its revision covers
  // every page, the page list and the active page.
  // Issue #23: whether this tab may save (`X-OPD-Editing`: another view opened later may edit
  // the canvas; this one is then read-only). Absent from an older server: no claim either way.
  const editingOf = (res) => {
    const e = res.headers.get('X-OPD-Editing');
    return e === 'here' ? true : e === 'elsewhere' ? false : undefined;
  };
  const fetchCanvas = async () => {
    const res = await api('/api/canvas');
    if (!res.ok) throw new Error(`Could not load the canvas (${res.status}).`);
    return { revision: res.headers.get('ETag'), editing: editingOf(res), text: await res.text() };
  };
  // The file's name is shown by the workspace's own top bar (from the canvas it loads).
  document.title = `${project.name} — openpaperdesign`;

  const seed = crypto.getRandomValues(new Uint32Array(2));
  const forceWebgl = new URLSearchParams(location.search).get('backend') === 'webgl';
  const first = await fetchCanvas();
  const m = opdMountEditor({
    lib: { Session, frameStep, saveStep, discardStep, claimStep, offerStep },
    canvas: $('canvas'),
    status,
    renderer,
    conflictBox: $('conflict'),
    // Appearance: System (the browser's colour-scheme, live) unless Menu ▸ Appearance chooses
    // Light or Dark. This standalone page has no store that outlives it (its origin's port
    // changes per launch), so a choice lasts while it is open; the page around the workspace
    // follows the editor's resolved appearance.
    boot: (canvas, page, webgl, systemDark, appearance) => {
      const r = canvas.getBoundingClientRect();
      return start(canvas, page.text, seed[0], seed[1], opdIsMac(navigator), webgl,
        r.width, r.height, devicePixelRatio, systemDark, appearance);
    },
    onAppearance: (appearance) => {
      const dark = opdResolveDark(appearance, opdSystemDark(scheme, null));
      if (appearance === 'system') document.documentElement.removeAttribute('data-theme');
      else document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light');
    },
    transport: {
      fetchPage: fetchCanvas,
      put: async (snapshot, revision) => {
        const res = await api('/api/canvas', {
          method: 'PUT', body: snapshot,
          headers: { 'If-Match': revision, 'Content-Type': 'application/json' },
        });
        return { ok: res.ok, status: res.status, revision: res.headers.get('ETag') };
      },
      revision: async () => {
        const res = await api('/api/canvas/revision');
        if (!res.ok) throw new Error(`revision ${res.status}`);
        let body = null;
        try { body = await res.json(); } catch { body = null; }
        const editing = body && typeof body.editing === 'boolean' ? body.editing : undefined;
        const out = { revision: res.headers.get('ETag'), editing };
        if (body && 'handover' in body) out.handover = body.handover;
        if (body && 'view' in body) out.view = body.view;
        return out;
      },
      // Edit here: the canvas's editing back for this tab (issue #23).
      claim: async () => {
        const res = await api('/api/claim', { method: 'POST' });
        let body = null;
        try { body = await res.json(); } catch { body = null; }
        const note = body && typeof body.note === 'string' ? body.note : null;
        const why = body && typeof body.error === 'string' ? body.error : `claim ${res.status}`;
        return { ok: res.ok, status: res.status, note, error: res.ok ? null : why };
      },
      // Issue #23's handover (see session.js), through this tab's own view.
      answer: async (offer, base) => {
        const body = offer ? JSON.stringify({ base, ...offer }) : '{}';
        const res = await api('/api/draft', {
          method: 'POST', body, headers: { 'Content-Type': 'application/json' },
        });
        // The offer it made (`{offer}`): kept recoveries go only on that offer's receipt (R83).
        let made = null;
        if (res.ok) {
          try { made = await res.json(); } catch { made = null; }
        }
        return { ok: res.ok, status: res.status, body: made };
      },
      applyOffer: (id, resolved) => offerCall('/api/draft/apply', id, resolved),
      keepOffer: (id, resolved) => offerCall('/api/draft/keep', id, resolved),
      dismissOffer: (id, resolved) => offerCall('/api/draft/dismiss', id, resolved),
      stopWaiting: (id) => offerCall('/api/draft/stop', id),
    },
    firstPage: first,
    forceWebgl,
    state,
    // The tab's title follows the name the editor has now: a finished rename, a reload's.
    onName: (name) => { document.title = `${name || project.name} — openpaperdesign`; },
    onReady: (session) => {
      window.__opdEditor = session.ed;
      // Tell this page's own server that the page loaded and the renderer is Ready, so the
      // home view can tell an actual load from a navigation request. It carries nothing else.
      api('/api/ready', { method: 'POST' }).catch(() => {});
    },
    log: (msg) => console.error(msg),
    env: opdBrowserEnv(window),
  });
  window.__opdSession = m.session;
  await m.started;
  window.__opdEditor = m.session.ed;
  // Unsaved content on any page or in the page list, an unfinished edit in the workspace (a
  // layer or page name being typed), a change applied here whose unfinished edits aren't open
  // yet, or an offer action not answered yet (`Session.holdsWork`, R80-02).
  window.addEventListener('beforeunload', (e) => {
    if (m.session.holdsWork()) e.preventDefault();
  });
}

main().catch((e) => fail(`Failed to start: ${e && e.message ? e.message : e}`));
