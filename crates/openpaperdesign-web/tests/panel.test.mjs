// The in-panel editor module as openpaperdesign-mcp assembles it (`panel_module` in
// crates/openpaperdesign-serve/src/launcher.rs): the built wasm-bindgen glue, then session.js,
// page.js, editor.js and panel.js, in that order, as one ES module, imported here for real.
// Needs the built glue (./scripts/build-web.sh writes target/openpaperdesign-web/): without it
// these tests fail; they never skip.
//
// What is real here: the module's parsing and linking, the renderer part transfer and its
// checks, the page transport, Session and the editor mount, against a fake MCP Apps parent
// that answers like the server's in-panel tools. What is not: the browser, the host, the GPU.
// The renderer's compile and draw are replaced by an explicit stand-in engine (WebAssembly in
// Node proves nothing about a host's CSP or graphics), so nothing here shows Ready in a host.
// Run: node --test crates/openpaperdesign-web/tests/panel.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const STATIC = new URL('../static/', import.meta.url);
const GLUE = new URL('../../../target/openpaperdesign-web/openpaperdesign_web.js', import.meta.url);
const OWN = ['session.js', 'page.js', 'editor.js', 'panel.js'];
const IDENTITY = '"00112233445566ff"';

// Same parts, order and identity line as `panel_module`.
function assemble(identity = IDENTITY) {
  let out = '// The in-panel editor (assembled by openpaperdesign-mcp from its packaged files).\n'
    + `const OPD_WASM_IDENTITY = ${JSON.stringify(identity)};\n`;
  const parts = [['openpaperdesign_web.js', readFileSync(GLUE, 'utf8')],
    ...OWN.map((f) => [f, readFileSync(new URL(f, STATIC), 'utf8')])];
  for (const [name, text] of parts) out += `\n// ---- ${name} ----\n${text}\n`;
  return out;
}
const asModule = (source) => import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const SOURCE = assemble();
const mod = await asModule(SOURCE);
const flush = async () => { for (let i = 0; i < 8; i++) await new Promise((r) => setImmediate(r)); };

test('the assembled module links, imports nothing and is safe inside the view\'s script element', async () => {
  // opdStatusDisplay: the home view's editor uses the editor's own rule for how its statuses
  // show (and keeps an unresolved save failure shown); the browser page uses opdStatusNote.
  for (const name of ['opdCreatePanel', 'opdMountEditor', 'opdBrowserEnv', 'opdStatusDisplay', 'opdStatusNote', 'opdStatusKind', 'opdQuietStatus', 'opdSystemDark', 'opdResolveDark', 'opdAppearance', 'Session', 'frameStep', 'saveStep', 'discardStep', 'start']) {
    assert.equal(typeof mod[name], 'function', name);
  }
  assert.equal(typeof mod.default, 'function', 'the glue\'s own init (__wbg_init)');
  const lower = SOURCE.toLowerCase();
  for (const bad of ['</script', '<script', '<!--']) assert.ok(!lower.includes(bad), bad);
  assert.ok(!SOURCE.split('\n').some((l) => /^\s*import[\s{(]/.test(l)), 'no import declaration or dynamic import');
  for (const f of OWN) {
    const text = readFileSync(new URL(f, STATIC), 'utf8');
    for (const banned of ['eval(', 'new Function', 'import(', 'fetch(', 'XMLHttpRequest', 'localStorage', 'sessionStorage', 'indexedDB', 'ui/message']) {
      assert.ok(!text.includes(banned), `${f}: ${banned}`);
    }
  }
  assert.ok(readFileSync(new URL('panel.js', STATIC), 'utf8').includes('__wbg_init({ module_or_path: bytes })'),
    'the glue is initialized with the transferred bytes, by its own init');
});

test('the home view\'s module with the editor in its slot parses and links as one module', async () => {
  const html = readFileSync(new URL('../../openpaperdesign-serve/src/home.html', import.meta.url), 'utf8');
  const open = '<script type="module">';
  const slot = 'const opdCreatePanel = null; // __OPD_PANEL_MODULE__';
  const home = html.slice(html.lastIndexOf(open) + open.length, html.lastIndexOf('</script>'));
  assert.equal(home.split(slot).length, 2, 'exactly one editor slot');
  // A function replacement: the glue's text is inserted as it is (no `$` patterns).
  const filled = home.replace(slot, () => SOURCE).replace("'__OPD_MODE__'", "'library'");
  // Parsing and linking succeed (a duplicate name or an import would be a SyntaxError before
  // anything ran); it then stops at the first DOM use, as there is no document here.
  await assert.rejects(asModule(filled), (e) => e.name === 'ReferenceError' && /document/.test(e.message));
});

// ---- a fake MCP Apps parent answering like the server's in-panel tools ----

const BINDING = '0123456789abcdef.7';
const PART = 262144;
const T = {
  ASSET: 'openpaperdesign_app_editor_asset', CANVAS: 'openpaperdesign_app_canvas', REVISION: 'openpaperdesign_app_canvas_revision',
  SAVE: 'openpaperdesign_app_canvas_save', READY: 'openpaperdesign_app_ready', RELEASE: 'openpaperdesign_app_page_release',
  // The per-page tools stay on the server for older views; this editor never calls them.
  PAGE: 'openpaperdesign_app_page', PAGE_SAVE: 'openpaperdesign_app_page_save',
};
const ok = (v) => ({ result: { content: [{ type: 'text', text: JSON.stringify(v) }] } });
const no = (status, error, revision = null) => ({ result: { isError: true, content: [{ type: 'text', text: JSON.stringify({ error, status, revision }) }] } });
// What the editor is given to open: the file's name (the fake server's canvas is "Plan"), the
// canvas's pages and active page, as text.
const opened = (c) => JSON.stringify({ canvas: { name: 'Plan' }, active_page: c.active_page, pages: c.pages });

function fakeServer(wasm = new Uint8Array(600000).fill(0x61)) {
  // The canvas as the server has it: its pages (id, name, revision, page file), the active page
  // and the canvas revision; `saved` is the last canvas save it stored.
  const srv = { calls: [], canvas: { active_page: 'p1', pages: [{ id: 'p1', name: 'Page 1', revision: '"a"', text: '{"page":1}' }] },
    revision: '"r1"', saved: undefined, mounts: new Set(), next: 0, wasm, identity: IDENTITY, override: {}, holds: {} };
  srv.part = (i) => {
    const parts = Math.ceil(srv.wasm.length / PART);
    const start = i * PART;
    const slice = srv.wasm.subarray(start, Math.min(start + PART, srv.wasm.length));
    return { asset: 'openpaperdesign_web_bg.wasm', identity: srv.identity, bytes: srv.wasm.length, parts, part: i, offset: start,
      part_bytes: slice.length, data: Buffer.from(slice).toString('base64') };
  };
  srv.rpc = async (method, params) => {
    assert.equal(method, 'tools/call', 'the editor only calls tools');
    const { name, arguments: a } = params;
    srv.calls.push([name, JSON.parse(JSON.stringify(a))]);
    if (srv.holds[name]) await srv.holds[name];
    if (srv.override[name]) return srv.override[name](a);
    const pinned = () => a.binding === BINDING && srv.mounts.has(a.mount);
    const canvas = (mount) => ok({ binding: BINDING, canvas: { name: 'Plan' }, revision: srv.revision,
      active_page: srv.canvas.active_page, pages: srv.canvas.pages, mount });
    switch (name) {
      case T.ASSET: return ok(srv.part(a.part));
      case T.CANVAS:
        if (a.binding !== BINDING) return no(410, 'stale');
        if (a.mount === undefined) {
          srv.mounts.add(++srv.next);
          return canvas(srv.next);
        }
        if (!pinned()) return no(410, 'not mounted');
        return canvas(a.mount);
      case T.REVISION:
        if (!pinned()) return no(410, 'not mounted');
        return ok({ binding: BINDING, revision: srv.revision });
      case T.SAVE:
        if (!pinned()) return no(410, 'not mounted');
        if (a.if_match !== srv.revision) return no(409, 'the canvas changed since it was loaded; nothing was written', srv.revision);
        srv.saved = a.text;
        srv.revision = `"s${srv.calls.length}"`;
        return ok({ binding: BINDING, saved: true, revision: srv.revision });
      case T.READY: return ok({ binding: BINDING, ready: true });
      case T.RELEASE: return ok({ binding: BINDING, released: srv.mounts.delete(a.mount) });
      default: throw new Error('unexpected tool ' + name);
    }
  };
  srv.named = (name) => srv.calls.filter(([n]) => n === name).map(([, a]) => a);
  return srv;
}

// A minimal DOM: listeners honour AbortSignal, so disposal can be checked.
function fakeEl(doc, tag = 'div') {
  const e = {
    tag, attrs: {}, children: [], classes: new Set(), ownerDocument: doc, listeners: [], focused: 0, captured: null, textContent: '',
    addEventListener(type, fn, o = {}) {
      const l = { type, fn };
      e.listeners.push(l);
      if (o.signal) o.signal.addEventListener('abort', () => { e.listeners = e.listeners.filter((x) => x !== l); });
    },
    fire(type, ev = {}) {
      for (const l of [...e.listeners]) if (l.type === type) l.fn(Object.assign({ preventDefault() {}, stopPropagation() { ev.stopped = true; } }, ev));
    },
    setAttribute(k, v) { e.attrs[k] = String(v); },
    getAttribute(k) { return k in e.attrs ? e.attrs[k] : null; },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }),
    focus() { e.focused += 1; },
    setPointerCapture(id) { e.captured = id; },
    hasPointerCapture(id) { return e.captured === id; },
    releasePointerCapture(id) { if (e.captured === id) e.captured = null; },
    cloneNode() { return fakeEl(doc, tag); },
    replaceWith(n) { e.replacedBy = n; },
    replaceChildren() { e.children = []; },
    append(...c) { e.children.push(...c); },
    classList: { add: (c) => e.classes.add(c), remove: (c) => e.classes.delete(c) },
  };
  return e;
}

function fakeEnv() {
  const env = {
    intervals: new Map(), frames: [], next: 1,
    setInterval(fn, ms) { const id = env.next++; env.intervals.set(id, { fn, ms }); return id; },
    clearInterval(id) { env.intervals.delete(id); },
    requestAnimationFrame(fn) { const id = env.next++; env.frames.push([id, fn]); return id; },
    cancelAnimationFrame(id) { env.frames = env.frames.filter(([i]) => i !== id); },
    // Each observer the mount creates is kept (`env.observers`), so a test can fire it.
    ResizeObserver: class { constructor(cb) { this.cb = cb; env.observers.push(this); } observe() {} disconnect() { this.disconnected = true; } },
    observers: [],
    AbortController,
    devicePixelRatio: () => 1,
  };
  env.frame = (t) => { const f = env.frames.shift(); if (f) f[1](t); };
  env.every = (ms) => { for (const { fn, ms: m } of [...env.intervals.values()]) if (m === ms) fn(); };
  return env;
}

// The stand-in engine: records the bytes it is given and boots a mock editor.
function fakeEngine() {
  const eng = { inits: [], editors: [] };
  eng.init = async (bytes) => { eng.inits.push(bytes); };
  eng.start = async (canvas, text, seedHi, seedLo, mac, webgl, width, height, dpr, systemDark, appearance) => {
    // The real editor's page-facing API (web.rs); the workspace itself draws in the canvas.
    // Appearance: what it was started with (`startSystem`, `startAppearance`), then each
    // `set_system_dark` (`systems`) and `set_appearance` (`kept`) since; `pref` is what its
    // menu shows (a test sets it, then asks for an 'appearance' request).
    const ed = { canvas, text, mac, webgl, startSystem: systemDark, startAppearance: appearance, systems: [], kept: [],
      pref: appearance, calls: [], pending: undefined, isReady: false, freed: false, canReload: true,
      requests: [], downloads: [], clip: undefined,
      set_system_dark(d) { this.systems.push(d); },
      set_appearance(a) { this.kept.push(a); this.pref = a; return true; },
      appearance() { return this.pref; },
      backend() { return 'mock (test)'; }, frame() {}, gpu_error() { return undefined; }, ready() { return this.isReady; },
      tool() { return 'select'; }, resize() {}, modifiers() {}, focus(f) { this.calls.push('focus:' + f); },
      pointer_down() { this.calls.push('down'); }, pointer_move() {}, pointer_up() { this.calls.push('up'); }, pointer_leave() {}, wheel() {},
      key_down(k) { this.calls.push('key:' + k); return true; }, key_up() {},
      copy() { return undefined; }, paste(t) { this.calls.push('paste:' + t); },
      cursor() { return 'default'; }, take_request() { return this.requests.shift(); },
      take_download() { return this.downloads.shift(); },
      take_clipboard_text() { const c = this.clip; this.clip = undefined; return c; },
      save_snapshot() { const s = this.pending; this.pending = undefined; return s; },
      save_succeeded() { this.calls.push('saved'); }, save_failed() { this.calls.push('save_failed'); },
      dirty() { return this.pending !== undefined; }, can_reload() { return this.canReload && this.pending === undefined; },
      reload(t) { this.calls.push('reload:' + t); }, free() { this.freed = true; } };
    eng.editors.push(ed);
    return ed;
  };
  return eng;
}

function setup(srv = fakeServer()) {
  const doc = { createElement: (t) => fakeEl(doc, t) };
  const env = fakeEnv();
  const engine = fakeEngine();
  const shown = { status: [], renderer: [] };
  const refs = {
    canvas: fakeEl(doc, 'canvas'),
    status: (t, error) => shown.status.push(error ? 'ERR ' + t : t),
    renderer: (t, error) => shown.renderer.push(error ? 'ERR ' + t : t),
    conflict: fakeEl(doc),
  };
  const panel = mod.opdCreatePanel({ rpc: srv.rpc, identity: IDENTITY, engine, env, nav: { platform: 'Linux' } });
  return { srv, env, engine, refs, shown, panel, doc };
}

async function readyEditor(t = setup()) {
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {} });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors.at(-1);
  ed.isReady = true;
  t.env.frame(16);
  await flush();
  return Object.assign(t, { h, ed });
}

test('the renderer comes in order, part after part, checked, and is compiled once with exactly those bytes', async () => {
  const wasm = new Uint8Array(600000);
  for (let i = 0; i < wasm.length; i++) wasm[i] = (i * 7) & 0xff;
  const t = setup(fakeServer(wasm));
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {} });
  assert.equal(await h.started, 'mounted');
  assert.deepEqual(t.srv.calls.map(([n, a]) => (n === T.ASSET ? `asset ${a.part}` : n)).slice(0, 4),
    ['asset 0', 'asset 1', 'asset 2', T.CANVAS], 'three parts, in order, then the canvas');
  assert.equal(t.engine.inits.length, 1);
  assert.deepEqual(t.engine.inits[0], wasm, 'exactly the packaged bytes');
  assert.deepEqual(t.srv.named(T.CANVAS)[0], { binding: BINDING }, 'the first read names only the binding');
  const ed = t.engine.editors[0];
  assert.equal(ed.text, opened(t.srv.canvas), 'the editor opens the whole canvas');
  assert.equal(JSON.parse(ed.text).pages[0].text, '{"page":1}', 'each page as its upstream page file');
  // A second editor in the same view reuses the compiled renderer.
  await h.dispose();
  const h2 = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {} });
  assert.equal(await h2.started, 'mounted');
  assert.equal(t.srv.named(T.ASSET).length, 3, 'no second transfer');
  assert.equal(t.engine.inits.length, 1);
});

test('a wrong, mixed, out-of-order, short or oversized part stops the transfer with the reason; nothing is compiled', async () => {
  for (const [what, fault, want] of [
    ['another build', (p) => ({ ...p, identity: '"ffffffffffffffff"' }), /another build/],
    ['out of order', (p) => ({ ...p, part: p.part + 1 }), /out of order or incomplete/],
    ['short data', (p) => ({ ...p, data: p.data.slice(0, 8) }), /isn't the length it says/],
    ['changed count', (p, i) => (i === 1 ? { ...p, bytes: p.bytes + PART, parts: p.parts + 1 } : p), /doesn't match the earlier parts/],
    ['oversized', (p) => ({ ...p, bytes: 17 * 1024 * 1024, parts: 68 }), /unexpected size/],
    ['fractional size', (p) => ({ ...p, bytes: p.bytes + 0.5 }), /unexpected size/],
    ['not base64', (p) => ({ ...p, data: '%%%%' }), /isn't the length it says/],
  ]) {
    const srv = fakeServer();
    srv.override[T.ASSET] = (a) => ok(fault(srv.part(a.part), a.part));
    const t = setup(srv);
    const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {} });
    await assert.rejects(h.started, want, what);
    assert.equal(t.engine.inits.length, 0, `${what}: nothing compiled`);
    assert.equal(srv.named(T.CANVAS).length, 0, `${what}: no canvas read`);
  }
  // A refused part is reported as the server said it.
  const srv = fakeServer();
  srv.override[T.ASSET] = () => ({ result: { isError: true, content: [{ type: 'text', text: 'This OpenPaperDesign command was built without the browser editor' }] } });
  const t = setup(srv);
  await assert.rejects(t.panel.open({ binding: BINDING, refs: t.refs }).started, /could not be sent \(part 0\): This OpenPaperDesign command was built without/);
  // A compile refusal (as a host's CSP would give) comes through as it came.
  const c = setup();
  c.engine.init = async () => { throw new Error('WebAssembly.instantiate(): Refused to compile or instantiate WebAssembly module because \'unsafe-eval\' is not allowed'); };
  await assert.rejects(c.panel.open({ binding: BINDING, refs: c.refs }).started, /Refused to compile/);
  assert.equal(c.srv.named(T.CANVAS).length, 0);
});

test('leaving during the transfer stops it; leaving during the first read releases the mount it got', async () => {
  const srv = fakeServer();
  let release;
  srv.holds[T.ASSET] = new Promise((r) => { release = r; });
  const t = setup(srv);
  const h = t.panel.open({ binding: BINDING, refs: t.refs });
  await flush();
  await h.dispose();
  release();
  assert.equal(await h.started, 'left');
  assert.equal(srv.named(T.ASSET).length, 1, 'no part after leaving');
  assert.equal(t.engine.inits.length, 0);
  assert.equal(srv.named(T.CANVAS).length, 0);
  // The canvas read under way.
  const s2 = fakeServer();
  let answer;
  s2.holds[T.CANVAS] = new Promise((r) => { answer = r; });
  const t2 = setup(s2);
  const h2 = t2.panel.open({ binding: BINDING, refs: t2.refs });
  await flush();
  assert.equal(s2.named(T.CANVAS).length, 1);
  await h2.dispose();
  answer();
  assert.equal(await h2.started, 'left');
  await flush();
  assert.deepEqual(s2.named(T.RELEASE), [{ binding: BINDING, mount: 1 }], 'the mount it was given is released');
  assert.equal(t2.engine.editors.length, 0, 'no editor booted');
});

test('an edit is saved through the canvas tool, pinned to its mount; Ready is the renderer\'s own', async () => {
  const t = await readyEditor();
  assert.ok(t.shown.status.includes('Ready'));
  assert.deepEqual(t.srv.named(T.READY), [{ binding: BINDING, mount: 1 }]);
  // Real input through the real mount, then the save loop.
  t.refs.canvas.fire('pointerdown', { pointerId: 3, button: 0, clientX: 5, clientY: 5, timeStamp: 1 });
  assert.equal(t.refs.canvas.captured, 3);
  t.refs.canvas.fire('pointerup', { pointerId: 3, button: 0, clientX: 9, clientY: 9, timeStamp: 2 });
  assert.deepEqual(t.ed.calls.slice(0, 2), ['down', 'up']);
  t.ed.pending = '{"pages":[],"edit":true}';
  t.env.every(300);
  await flush();
  assert.deepEqual(t.srv.named(T.SAVE), [{ binding: BINDING, mount: 1, if_match: '"r1"', text: '{"pages":[],"edit":true}' }]);
  assert.equal(t.srv.saved, '{"pages":[],"edit":true}', 'the server has the canvas save');
  assert.equal(t.shown.status.at(-1), 'Saved');
  assert.ok(t.ed.calls.includes('saved'));
  // Polls name the same mount.
  t.env.every(1000);
  await flush();
  assert.deepEqual(t.srv.named(T.REVISION).at(-1), { binding: BINDING, mount: 1 });
  // The per-page tools are never this editor's.
  assert.equal(t.srv.named(T.PAGE).length + t.srv.named(T.PAGE_SAVE).length, 0);
  // The canvas's keys are the editor's, not the view's (its Escape is Back).
  const key = { key: 'Escape', code: 'Escape', repeat: false };
  t.refs.canvas.fire('keydown', key);
  assert.equal(key.stopped, true);
  assert.ok(t.ed.calls.includes('key:Escape'));
});

test('leaving waits for the save under way and saves what is left; then everything is undone once', async () => {
  const t = await readyEditor();
  let answer;
  t.srv.holds[T.SAVE] = new Promise((r) => { answer = r; });
  t.ed.pending = '{"v":1}';
  t.env.every(300);
  await flush();
  t.ed.pending = '{"v":2}'; // a completed edit after that snapshot
  const leaving = t.h.drain();
  t.refs.canvas.fire('pointerdown', { pointerId: 1, button: 0, clientX: 1, clientY: 1, timeStamp: 3 });
  assert.ok(!t.ed.calls.includes('down'), 'no input while leaving');
  delete t.srv.holds[T.SAVE];
  answer();
  assert.equal(await leaving, 'clean');
  assert.deepEqual(t.srv.named(T.SAVE).map((a) => a.text), ['{"v":1}', '{"v":2}']);
  assert.equal(t.srv.saved, '{"v":2}');
  await t.h.dispose();
  await t.h.dispose();
  assert.equal(t.ed.freed, true);
  assert.equal(t.env.intervals.size, 0, 'save and poll timers stopped');
  assert.equal(t.env.frames.length, 0, 'frame loop stopped');
  assert.equal(t.refs.canvas.listeners.length, 0, 'canvas listeners removed');
  assert.deepEqual(t.srv.named(T.RELEASE), [{ binding: BINDING, mount: 1 }], 'released once');
  // Nothing more reaches it.
  t.ed.pending = '{"v":3}';
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).length, 2);
});

test('a conflict, an unanswered save or a page too large for the panel is never shown as saved, and keeps the editor', async () => {
  // Conflict: the canvas changed elsewhere (any page, the list or the active page).
  const c = await readyEditor();
  c.srv.revision = '"elsewhere"';
  c.ed.pending = '{"mine":1}';
  c.env.every(300);
  await flush();
  assert.equal(c.shown.status.at(-1), 'Not saved: conflict');
  assert.ok(c.refs.conflict.classes.has('show'), 'the banner with its explicit discard');
  assert.equal(await c.h.drain(), 'conflict');
  assert.equal(c.ed.freed, false);
  assert.ok(!c.shown.status.includes('Saved'));
  // No answer (the host timed out): unknown, retried, never Saved.
  const u = await readyEditor();
  u.srv.override[T.SAVE] = () => ({ timeout: true });
  u.ed.pending = '{"mine":1}';
  u.env.every(300);
  await flush();
  assert.equal(u.shown.status.at(-1), 'Save failed (server unreachable); retrying');
  u.ed.pending = '{"mine":1}'; // the mock forgets on failure; the real editor keeps it dirty
  assert.equal(await u.h.drain(), 'unreachable');
  assert.ok(!u.shown.status.includes('Saved'));
  // Too large: refused here, unsent.
  const l = await readyEditor();
  l.ed.pending = 'x'.repeat(4 * 1024 * 1024 + 1);
  l.env.every(300);
  await flush();
  assert.equal(l.srv.named(T.SAVE).length, 0, 'not sent');
  assert.equal(l.shown.status.at(-1), 'Save failed; retrying');
});

test('a save is bounded as the whole request it would be (envelope, escaping and host allowance), not its arguments', async () => {
  const MAX = 4 * 1024 * 1024;
  const bytes = (s) => new TextEncoder().encode(s).length;
  const t = await readyEditor();
  // Escaping-heavy: the arguments alone fit, the request as sent (plus the allowance) doesn't.
  const quotes = '"'.repeat(MAX / 2 - 4096);
  const args = { binding: BINDING, mount: 1, if_match: '"r1"', text: quotes };
  assert.ok(bytes(JSON.stringify(args)) < MAX, 'the arguments alone fit');
  t.ed.pending = quotes;
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).length, 0, 'not sent');
  assert.equal(t.shown.status.at(-1), 'Save failed; retrying');
  // Plain text just under the arguments-only limit: not sent either.
  t.ed.pending = 'x'.repeat(MAX - 1024);
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).length, 0);
  // Well within the whole-request limit: sent, and saved.
  const fits = 'x'.repeat(MAX - 64 * 1024);
  t.ed.pending = fits;
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).length, 1);
  assert.equal(t.srv.saved, fits);
  assert.equal(t.shown.status.at(-1), 'Saved');
});

test('what the workspace asks the page for: Save saves now, Home goes back through the view, nothing is dropped', async () => {
  const t = setup();
  let homes = 0;
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {}, onHome: () => { homes += 1; } });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors.at(-1);
  ed.isReady = true;
  t.env.frame(16);
  await flush();
  // Upstream's Save (⌘S or File > Save Now): the same checked canvas save, at once.
  ed.pending = '{"page":1,"saved":"now"}';
  ed.requests.push('save');
  t.env.frame(32);
  await flush();
  assert.deepEqual(t.srv.named(T.SAVE).map((a) => a.text), ['{"page":1,"saved":"now"}'], 'saved without waiting for the timer');
  assert.equal(t.shown.status.at(-1), 'Saved');
  // Upstream's Home (All projects): the view's own Back, which drains first.
  ed.requests.push('home');
  t.env.frame(48);
  assert.equal(homes, 1);
  // An export the page can't download here is said so, not dropped silently.
  ed.downloads.push({ name: 'Page 1.svg', mime: 'image/svg+xml', bytes: new Uint8Array([60]) });
  t.env.frame(64);
  assert.match(t.shown.status.at(-1), /can't start downloads here/);
  assert.equal(ed.downloads.length, 0);
  // Focus is the editor's (egui's text fields need it).
  t.refs.canvas.fire('focus');
  assert.ok(ed.calls.includes('focus:true'));
  await h.dispose();
});

test('a stale binding or released mount is refused by the server and never applied', async () => {
  const t = await readyEditor();
  t.srv.mounts.clear(); // another view closed or replaced the canvas
  t.ed.pending = '{"mine":1}';
  t.env.every(300);
  await flush();
  assert.equal(t.srv.saved, undefined, 'nothing written');
  assert.equal(t.shown.status.at(-1), 'Save failed; retrying');
  t.env.every(1000);
  await flush();
  assert.ok(!t.ed.calls.some((c) => c.startsWith('reload')), 'a refused poll loads nothing');
});

test('the whole canvas travels: every page and the active page open, and a change elsewhere reloads all of it', async () => {
  const srv = fakeServer();
  srv.canvas = {
    active_page: 'p2',
    pages: [{ id: 'p1', name: 'Plan', revision: '"a"', text: '{"page":1}' },
      { id: 'p2', name: 'Notes', revision: '"b"', text: '{"page":2}' }],
  };
  const t = await readyEditor(setup(srv));
  const first = JSON.parse(t.ed.text);
  assert.equal(first.active_page, 'p2', 'the remembered active page');
  assert.deepEqual(first.pages.map((p) => [p.id, p.name, p.text]),
    [['p1', 'Plan', '{"page":1}'], ['p2', 'Notes', '{"page":2}']], 'every page, in list order');
  // An agent added a page and switched to it: the canvas revision moves, and a clean editor
  // loads the whole canvas in place.
  srv.canvas = { active_page: 'p3', pages: [...srv.canvas.pages, { id: 'p3', name: 'Agent', revision: '"c"', text: '{"page":3}' }] };
  srv.revision = '"r2"';
  t.env.every(1000);
  await flush();
  const reloaded = t.ed.calls.filter((c) => c.startsWith('reload:')).map((c) => JSON.parse(c.slice('reload:'.length)));
  assert.equal(reloaded.length, 1);
  assert.equal(reloaded[0].active_page, 'p3');
  assert.deepEqual(reloaded[0].pages.map((p) => p.id), ['p1', 'p2', 'p3']);
  assert.equal(t.shown.status.at(-1), 'Updated with changes made elsewhere');
  // The next save is based on that revision.
  t.ed.pending = '{"pages":[]}';
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).at(-1).if_match, '"r2"');
  assert.equal(t.shown.status.at(-1), 'Saved');
});

test('an answer for another binding, or without pages, is never opened', async () => {
  for (const bad of [{ binding: '0123456789abcdef.8' }, { pages: [] }, { pages: [{ id: 'p1', name: 'P' }] }, { active_page: 5 }]) {
    const srv = fakeServer();
    srv.override[T.CANVAS] = (a) => {
      srv.mounts.add(++srv.next);
      return ok(Object.assign({ binding: BINDING, revision: srv.revision, active_page: 'p1', pages: srv.canvas.pages, mount: srv.next }, bad));
    };
    const t = setup(srv);
    await assert.rejects(t.panel.open({ binding: BINDING, refs: t.refs }).started, /another canvas/, JSON.stringify(bad));
    assert.equal(t.engine.editors.length, 0, 'no editor booted');
    assert.deepEqual(srv.named(T.RELEASE), [{ binding: BINDING, mount: 1 }], 'its mount is released');
  }
});

// ---- resizing: the real mount redraws in the resize callback itself ----
// The actual mount and Session: the mount's ResizeObserver callback resizes the editor (which
// reconfigures its surface, clearing the canvas) and the picture is drawn again before the
// callback returns, i.e. before the page paints; the animation frame loop is unchanged.
// Graphics are the stand-in engine, so this shows the call order, not pixels.

test('the mount\'s resize callback resizes the editor and draws it again before returning', async () => {
  const t = await readyEditor();
  const order = [];
  t.ed.resize = (w, h, dpr) => order.push(`resize ${w}x${h}@${dpr}`);
  t.ed.frame = (time) => order.push(`frame ${time}`);
  const frames = t.env.frames.length;
  const observer = t.env.observers.at(-1);
  assert.ok(observer && !observer.disconnected, 'the editor\'s canvas is observed');
  observer.cb();
  assert.deepEqual(order, ['resize 800x600@1', 'frame 16'], 'resized, then drawn at the last frame\'s time');
  assert.equal(t.env.frames.length, frames, 'no extra animation frame is scheduled');
  // A burst of resize callbacks between two animation frames: each one redraws.
  order.length = 0;
  for (let i = 0; i < 3; i++) observer.cb();
  assert.deepEqual(order, ['resize 800x600@1', 'frame 16', 'resize 800x600@1', 'frame 16', 'resize 800x600@1', 'frame 16']);
  // After disposal nothing reaches the editor.
  await t.h.dispose();
  order.length = 0;
  observer.cb();
  assert.deepEqual(order, []);
  assert.equal(observer.disconnected, true);
});

// ---- appearance: the real mount gives each editor the OS appearance and the kept preference,
// follows the OS live, and hands a menu choice to the view to keep ----
// The browser's colour-scheme query and the host's theme are stand-ins; what is checked is
// what each editor is given and when, and what the view is asked to keep, not pixels.

function scheme(t, matches) {
  const media = Object.assign(fakeEl(t.doc, 'media'), { matches });
  t.env.colorScheme = () => media;
  return media;
}

test('the resolution rule: the browser\'s colour-scheme first, the host\'s theme only without it, explicit modes win', () => {
  assert.equal(mod.opdSystemDark({ matches: true }, 'light'), true, 'the OS signal wins over the host');
  assert.equal(mod.opdSystemDark({ matches: false }, 'dark'), false);
  assert.equal(mod.opdSystemDark(null, 'dark'), true, 'the host only where the browser can\'t say');
  assert.equal(mod.opdSystemDark({}, 'light'), false);
  assert.equal(mod.opdSystemDark(null, 'sepia'), null);
  assert.equal(mod.opdResolveDark('system', true), true);
  assert.equal(mod.opdResolveDark('system', null), false, 'no signal at all: light');
  assert.equal(mod.opdResolveDark('light', true), false, 'Light overrides a dark OS');
  assert.equal(mod.opdResolveDark('dark', false), true, 'Dark overrides a light OS');
  for (const bad of [undefined, null, '', 'Dark', 'auto', 5, {}]) assert.equal(mod.opdAppearance(bad), 'system', String(bad));
});

test('the editor starts in the kept appearance and the OS one, follows the OS live, and a menu choice is kept by the view; nothing after disposal', async () => {
  const t = setup();
  const media = scheme(t, true);
  let host = 'light';
  const chosen = [];
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {},
    appearance: 'light', hostTheme: () => host, onAppearance: (a) => chosen.push(a) });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors.at(-1);
  assert.equal(ed.startSystem, true, 'the browser\'s dark OS, from the first frame (not the host\'s light)');
  assert.equal(ed.startAppearance, 'light', 'the kept preference, from the first frame');
  assert.deepEqual([ed.systems, ed.kept], [[true], ['light']], 'given again once booted');
  ed.isReady = true;
  t.env.frame(16);
  await flush();
  // The OS changes: told live. The host's theme doesn't matter while the browser can say.
  media.matches = false;
  media.fire('change');
  assert.equal(ed.systems.at(-1), false);
  host = 'dark';
  h.hostThemeChanged();
  assert.equal(ed.systems.at(-1), false, 'the browser\'s signal still wins');
  // Menu ▸ Appearance ▸ Dark in the editor: the view is asked to keep it, once.
  ed.pref = 'dark';
  ed.requests.push('appearance');
  t.env.frame(32);
  assert.deepEqual(chosen, ['dark']);
  assert.equal(h.mounted.appearance, 'dark');
  // An unknown answer is kept as System, never passed on as it is.
  ed.pref = 'sepia';
  ed.requests.push('appearance');
  t.env.frame(48);
  assert.deepEqual(chosen, ['dark', 'system']);
  // Presentation only: nothing was saved, reloaded or released.
  t.env.every(300);
  await flush();
  assert.equal(t.srv.named(T.SAVE).length, 0);
  assert.equal(t.srv.named(T.RELEASE).length, 0);
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  // Disposal: the listener goes; nothing reaches the freed editor, nothing more is kept.
  await h.dispose();
  const told = ed.systems.length;
  media.matches = true;
  media.fire('change');
  h.hostThemeChanged();
  assert.equal(ed.systems.length, told);
  assert.equal(media.listeners.length, 0, 'the colour-scheme listener is removed');
  assert.equal(ed.freed, true);
  assert.deepEqual(chosen, ['dark', 'system']);
});

test('without a browser signal the host\'s theme stands in, and changes with it', async () => {
  const t = setup();
  let host = 'dark';
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {}, hostTheme: () => host });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors.at(-1);
  assert.equal(ed.startSystem, true);
  assert.equal(ed.startAppearance, 'system', 'no kept preference: System');
  host = 'light';
  h.hostThemeChanged();
  assert.equal(ed.systems.at(-1), false);
  host = undefined;
  const n = ed.systems.length;
  h.hostThemeChanged();
  assert.equal(ed.systems.length, n + 1, 'the loss of the signal is delivered');
  assert.equal(ed.systems.at(-1), false, 'no signal at all: the documented Light fallback (as Home resolves it)');
  await h.dispose();
});

// ---- R31: the documented no-signal Light fallback reaches the editor (live and after boot),
// and a kept preference the view learns late (`setAppearance`) reaches the right editor
// without being reported as a new choice or touching the canvas. From the R31 review's probes
// (theme-r31-residual/panel-review-probes.mjs), with their assertions. ----

const r31NoWrite = (t, chosen) => {
  assert.equal(t.srv.named(T.SAVE).length, 0, 'no canvas save');
  assert.equal(t.srv.saved, undefined);
  assert.deepEqual(chosen, [], 'a kept read is not reported as a new menu choice');
};

test('R31 no-signal: losing a dark host fallback resolves the mounted editor to Light', async () => {
  const t = setup();
  let host = 'dark';
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', hostTheme: () => host });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors[0];
  assert.equal(ed.startSystem, true);
  assert.deepEqual(ed.systems, [true]);
  host = null;
  h.hostThemeChanged();
  assert.equal(ed.systems.at(-1), false, 'the old Dark system signal is reset to the Light fallback');
  assert.equal(mod.opdResolveDark(ed.pref, ed.systems.at(-1)), false);
  await h.dispose();
  const n = ed.systems.length;
  h.hostThemeChanged();
  assert.equal(ed.systems.length, n, 'nothing reaches the disposed editor');
});

test('R31 no-signal boot: a dark fallback lost while engine boot is pending is reconciled on completion', async () => {
  const t = setup();
  let host = 'dark';
  const boot = t.engine.start;
  let release;
  const gate = new Promise((r) => { release = r; });
  t.engine.start = async (...args) => { const ed = await boot(...args); await gate; return ed; };
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', hostTheme: () => host });
  await flush();
  assert.equal(t.engine.editors[0].startSystem, true);
  host = null;
  h.hostThemeChanged();
  release();
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors[0];
  assert.equal(ed.systems.at(-1), false, 'post-boot reconciliation resets the no-longer-current Dark');
  await h.dispose();
});

test('R31 no-signal: the browser\'s OS signal still comes first and an explicit mode is untouched', async () => {
  const t = setup();
  const media = scheme(t, true);
  let host = 'light';
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'dark', hostTheme: () => host });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors[0];
  assert.equal(ed.startSystem, true, 'the OS, not the host');
  host = null;
  h.hostThemeChanged();
  assert.equal(ed.systems.at(-1), true, 'losing the host signal changes nothing while the OS says');
  media.matches = false;
  media.fire('change');
  assert.equal(ed.systems.at(-1), false);
  assert.equal(ed.pref, 'dark', 'the explicit Dark preference is the editor\'s own');
  assert.equal(mod.opdResolveDark(ed.pref, ed.systems.at(-1)), true);
  await h.dispose();
});

test('R31 setter: a preference learned during renderer transfer is the first editor appearance', async () => {
  const t = setup();
  let release;
  t.srv.holds[T.ASSET] = new Promise((r) => { release = r; });
  const chosen = [];
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', onAppearance: (a) => chosen.push(a) });
  await flush();
  assert.equal(h.mounted, null);
  h.setAppearance('dark');
  release();
  assert.equal(await h.started, 'mounted');
  assert.equal(t.engine.editors[0].startAppearance, 'dark');
  assert.equal(t.engine.editors[0].pref, 'dark');
  r31NoWrite(t, chosen);
  await h.dispose();
});

test('R31 setter: a preference learned during engine boot reaches that editor after boot, without reporting it', async () => {
  const t = setup();
  const chosen = [];
  const boot = t.engine.start;
  let release;
  const gate = new Promise((r) => { release = r; });
  t.engine.start = async (...args) => { const ed = await boot(...args); await gate; return ed; };
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', onAppearance: (a) => chosen.push(a) });
  await flush();
  assert.ok(h.mounted);
  assert.equal(t.engine.editors[0].startAppearance, 'system');
  h.setAppearance('dark');
  release();
  assert.equal(await h.started, 'mounted');
  assert.equal(t.engine.editors[0].pref, 'dark');
  assert.deepEqual(t.engine.editors[0].kept, ['dark']);
  r31NoWrite(t, chosen);
  await h.dispose();
});

test('R31 setter: a mounted update keeps a draft, the session and its revision; disposal stops later updates', async () => {
  const t = setup();
  const chosen = [];
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', onAppearance: (a) => chosen.push(a) });
  assert.equal(await h.started, 'mounted');
  const ed = t.engine.editors[0];
  ed.isReady = true;
  t.env.frame(16);
  ed.pending = '{"unfinished":"draft"}';
  ed.canReload = false;
  const snapshot = () => ({ text: ed.text, pending: ed.pending, revision: h.mounted.session.state.revision,
    saves: h.mounted.session.state.saves });
  const before = snapshot();
  h.setAppearance('dark');
  assert.equal(ed.pref, 'dark');
  assert.deepEqual(snapshot(), before);
  assert.equal(ed.canReload, false);
  assert.equal(h.mounted.session.closing, false);
  r31NoWrite(t, chosen);
  await h.dispose();
  const calls = ed.kept.length;
  h.setAppearance('light');
  h.mounted.setAppearance('light');
  assert.equal(ed.kept.length, calls);
  assert.equal(ed.freed, true);
});

test('R31 setter: a preference learned while a fallback is pending reaches only its replacement', async () => {
  const t = setup();
  const chosen = [];
  const h = t.panel.open({ binding: BINDING, refs: t.refs, appearance: 'system', onAppearance: (a) => chosen.push(a) });
  assert.equal(await h.started, 'mounted');
  const first = t.engine.editors[0];
  const before = first.kept.length;
  let release;
  t.srv.holds[T.CANVAS] = new Promise((r) => { release = r; });
  first.gpu_error = () => 'WebGPU device lost';
  t.env.frame(16);
  await flush();
  assert.equal(h.mounted.session.state.switching, true);
  h.setAppearance('dark');
  assert.equal(first.kept.length, before, 'not sent to the editor being replaced');
  delete t.srv.holds[T.CANVAS];
  release();
  await flush();
  const next = t.engine.editors.at(-1);
  assert.notEqual(next, first);
  assert.equal(next.webgl, true);
  assert.equal(next.startAppearance, 'dark');
  assert.equal(next.pref, 'dark');
  r31NoWrite(t, chosen);
  await h.dispose();
});

test('an appearance chosen before a WebGL fallback is what the new editor boots in; the editor being replaced is not told', async () => {
  const t = setup();
  const media = scheme(t, false);
  const h = t.panel.open({ binding: BINDING, refs: t.refs, onStatus: () => {}, appearance: 'system' });
  assert.equal(await h.started, 'mounted');
  const first = t.engine.editors.at(-1);
  first.pref = 'dark';
  first.requests.push('appearance');
  t.env.frame(8);
  assert.equal(h.mounted.appearance, 'dark');
  // WebGPU fails before Ready: the session switches to a fresh WebGL editor.
  first.gpu_error = () => 'WebGPU device lost';
  t.env.frame(16);
  media.matches = true;
  media.fire('change');
  assert.deepEqual(first.systems, [false], 'the editor being replaced is not told');
  await flush();
  const second = t.engine.editors.at(-1);
  assert.notEqual(second, first);
  assert.equal(second.webgl, true);
  assert.equal(first.freed, true);
  assert.equal(second.startSystem, true, 'the OS appearance the page has now');
  assert.equal(second.startAppearance, 'dark', 'the choice made before the fallback');
  await h.dispose();
});

// ---- R26-F2: an informational note never replaces an unresolved problem ----
// The actual shared mount, Session and page steps produce the statuses (the mount's frame loop
// reports the terminal error; its clipboard flush reports a refused copy; the save tick reports
// the failed save), into the browser page's `opdStatusNote`. Graphics, DOM and transport are
// stand-ins. These are the R26 review's probes (panel-probes.mjs), with their fixtures and
// assertions; `noteEditor` is theirs.

async function noteEditor() {
  const t = setup();
  const live = { textContent: '' };
  const note = { textContent: '', className: '', hidden: true };
  const noteTimers = new Map();
  let id = 0;
  t.refs.status = mod.opdStatusNote(live, note, {
    setTimeout: (fn, ms) => { noteTimers.set(++id, { fn, ms }); return id; },
    clearTimeout: (n) => noteTimers.delete(n),
  });
  await readyEditor(t);
  return { ...t, live, note, noteTimers };
}

test('R26 control: an ordinary note timer cannot hide the subsequent failure or terminal error', async () => {
  const t = await noteEditor();
  let reject;
  t.env.writeClipboard = () => new Promise((_, j) => { reject = j; });
  t.ed.clip = 'upstream clipboard text';
  t.refs.canvas.fire('keydown', { key: 'c', code: 'KeyC', repeat: false });
  reject(new Error('clipboard permission refused'));
  await flush();
  const staleTimers = [...t.noteTimers.values()];
  assert.equal(t.note.className, 'note');
  t.ed.gpu_error = () => 'device lost';
  t.env.frame(32);
  assert.equal(t.note.className, 'error');
  for (const { fn } of staleTimers) { if (t.noteTimers.size) fn(); }
  assert.equal(t.note.className, 'error');
  assert.equal(t.note.hidden, false);
  await t.h.dispose();
});

test('R26: a late real clipboard rejection must not replace an unresolved terminal renderer error', async () => {
  const t = await noteEditor();
  let reject;
  t.env.writeClipboard = () => new Promise((_, j) => { reject = j; });
  t.ed.clip = 'upstream clipboard text';
  t.refs.canvas.fire('keydown', { key: 'c', code: 'KeyC', repeat: false });
  t.ed.gpu_error = () => 'device lost';
  t.env.frame(32);
  assert.equal(t.h.mounted.session.state.error, 'Renderer error: device lost');
  assert.equal(t.note.className, 'error');
  reject(new Error('clipboard permission refused'));
  await flush();
  // Added: the refused copy is still said, in the live region.
  assert.match(t.live.textContent, /didn't allow copying/);
  await t.h.dispose();
  assert.equal(t.note.className, 'error', 'the unresolved terminal error must stay visible');
  assert.match(t.note.textContent, /Renderer error/);
  assert.equal(t.note.hidden, false);
  assert.equal(t.noteTimers.size, 0, 'and nothing will hide it');
});

test('R26: a late real clipboard rejection must not mask a save failure during its unanswered retry', async () => {
  const t = await noteEditor();
  t.ed.save_failed = () => { t.ed.pending = '{"mine":1}'; };
  t.srv.override[T.SAVE] = () => no(500, 'disk write failed');
  t.ed.pending = '{"mine":1}';
  t.env.every(300);
  await flush();
  assert.equal(t.note.className, 'failure');
  assert.equal(t.note.textContent, 'Save failed; retrying');
  let reject;
  let release;
  t.env.writeClipboard = () => new Promise((_, j) => { reject = j; });
  t.ed.clip = 'upstream clipboard text';
  t.refs.canvas.fire('keydown', { key: 'c', code: 'KeyC', repeat: false });
  t.srv.holds[T.SAVE] = new Promise((r) => { release = r; });
  delete t.srv.override[T.SAVE];
  t.env.every(300);
  await flush();
  assert.equal(t.h.mounted.session.state.saving, true);
  assert.equal(t.note.className, 'failure');
  reject(new Error('clipboard permission refused'));
  await flush();
  t.env.every(300);
  await flush();
  const during = { ...t.note };
  // Added: the refused copy reached the live region; no timer will hide the failure.
  assert.match(t.live.textContent, /didn't allow copying|Saving…/);
  assert.equal(t.noteTimers.size, 0);
  delete t.srv.holds[T.SAVE];
  release();
  await flush();
  assert.equal(t.note.hidden, true, 'the genuine later successful save resolves the latch');
  await t.h.dispose();
  assert.equal(during.className, 'failure', 'the unsaved failure must remain visible until recovery');
  assert.match(during.textContent, /Save failed/);
});
