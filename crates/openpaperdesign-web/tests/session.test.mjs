// Tests for static/session.js with mocked editors, canvases and DOM (no browser, no wasm).
// Run: node --test crates/openpaperdesign-web/tests/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Session } from '../static/session.js';
// `saveStep` (with `discardStep`) is imported once, below, with the page-step tests; module
// imports are hoisted, so the R25-F2 tests here use it too.
import { opdStatusNote } from '../static/editor.js';

/** A mock editor that records calls. */
function editor(name, { gpuError = null, ready = true, snapshot = '{"page":1}' } = {}) {
  return {
    name, calls: [], freed: false, gpuError, isReady: ready, pending: snapshot,
    backend() { return `${name}-backend`; },
    frame() { this.calls.push('frame'); },
    gpu_error() { return this.gpuError; },
    ready() { return this.isReady; },
    set_tool(t) { this.calls.push(`tool:${t}`); },
    resize() { this.calls.push('resize'); },
    save_snapshot() { const s = this.pending; this.pending = undefined; return s; },
    save_succeeded() { this.calls.push('saved'); },
    save_failed() { this.calls.push('save_failed'); },
    dirty() { return this.pending !== undefined; },
    canReload: true,
    can_reload() { return this.canReload && this.pending === undefined; },
    reload(text) { this.calls.push(`reload:${text}`); },
    free() { this.freed = true; },
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((r, j) => { resolve = r; reject = j; });
  return { promise, resolve, reject };
}

// ---- R25-F2: the browser page's status note (app.js uses `opdStatusNote`) through the real
// save tick and session. The mount forwards save statuses as text only (opdMountEditor's
// `ui.status`), so the stand-in `ui` below does the same; adapted from the R25 review's probe.

/** The page's #status (live region) and #alert (the note), and a timer list that never fires. */
function pageNote() {
  const live = { textContent: '' };
  const note = { textContent: '', className: '', hidden: true };
  const timers = [];
  const status = opdStatusNote(live, note, {
    setTimeout: (fn, ms) => { timers.push({ fn, ms }); return timers.length; },
    clearTimeout: () => {},
  });
  return { live, note, timers, status };
}

/** A ready session with one unsaved change, saving through `put`. */
function savingSession(put) {
  const session = new Session({ put, log: () => {} });
  session.ed = { dirty: () => true, save_snapshot: () => '{"pages":[]}', save_failed: () => {}, save_succeeded: () => {} };
  session.state.ready = true;
  return session;
}

test('R25-F2 (browser page): a failed save stays shown through an unanswered retry until a save is acknowledged', async () => {
  const p = pageNote();
  const ui = { status: (text) => p.status(text), conflict: () => {} };
  p.status('Ready');
  assert.equal(p.note.hidden, true, 'healthy: nothing shown');
  const session = savingSession(async () => ({ ok: false, status: 500, revision: null }));
  await saveStep(session, ui);
  assert.equal(p.note.hidden, false);
  assert.equal(p.note.textContent, 'Save failed; retrying');
  assert.equal(p.note.className, 'failure');
  // The retry: its "Saving…" is in the live region, the failure stays on screen.
  const answer = deferred();
  session.deps.put = () => answer.promise;
  const retry = saveStep(session, ui);
  assert.equal(p.live.textContent, 'Saving…');
  assert.equal(p.note.hidden, false, 'last save failure stays visible until a successful save acknowledgement');
  assert.equal(p.note.textContent, 'Save failed; retrying');
  for (const t of p.timers) t.fn();
  assert.equal(p.note.hidden, false, 'no timer hides it');
  answer.resolve({ ok: true, status: 200, revision: '"next"' });
  await retry;
  assert.equal(p.live.textContent, 'Saved');
  assert.equal(p.note.hidden, true, 'the acknowledged save resolves it');
  p.status('Saving…');
  assert.equal(p.note.hidden, true, 'healthy again: Saving… shows nothing');
});

test('R25-F2 (browser page): a persistent or unreachable failure and a conflict stay distinct and shown', async () => {
  const p = pageNote();
  const ui = { status: (text) => p.status(text), conflict: () => {} };
  const session = savingSession(async () => ({ ok: false, status: 500, revision: null }));
  await saveStep(session, ui);
  await saveStep(session, ui);
  assert.equal(p.note.textContent, 'Save failed; retrying', 'still failing');
  session.deps.put = async () => { throw new Error('offline'); };
  await saveStep(session, ui);
  assert.equal(p.note.textContent, 'Save failed (server unreachable); retrying');
  assert.equal(p.note.className, 'failure');
  p.status('Ready');
  assert.equal(p.note.hidden, false, 'Ready doesn\'t resolve an unsaved change');
  // A conflict: not retried; shown until the explicit reload of the version on disk.
  session.deps.put = async () => ({ ok: false, status: 409, revision: '"other"' });
  await saveStep(session, ui);
  assert.equal(p.note.textContent, 'Not saved: conflict');
  assert.equal(p.note.className, 'conflict');
  await saveStep(session, ui);
  p.status('Updated with changes made elsewhere');
  assert.equal(p.note.textContent, 'Not saved: conflict', 'nothing healthy hides it');
  p.status('Reloaded the version on disk');
  assert.equal(p.note.className, 'note', 'the reload is said');
  p.status('Ready');
  assert.equal(p.note.hidden, true, 'resolved');
  // A terminal error stays.
  p.status('The renderer stopped', true);
  p.status('Saved');
  assert.equal(p.note.hidden, false);
  assert.equal(p.note.className, 'error');
});

/** A session with mock deps. `editors` are handed out by boot() in order. */
function setup(editors, { pages = [{ text: 'v1', revision: '"r1"' }], put, revision } = {}) {
  const log = [];
  const observers = [];
  const booted = [];
  const fetched = [...pages];
  let canvasN = 0;
  const deps = {
    boot: async (canvas, page, webgl) => {
      const ed = editors.shift();
      booted.push({ canvas, page, webgl, ed });
      if (ed instanceof Error) throw ed;
      return ed;
    },
    fetchPage: async () => fetched.shift() || { text: 'latest', revision: '"latest"' },
    replaceCanvas: (old) => ({ id: `canvas${++canvasN}`, replaced: old.id }),
    observeResize: (canvas, cb) => {
      const o = { canvas, cb, disconnected: false, disconnect() { this.disconnected = true; } };
      observers.push(o);
      return o;
    },
    resize: (ed) => ed.resize(),
    put: put || (async () => ({ ok: true, status: 200, revision: '"r2"' })),
    revision: revision || (async () => '"r1"'),
    log: (m) => log.push(m),
  };
  const s = new Session(deps);
  return { s, log, observers, booted };
}

test('input, toolbar and saves are ignored until Ready', async () => {
  const ed = editor('gpu', { ready: false });
  let puts = 0;
  const { s } = setup([ed], { put: async () => { puts += 1; return { ok: true, revision: '"x"' }; } });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(s.editable(), false);
  assert.equal(s.edit((e) => { e.set_tool('rect'); return true; }), undefined);
  assert.equal(await s.save(), undefined);
  assert.equal(puts, 0);
  assert.deepEqual(ed.calls, []);
  s.frame(0); // not ready yet
  assert.equal(s.editable(), false);
  ed.isReady = true;
  assert.equal(s.frame(16), 'ready');
  assert.equal(s.edit((e) => { e.set_tool('rect'); return true; }), true);
  assert.equal(await s.save(), 'saved');
  assert.equal(puts, 1);
});

test('a startup failure falls back to WebGL on a fresh canvas with the current server page', async () => {
  const webgl = editor('webgl');
  const { s, log, observers, booted } = setup([new Error('WGSL invalid'), webgl], {
    pages: [{ text: 'server-now', revision: '"r7"' }],
  });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(booted.length, 2);
  assert.equal(booted[1].webgl, true);
  assert.equal(booted[1].canvas.replaced, 'canvas0', 'a fresh canvas');
  assert.equal(booted[1].page.text, 'server-now', 'reloaded from the server');
  assert.equal(s.state.revision, '"r7"');
  assert.equal(s.state.fallback, 'WGSL invalid');
  assert.match(log[0], /falling back to WebGL: WGSL invalid/);
  assert.equal(observers.length, 1);
  assert.equal(observers[0].canvas, s.canvas);
});

test('a GPU error before Ready falls back once; the old observer and editor are retired', async () => {
  const gpu = editor('gpu', { ready: false, gpuError: 'device lost' });
  const webgl = editor('webgl', { ready: false });
  const { s, observers } = setup([gpu, webgl], {
    pages: [{ text: 'server-now', revision: '"r3"' }],
  });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(s.frame(0), 'fallback');
  assert.equal(s.state.switching, true);
  // While switching, nothing touches an editor: frames, resizes, input.
  assert.equal(s.frame(16), undefined);
  observers[0].cb();
  assert.equal(s.edit(() => true), undefined);
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(s.state.switching, false);
  assert.equal(gpu.freed, true);
  assert.equal(observers[0].disconnected, true, 'old observer disconnected');
  assert.equal(observers[1].canvas, s.canvas, 'new observer on the new canvas');
  // The retired observer firing late does nothing to the old (freed) editor.
  observers[0].cb();
  assert.ok(!gpu.calls.includes('resize'));
  observers[1].cb();
  assert.ok(webgl.calls.includes('resize'));
  // A GPU error after the fallback (or after Ready) is an error, not another fallback.
  webgl.gpuError = 'lost again';
  assert.equal(s.frame(32), 'error');
  assert.equal(s.editable(), false);
});

test('a fallback waits for a save in flight and never applies its answer to the new editor', async () => {
  const gpu = editor('gpu', { ready: true });
  const webgl = editor('webgl');
  const answer = deferred();
  const order = [];
  const { s, booted } = setup([gpu, webgl], {
    put: async (snap, rev) => { order.push(`put ${rev}`); return answer.promise; },
    pages: [{ text: 'after-save', revision: '"r2"' }],
  });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  s.frame(0);
  assert.equal(s.editable(), true);
  const saving = s.save();
  // Fallback starts while the save is in flight (defensive: fallbacks only happen before
  // Ready, so this is the worst case).
  const switching = s.fallBack('device lost');
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(booted.length, 1, 'waits for the save before reopening');
  answer.resolve({ ok: true, status: 200, revision: '"r2"' });
  assert.equal(await saving, 'saved');
  assert.ok(gpu.calls.includes('saved'), 'the answer went to the editor that took the snapshot');
  await switching;
  assert.equal(booted.length, 2);
  assert.equal(booted[1].page.text, 'after-save', 'reopened with what the server has now');
  assert.equal(s.state.revision, '"r2"');
  assert.deepEqual(webgl.calls, [], 'nothing applied to the replacement editor');
  assert.deepEqual(order, ['put "r1"']);
});

test('forced WebGL failing is an error, not a fallback', async () => {
  const { s } = setup([new Error('no webgl')]);
  await assert.rejects(s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, true), /no webgl/);
  assert.equal(s.state.fallback, null);
});

test('an answer arriving after the editor was swapped is dropped', async () => {
  const first = editor('first', { ready: true });
  const second = editor('second');
  const answer = deferred();
  const { s } = setup([first], { put: async () => answer.promise });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  s.frame(0);
  const saving = s.save();
  s.ed = second; // whatever replaced it
  answer.resolve({ ok: true, status: 200, revision: '"r9"' });
  assert.equal(await saving, 'stale');
  assert.equal(s.state.revision, '"r1"', 'revision not advanced');
  assert.deepEqual(second.calls, []);
  assert.deepEqual(first.calls.filter((c) => c.startsWith('save')), []);
});

// ---- the page glue (static/page.js) for a fallback that fails

import { frameStep } from '../static/page.js';

function mockUi() {
  const ui = { shown: [], fatals: [], ready() { this.shown.push('Ready'); },
    fatal(m) { this.fatals.push(m); this.shown.push(`FATAL ${m}`); },
    status(t) { this.shown.push(t); }, tick() {} };
  return ui;
}

/** Run frames like app.js does until the loop stops (or `max` frames). */
async function runFrames(s, ui, max = 20) {
  let frames = 0;
  while (frames < max) {
    frames += 1;
    if (!frameStep(s, ui, frames * 16)) return frames;
    await new Promise((r) => setTimeout(r, 0));
  }
  return frames;
}

for (const [what, failing] of [
  ['the page fetch fails', (deps) => { deps.fetchPage = async () => { throw new Error('page fetch: HTTP 500'); }; }],
  ['WebGL init fails', (deps) => { deps.boot = async () => { throw new Error('no WebGL2 context'); }; }],
]) {
  test(`a fallback that fails (${what}) ends in a visible red error and stops the frame loop`, async () => {
    const gpu = editor('gpu', { ready: false, gpuError: 'WGSL invalid' });
    const { s } = setup([gpu]);
    await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
    failing(s.deps);
    const ui = mockUi();
    const frames = await runFrames(s, ui);
    assert.ok(frames < 20, 'the frame loop stopped');
    assert.ok(ui.shown.includes('WebGPU failed; switching to WebGL…'));
    assert.equal(ui.fatals.length, 1, 'the error is shown once');
    assert.match(ui.fatals[0], /Renderer failed: .*(HTTP 500|no WebGL2 context)/);
    assert.equal(ui.shown.at(-1), `FATAL ${ui.fatals[0]}`, 'not left at "switching"');
    assert.equal(s.state.switching, false);
    assert.equal(s.editable(), false);
    // Later frames stay stopped and quiet.
    assert.equal(frameStep(s, ui, 999), false);
    assert.equal(ui.fatals.length, 1);
  });
}

test('the page glue shows Ready once and keeps the loop running', async () => {
  const ed = editor('gpu', { ready: false });
  const { s } = setup([ed]);
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  const ui = mockUi();
  assert.equal(frameStep(s, ui, 0), true);
  ed.isReady = true;
  assert.equal(frameStep(s, ui, 16), true);
  assert.equal(frameStep(s, ui, 32), true);
  assert.deepEqual(ui.shown, ['Ready']);
  // A GPU error after Ready is terminal: shown once, loop stops.
  ed.gpuError = 'device lost';
  assert.equal(frameStep(s, ui, 48), false);
  assert.deepEqual(ui.fatals, ['Renderer error: device lost']);
  assert.equal(frameStep(s, ui, 64), false);
  assert.equal(ui.fatals.length, 1);
});

// ---- resizing: the canvas is never painted cleared between animation frames ----
// A model of the canvas as the page shows it: a new backing size (the editor's surface
// reconfigure in `resize`) clears it; `frame` draws it again when something changed (the
// real WebEditor's redraw rule). The browser runs animation frames, then resize observers,
// then paints. What is painted is checked after each step's resize callbacks.

/** A mock editor whose canvas is cleared by a new size and drawn by a frame. */
function canvasEditor(name = 'gpu') {
  const ed = editor(name);
  ed.size = [960, 760];
  ed.shown = 'drawn 960x760';
  ed.redraw = false;
  ed.frames = [];
  ed.resize = (w, h) => {
    ed.calls.push(`resize ${w}x${h}`);
    if (w !== ed.size[0] || h !== ed.size[1]) {
      ed.size = [w, h];
      ed.shown = 'cleared';
    }
    ed.redraw = true;
  };
  ed.frame = (t) => {
    ed.calls.push(`frame ${t}`);
    ed.frames.push(t);
    if (ed.redraw && !ed.gpuError) {
      ed.shown = `drawn ${ed.size[0]}x${ed.size[1]}`;
      ed.redraw = false;
    }
  };
  return ed;
}

/** A Ready session on `ed`, resizing to `size.w` × `size.h`, after one animation frame. */
async function resizingSession(ed) {
  const size = { w: 960, h: 760 };
  const { s, observers } = setup([ed]);
  s.deps.resize = (e) => e.resize(size.w, size.h);
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(s.frame(16), 'ready');
  ed.calls.length = 0;
  return { s, observers, size };
}

test('a resize draws the reconfigured canvas at once, in the same task, at the last frame\'s time', async () => {
  const ed = canvasEditor();
  const { observers, size } = await resizingSession(ed);
  size.w = 900;
  observers[0].cb();
  assert.deepEqual(ed.calls, ['resize 900x760', 'frame 16'], 'reconfigured, then drawn before the page paints');
  assert.equal(ed.shown, 'drawn 900x760', 'what the page paints next is the new picture');
});

test('a sustained resize in both directions, across the narrow breakpoint, never paints a cleared canvas', async () => {
  const ed = canvasEditor();
  const { s, observers, size } = await resizingSession(ed);
  // A divider dragged from 960 down to 300 (below the 720 breakpoint) and back, one step per
  // animation frame, with the occasional frame of no movement and two steps in one frame.
  const widths = [];
  for (let w = 960; w >= 300; w -= 24) widths.push(w);
  for (let w = 300; w <= 960; w += 36) widths.push(w);
  const painted = [];
  let t = 16;
  for (const [i, w] of widths.entries()) {
    t += 16;
    s.frame(t); // the animation frame
    size.w = w;
    observers[0].cb(); // resize observers run
    if (i % 7 === 3) { size.w = w - 10; observers[0].cb(); } // two steps in one frame
    painted.push(ed.shown); // then the page paints
    if (i % 5 === 4) { t += 16; s.frame(t); painted.push(ed.shown); } // a frame of no movement
  }
  assert.deepEqual(painted.filter((p) => !p.startsWith('drawn')), [], 'no cleared canvas is ever painted');
  assert.equal(painted.at(-1), `drawn ${size.w}x760`, 'the last picture is at the final size');
  assert.ok(widths.some((w) => w < 720) && widths.some((w) => w > 720), 'the burst crosses the narrow breakpoint');
  assert.equal(s.editable(), true, 'still editing');
  assert.equal(s.state.error, null);
});

test('a resize before the first frame, after a terminal error, while switching or after dispose draws nothing', async () => {
  // Before any animation frame: start presented its own frame; the loop draws the first.
  const early = canvasEditor();
  const { s: s0, observers: o0 } = setup([early]);
  s0.deps.resize = (e) => e.resize(800, 600);
  await s0.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  o0[0].cb();
  assert.deepEqual(early.calls, ['resize 800x600'], 'no frame before the loop\'s first');
  // After a terminal error: it stays the editor's last word; the frame loop has stopped.
  const failing = canvasEditor();
  const { s, observers, size } = await resizingSession(failing);
  failing.gpuError = 'device lost';
  assert.equal(s.frame(32), 'error');
  failing.calls.length = 0;
  size.w = 700;
  observers[0].cb();
  assert.deepEqual(failing.calls, ['resize 700x760'], 'resized, not drawn over the error');
  assert.equal(s.frame(48), 'stopped');
  // Disposed: the observer's late call reaches nothing.
  const gone = canvasEditor();
  const d = await resizingSession(gone);
  d.s.dispose();
  gone.calls.length = 0;
  d.size.w = 640;
  d.observers[0].cb();
  assert.deepEqual(gone.calls, []);
});

test('a resize during a drain (input stopped) still redraws, and the drain\'s outcome is unchanged', async () => {
  const ed = canvasEditor();
  ed.pending = undefined; // nothing unsaved
  const { s, observers, size } = await resizingSession(ed);
  const leaving = s.drain();
  size.w = 500;
  observers[0].cb();
  assert.equal(ed.shown, 'drawn 500x760', 'the picture follows the surface while leaving');
  assert.equal(s.edit(() => true), undefined, 'input stays stopped');
  assert.equal(await leaving, 'clean');
});

// ---- a terminal error while a save is in flight (static/page.js saveStep + frameStep)

import { discardStep, saveStep } from '../static/page.js';

for (const [outcome, settle, check] of [
  ['success', (d) => d.resolve({ ok: true, status: 200, revision: '"r2"' }),
    (s, ed) => { assert.ok(ed.calls.includes('saved')); assert.equal(s.state.revision, '"r2"'); }],
  ['409 conflict', (d) => d.resolve({ ok: false, status: 409, conflicts: ['3'] }),
    (s, ed) => { assert.ok(ed.calls.includes('save_failed')); assert.equal(s.state.conflict, true); }],
  ['HTTP failure', (d) => d.resolve({ ok: false, status: 500 }),
    (s, ed) => { assert.ok(ed.calls.includes('save_failed')); }],
  ['network rejection', (d) => d.reject(new Error('connection reset')),
    (s, ed) => { assert.ok(ed.calls.includes('save_failed')); }],
]) {
  test(`a save in flight when the renderer fails (${outcome}) keeps the red terminal status`, async () => {
    const ed = editor('gpu', { ready: true });
    const answer = deferred();
    const { s } = setup([ed], { put: async () => answer.promise });
    await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
    const ui = mockUi();
    ui.conflicts = [];
    ui.conflict = (ids) => ui.conflicts.push(ids);
    assert.equal(frameStep(s, ui, 0), true); // Ready
    const saving = saveStep(s, ui);          // PUT in flight
    assert.equal(ui.shown.at(-1), 'Saving…');
    ed.gpuError = 'device lost';             // terminal while waiting
    assert.equal(frameStep(s, ui, 16), false);
    const fatal = ui.shown.at(-1);
    assert.match(fatal, /^FATAL Renderer error: device lost/);
    settle(answer);
    await saving;
    assert.equal(ui.shown.at(-1), fatal, `"${outcome}" must not replace the terminal status`);
    assert.ok(!ui.shown.slice(ui.shown.indexOf(fatal) + 1).length, 'nothing shown after it');
    check(s, ed);                            // bookkeeping still done
    if (outcome === '409 conflict') assert.equal(ui.conflicts.length, 1, 'banner still shown');
    assert.equal(await saveStep(s, ui), undefined, 'no further saves once terminal');
    assert.equal(ui.shown.at(-1), fatal);
  });
}

/** A Ready session on page revision r1, with a server whose revision/page can be changed. */
async function readySession({ put } = {}) {
  const ed = editor('gpu');
  ed.pending = undefined; // nothing unsaved
  const server = { revision: '"r1"', text: 'v1' };
  const { s } = setup([ed], {
    put,
    revision: async () => server.revision,
    pages: [{ text: 'v1', revision: '"r1"' }],
  });
  s.deps.fetchPage = async () => ({ text: server.text, revision: server.revision });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  s.frame(0);
  assert.equal(s.editable(), true);
  return { s, ed, server };
}

test('a change made elsewhere is loaded in place when nothing is unsaved', async () => {
  const { s, ed, server } = await readySession();
  assert.equal(await s.poll(), undefined, 'no change');
  server.revision = '"r2"';
  server.text = 'agent-change';
  assert.equal(await s.poll(), 'reloaded');
  assert.deepEqual(ed.calls.filter((c) => c.startsWith('reload')), ['reload:agent-change']);
  assert.equal(s.state.revision, '"r2"');
});

test('never reloads over unsaved edits; the stale save is a conflict and the edit stays', async () => {
  let sentWith = null;
  const { s, ed, server } = await readySession({
    put: async (snap, rev) => { sentWith = rev; return { ok: false, status: 409, revision: '"r2"' }; },
  });
  ed.pending = '{"mine":1}'; // an unsaved local edit
  server.revision = '"r2"';  // an agent wrote meanwhile
  server.text = 'agent-change';
  assert.equal(await s.poll(), undefined, 'a dirty editor does not poll');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')), 'unsaved work not replaced');
  assert.equal(await s.save(), 'conflict');
  assert.equal(sentWith, '"r1"', 'saved against the revision it was based on');
  assert.ok(ed.calls.includes('save_failed'), 'the edit stays unsaved, on screen');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  assert.equal(s.state.revision, '"r1"');
  // Stopped until the person decides: no silent reload, no further save.
  assert.equal(await s.poll(), undefined);
  assert.equal(await s.save(), undefined);
});

/** A mocked page for discardStep: records what is shown. */
function discardUi() {
  return { shown: [], hidden: 0, status(t) { this.shown.push(t); }, hideConflict() { this.hidden += 1; } };
}

test('explicit discard loads the version on disk in place and resumes saving', async () => {
  const { s, ed, server } = await readySession({
    put: async () => ({ ok: false, status: 409 }),
  });
  ed.pending = '{"mine":1}';
  server.revision = '"r2"';
  server.text = 'agent-change';
  assert.equal(await s.save(), 'conflict');
  const ui = discardUi();
  assert.equal(await discardStep(s, ui), 'discarded');
  assert.ok(ed.calls.includes('reload:agent-change'), "the agent's version is shown");
  assert.equal(s.state.revision, '"r2"');
  assert.equal(s.state.conflict, false);
  assert.equal(ui.hidden, 1, 'banner hidden');
  assert.deepEqual(ui.shown, ['Reloaded the version on disk']);
  // Saving resumes from the new revision.
  let sentWith = null;
  s.deps.put = async (snap, rev) => { sentWith = rev; return { ok: true, status: 200, revision: '"r3"' }; };
  ed.pending = '{"next":1}';
  assert.equal(await s.save(), 'saved');
  assert.equal(sentWith, '"r2"');
});

test('a discard answer is dropped when the editor changed meanwhile, and a failed one changes nothing', async () => {
  const { s, ed } = await readySession({ put: async () => ({ ok: false, status: 409 }) });
  ed.pending = '{"mine":1}';
  assert.equal(await s.save(), 'conflict');
  const page = deferred();
  s.deps.fetchPage = async () => page.promise;
  const ui = discardUi();
  const discarding = discardStep(s, ui);
  s.ed = editor('replacement'); // whatever replaced it
  page.resolve({ text: 'late', revision: '"r9"' });
  assert.equal(await discarding, undefined);
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  assert.equal(s.state.revision, '"r1"');
  assert.equal(ui.hidden, 0);
  // A fetch failure keeps the banner, the conflict and the unsaved state.
  s.ed = ed;
  s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await discardStep(s, ui), undefined);
  assert.equal(s.state.conflict, true);
  assert.equal(ui.hidden, 0);
  assert.deepEqual(ui.shown, ['Could not load the version on disk; try again']);
});

test('no polling before Ready or while switching', async () => {
  const ed = editor('gpu', { ready: false });
  let asked = 0;
  const { s } = setup([ed], { revision: async () => { asked += 1; return '"r2"'; } });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(await s.poll(), undefined);
  assert.equal(asked, 0);
});

test('a late poll answer never replaces a newer saved state (reviewer sequence)', async () => {
  const fetched = deferred();
  const { s, ed, server } = await readySession({
    put: async () => ({ ok: true, status: 200, revision: '"r3"' }),
  });
  server.revision = '"r2"';                 // someone else saved r2
  s.deps.fetchPage = () => fetched.promise; // the poll's fetch is still pending...
  const poll = s.poll();
  await new Promise((r) => setTimeout(r, 0));
  ed.pending = '{"human":1}';               // ...while a human edit is saved as r3
  assert.equal(await s.save(), 'saved');
  assert.equal(s.state.revision, '"r3"');
  fetched.resolve({ text: 'older r2 page', revision: '"r2"' });
  await poll;
  assert.ok(!ed.calls.includes('reload:older r2 page'), 'the older page was not applied');
  assert.equal(s.state.revision, '"r3"', 'no rollback');
});

test('polls never overlap, and a discard makes an older poll answer stale', async () => {
  const fetched = deferred();
  const { s, ed, server } = await readySession();
  server.revision = '"r2"';
  s.deps.fetchPage = () => fetched.promise;
  const first = s.poll();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(await s.poll(), undefined, 'a second poll does not start while one is pending');
  // A discard starts after the poll (e.g. the person clicked it): the poll's answer is stale.
  s.state.conflict = true;
  s.deps.fetchPage = async () => ({ text: 'discarded-to', revision: '"r4"' });
  assert.equal(await s.discard(), 'discarded');
  fetched.resolve({ text: 'stale poll page', revision: '"r2"' });
  await first;
  assert.ok(!ed.calls.includes('reload:stale poll page'));
  assert.equal(s.state.revision, '"r4"');
});

test('editing is suspended while a discard loads, so it cannot erase a newer edit (reviewer sequence)', async () => {
  const { s, ed } = await readySession({ put: async () => ({ ok: false, status: 409 }) });
  ed.pending = '{"mine":1}';
  assert.equal(await s.save(), 'conflict');
  const page = deferred();
  s.deps.fetchPage = async () => page.promise;
  const discarding = s.discard();
  // The person types while the server's version is loading: the edit never reaches the
  // editor (input is suspended), so the discard can't silently replace it.
  assert.equal(s.editable(), false);
  let applied = false;
  assert.equal(s.edit(() => { applied = true; }), undefined);
  assert.equal(applied, false);
  assert.equal(await s.save(), undefined, 'no save either');
  assert.equal(await s.discard(), undefined, 'one discard at a time');
  page.resolve({ text: 'server page', revision: '"r2"' });
  assert.equal(await discarding, 'discarded');
  assert.ok(ed.calls.includes('reload:server page'));
  // Editing resumes once it is done, also after a failed load.
  assert.equal(s.editable(), true);
  assert.equal(s.edit(() => 'ok'), 'ok');
  s.state.conflict = true;
  s.deps.fetchPage = async () => { throw new Error('offline'); };
  await assert.rejects(s.discard());
  assert.equal(s.editable(), true);
  assert.equal(s.state.conflict, true, 'the conflict and unsaved work stay');
});

test('only a clean editor polls: no revision request while dirty, in a gesture or a text edit (review round 3)', async () => {
  let requests = 0;
  const { s, ed, server } = await readySession();
  s.deps.revision = async () => { requests += 1; return server.revision; };
  server.revision = '"r2"';
  // Unsaved work.
  ed.pending = '{"mine":1}';
  assert.equal(await s.poll(), undefined);
  // A gesture or text edit under way (the editor reports it can't reload).
  ed.pending = undefined;
  ed.canReload = false;
  assert.equal(await s.poll(), undefined);
  assert.equal(requests, 0, 'no revision request was made');
  // Clean again: it polls and loads the change.
  ed.canReload = true;
  assert.equal(await s.poll(), 'reloaded');
  assert.equal(requests, 1);
});

test('an edit made while a clean poll is under way is never replaced', async () => {
  const { s, ed, server } = await readySession();
  const rev = deferred();
  s.deps.revision = async () => rev.promise;
  const polling = s.poll();
  ed.pending = '{"mine":1}';
  server.revision = '"r2"';
  rev.resolve('"r2"');
  assert.equal(await polling, 'behind');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
});

// ---- leaving (drain) and disposal: the in-panel editor's Back, Close and teardown

const tick = () => new Promise((r) => setTimeout(r, 0));

test('disposal while the renderer boots: the late editor is freed at once and gets nothing', async () => {
  const late = editor('late');
  const booting = deferred();
  const observed = [];
  const { s } = setup([]);
  s.deps.boot = async () => booting.promise;
  s.deps.observeResize = (c) => { observed.push(c); return { disconnect() {} }; };
  const starting = s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  s.dispose();
  booting.resolve(late);
  await starting;
  assert.equal(late.freed, true, 'freed as soon as it arrived');
  assert.equal(s.ed, null);
  assert.deepEqual(observed, [], 'no resize observer for it');
  assert.equal(s.frame(0), 'stopped', 'no frame, and so no Ready');
  assert.deepEqual(late.calls, []);
  assert.equal(await s.save(), undefined);
  assert.equal(await s.saveNow(), undefined);
});

test('disposal while the renderer fails to boot: no fallback starts', async () => {
  const booting = deferred();
  const { s, booted } = setup([]);
  let fetched = 0;
  s.deps.boot = async (canvas, page, webgl) => { booted.push({ webgl }); return booting.promise; };
  s.deps.fetchPage = async () => { fetched += 1; return { text: 'x', revision: '"x"' }; };
  const starting = s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  s.dispose();
  booting.reject(new Error('WebGPU unavailable'));
  await starting;
  assert.equal(booted.length, 1, 'no WebGL boot after disposal');
  assert.equal(fetched, 0, 'no page fetch for a fallback');
  assert.equal(s.state.fallback, null);
});

for (const stage of ['save wait', 'page fetch', 'WebGL boot']) {
  test(`disposal during a fallback (${stage}): no canvas, observer, editor or Ready afterwards`, async () => {
    const gpu = editor('gpu', { ready: false, gpuError: 'device lost' });
    const webgl = editor('webgl');
    const gate = deferred();
    const { s, observers } = setup([gpu, webgl]);
    let replaced = 0;
    s.deps.replaceCanvas = (old) => { replaced += 1; return { id: 'fresh', replaced: old.id }; };
    const realBoot = s.deps.boot;
    await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
    // Held at the stage under test (after the first, WebGPU boot).
    if (stage === 'page fetch') s.deps.fetchPage = async () => { await gate.promise; return { text: 'p', revision: '"p"' }; };
    if (stage === 'WebGL boot') s.deps.boot = async (...a) => { const ed = await realBoot(...a); await gate.promise; return ed; };
    if (stage === 'save wait') s.saveDone = gate.promise;
    assert.equal(s.frame(0), 'fallback');
    await tick();
    s.dispose();
    assert.equal(gpu.freed, true, 'the editor in use is freed by the disposal');
    assert.equal(observers[0].disconnected, true);
    gate.resolve();
    await tick();
    await tick();
    if (stage === 'WebGL boot') {
      assert.equal(webgl.freed, true, 'the late WebGL editor is freed');
      assert.equal(replaced, 1, 'the canvas was replaced before the disposal, not after');
    } else {
      assert.equal(replaced, 0, 'no canvas replaced after the disposal');
    }
    assert.equal(observers.length, 1, 'no observer for a late editor');
    assert.equal(s.ed, null);
    assert.equal(s.frame(16), 'stopped');
    assert.equal(s.state.ready, false);
    assert.equal(s.state.error, null, 'a disposed session reports no error either');
  });
}

test('disposal during a poll or a discard: their late answers change nothing', async () => {
  const { s, ed, server } = await readySession({ put: async () => ({ ok: false, status: 409 }) });
  const rev = deferred();
  s.deps.revision = async () => rev.promise;
  server.revision = '"r2"';
  const polling = s.poll();
  s.dispose();
  rev.resolve('"r2"');
  assert.equal(await polling, undefined);
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  assert.equal(ed.freed, true);
  // A discard that was loading.
  const b = await readySession({ put: async () => ({ ok: false, status: 409 }) });
  b.ed.pending = '{"mine":1}';
  assert.equal(await b.s.save(), 'conflict');
  const page = deferred();
  b.s.deps.fetchPage = async () => page.promise;
  const discarding = b.s.discard();
  b.s.dispose();
  page.resolve({ text: 'late', revision: '"r9"' });
  assert.equal(await discarding, undefined);
  assert.ok(!b.ed.calls.some((c) => c.startsWith('reload')));
});

test('disposal during a save: its answer is not applied and never shown as saved', async () => {
  const answer = deferred();
  const { s, ed } = await readySession({ put: async () => answer.promise });
  ed.pending = '{"mine":1}';
  const saving = s.save();
  s.dispose();
  answer.resolve({ ok: true, status: 200, revision: '"r2"' });
  assert.equal(await saving, 'stale');
  assert.ok(!ed.calls.includes('saved'), 'no acknowledgement reached a freed editor');
  assert.equal(s.state.saves, 0);
});

test('drain waits for the save in flight, then saves the edit made meanwhile, and only then is clean', async () => {
  const answers = [deferred(), deferred()];
  const sent = [];
  const { s, ed } = await readySession({
    put: async (snap, rev) => { sent.push([snap, rev]); return answers[sent.length - 1].promise; },
  });
  ed.pending = '{"a":1}';
  const saving = s.save();                 // in flight
  ed.pending = '{"a":1,"b":2}';            // a completed edit after that snapshot, before the next tick
  const draining = s.drain();
  assert.equal(s.editable(), false, 'no input while leaving');
  let typed = false;
  s.edit(() => { typed = true; });
  assert.equal(typed, false);
  answers[0].resolve({ ok: true, status: 200, revision: '"r2"' });
  await saving;
  await tick();
  assert.deepEqual(sent.map(([, r]) => r), ['"r1"', '"r2"'], 'the rest is saved on the new revision');
  answers[1].resolve({ ok: true, status: 200, revision: '"r3"' });
  assert.equal(await draining, 'clean');
  assert.equal(s.state.revision, '"r3"');
  assert.equal(await s.poll(), undefined, 'no polling while leaving');
});

test('drain is never clean with a conflict, a failed or unanswered save, an edit under way or a stopped renderer', async () => {
  for (const [why, prepare] of [
    ['conflict', (t) => { t.s.deps.put = async () => ({ ok: false, status: 409 }); t.ed.pending = '{"m":1}'; }],
    ['failed', (t) => { t.s.deps.put = async () => ({ ok: false, status: 500 }); t.ed.pending = '{"m":1}'; }],
    ['unreachable', (t) => { t.s.deps.put = async () => { throw new Error('no answer'); }; t.ed.pending = '{"m":1}'; }],
    ['busy', (t) => { t.ed.canReload = false; }],
    ['error', (t) => { t.ed.pending = '{"m":1}'; t.s.fail('Renderer error: lost'); }],
  ]) {
    const t = await readySession();
    prepare(t);
    assert.equal(await t.s.drain(), why, why);
    assert.equal(t.ed.freed, false, `${why}: the editor stays`);
    t.s.resume();
    if (why !== 'error') assert.equal(t.s.editable(), true, `${why}: editing goes on after resume`);
  }
  // No snapshot during a gesture is not "clean".
  const g = await readySession();
  g.ed.canReload = false;
  assert.equal(g.ed.save_snapshot(), undefined);
  assert.equal(await g.s.drain(), 'busy');
});

test('a stopped renderer is judged by what it holds: never Ready or nothing unsaved is clean, unsaved work is not', async () => {
  // Failed before it was ever Ready: no edit was possible.
  const early = editor('gpu', { ready: false });
  early.pending = '{"never":1}';
  const a = setup([early]);
  await a.s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  a.s.fail('Renderer failed: no adapter');
  assert.equal(await a.s.drain(), 'clean');
  // Ready, then stopped with nothing unsaved.
  const b = await readySession();
  b.s.fail('Renderer error: lost');
  assert.equal(b.s.state.ready, false, 'the error cleared ready');
  assert.equal(await b.s.drain(), 'clean');
  // Ready, then stopped with unsaved work (or an edit under way): never clean.
  const c = await readySession();
  c.ed.pending = '{"m":1}';
  c.s.fail('Renderer error: lost');
  assert.equal(await c.s.drain(), 'error');
  const d = await readySession();
  d.ed.canReload = false;
  d.s.fail('Renderer error: lost');
  assert.equal(await d.s.drain(), 'error');
  assert.equal(c.ed.freed, false);
  assert.equal(d.ed.freed, false);
});

test('drain before Ready is clean (nothing could be edited); dispose is once and idempotent', async () => {
  const ed = editor('gpu', { ready: false });
  const { s, observers } = setup([ed]);
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(await s.drain(), 'clean');
  let frees = 0;
  ed.free = () => { frees += 1; };
  s.dispose();
  s.dispose();
  assert.equal(frees, 1);
  assert.equal(observers[0].disconnected, true);
  s.resume();
  assert.equal(s.editable(), false, 'a disposed session never resumes');
});
