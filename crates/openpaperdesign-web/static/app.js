// Page glue for openpaperdesign-web: loads the whole served canvas (every page, the page list and
// the active page, `/api/canvas`) and mounts the editor on it (editor.js: DOM input to upstream's
// workspace, frame loop, saving and polling), with saves of the canvas revision-checked over HTTP
// (If-Match). The editor lifecycle (gating, fallback, saving) is in session.js. The home view's
// in-panel editor mounts the same editor (panel.js).
import init, { start } from './openpaperdesign_web.js';
import { Session } from './session.js';
import { discardStep, frameStep, saveStep } from './page.js';
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

async function api(path, opts = {}) {
  const headers = new Headers(opts.headers || {});
  headers.set('Authorization', `Bearer ${TOKEN}`);
  return fetch(path, { ...opts, headers, cache: 'no-store' });
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
  // The canvas as one snapshot: its text goes to the editor as it came, its revision covers
  // every page, the page list and the active page.
  const fetchCanvas = async () => {
    const res = await api('/api/canvas');
    if (!res.ok) throw new Error(`Could not load the canvas (${res.status}).`);
    return { revision: res.headers.get('ETag'), text: await res.text() };
  };
  // The file's name is shown by the workspace's own top bar (from the canvas it loads).
  document.title = `${project.name} — openpaperdesign`;

  const seed = crypto.getRandomValues(new Uint32Array(2));
  const forceWebgl = new URLSearchParams(location.search).get('backend') === 'webgl';
  const first = await fetchCanvas();
  const m = opdMountEditor({
    lib: { Session, frameStep, saveStep, discardStep },
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
        return res.headers.get('ETag');
      },
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
  // Unsaved content on any page or in the page list, or an unfinished edit in the workspace (a
  // layer or page name being typed).
  window.addEventListener('beforeunload', (e) => {
    if (m.session.ed && m.session.ed.unsaved_work()) e.preventDefault();
  });
}

main().catch((e) => fail(`Failed to start: ${e && e.message ? e.message : e}`));
