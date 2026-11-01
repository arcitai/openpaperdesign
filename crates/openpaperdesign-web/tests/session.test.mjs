// Tests for static/session.js with mocked editors, canvases and DOM (no browser, no wasm).
// Run: node --test crates/openpaperdesign-web/tests/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Session } from '../static/session.js';
// Test-only: the exact run-192 Session (tests/legacy/README.md), an older editor's reader.
import { Session as Session192 } from './legacy/session-192.mjs';
import { Session as Session193 } from './legacy/session-193.mjs';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
// Issue #23's Edit here, with the read-only tests at the end.
import { claimStep, offerStep, OPD_EDITING_STATUS } from '../static/page.js';
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
  assert.equal(await s.poll(), 'behind', 'a dirty editor learns of the change (issue #23: it polls)');
  assert.equal(await s.poll(), undefined, 'said once');
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

test('only a clean editor loads: while dirty, in a gesture or a text edit it polls (issue #23, 23-B) but never reloads (review round 3)', async () => {
  let requests = 0;
  const { s, ed, server } = await readySession();
  s.deps.revision = async () => { requests += 1; return server.revision; };
  let fetches = 0;
  const fetchPage = s.deps.fetchPage;
  s.deps.fetchPage = async () => { fetches += 1; return fetchPage(); };
  server.revision = '"r2"';
  // Unsaved work: the change is learned once, nothing is loaded.
  ed.pending = '{"mine":1}';
  assert.equal(await s.poll(), 'behind');
  // A gesture or text edit under way (the editor reports it can't reload).
  ed.pending = undefined;
  ed.canReload = false;
  assert.equal(await s.poll(), undefined, 'already said');
  assert.equal(requests, 2, 'the revision (and editing) is asked in every state');
  assert.equal(fetches, 0, 'nothing was loaded');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  // Clean again: it loads the change.
  ed.canReload = true;
  assert.equal(await s.poll(), 'reloaded');
  assert.equal(requests, 3);
  assert.equal(s.state.behind, false);
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

// ---- issue #23: the same canvas in several views; the one that opened it last edits it.

test('issue #23: a view that lost the editing keeps its change unsaved, tries no save, still sees saves made there, and Edit here gives a conflict instead of an overwrite', async () => {
  const ed = editor('gpu');
  const change = ed.pending;
  // As the real editor: a failed save leaves the change unsaved.
  ed.save_failed = () => { ed.calls.push('save_failed'); ed.pending = change; };
  const server = { revision: '"r1"', editing: true, text: 'v1' };
  const puts = [];
  const { s } = setup([ed], {
    put: async (snap, rev) => {
      puts.push(rev);
      if (!server.editing) return { ok: false, status: 423, revision: null };
      if (rev !== server.revision) return { ok: false, status: 409, revision: server.revision };
      server.revision = '"mine"';
      return { ok: true, status: 200, revision: '"mine"' };
    },
    revision: async () => ({ revision: server.revision, editing: server.editing }),
  });
  s.deps.fetchPage = async () => ({ text: server.text, revision: server.revision, editing: server.editing });
  s.deps.claim = async () => { server.editing = true; return { ok: true }; };
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"', editing: true }, false);
  assert.equal(s.frame(0), 'ready');
  assert.equal(s.editable(), true);
  // Another view opened the canvas: this one's save is refused, nothing written (423).
  server.editing = false;
  assert.equal(await s.save(), 'readonly');
  assert.equal(s.state.readonly, true);
  assert.ok(ed.calls.includes('save_failed') && !ed.calls.includes('saved'), 'never shown as saved');
  assert.equal(ed.can_reload(), false, 'the change is still in the editor');
  assert.equal(s.editable(), false, 'no input while read-only');
  // No save is tried while read-only, and leaving says why instead of reporting clean.
  const tried = puts.length;
  assert.equal(await s.save(), undefined);
  assert.equal(await s.saveNow(), undefined);
  assert.equal(puts.length, tried);
  assert.equal(await s.drain(), 'readonly');
  s.resume();
  // A save made in the editing view: seen, and nothing is loaded over the change here.
  server.revision = '"theirs"';
  server.text = 'theirs';
  assert.equal(await s.poll(), 'behind');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')), 'the change stays');
  // Edit here: editing again. The next save, on the revision this view had, is a conflict.
  assert.equal(await s.claim(), 'editing');
  assert.equal(s.state.readonly, false);
  assert.equal(await s.save(), 'conflict');
  assert.equal(puts.at(-1), '"r1"');
  assert.equal(server.revision, '"theirs"', 'nothing overwritten');
  assert.equal(ed.can_reload(), false, 'and the change is still here');
});

test('issue #23: a clean read-only view keeps showing what the editing view saves and learns when it edits again', async () => {
  const ed = editor('gpu');
  ed.pending = undefined; // clean: nothing unsaved
  const server = { revision: '"r1"', editing: false, text: 'v1' };
  const { s } = setup([ed], { revision: async () => ({ revision: server.revision, editing: server.editing }) });
  s.deps.fetchPage = async () => ({ text: server.text, revision: server.revision, editing: server.editing });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"', editing: false }, false);
  assert.equal(s.frame(0), 'ready');
  assert.equal(s.state.readonly, true, 'opened while another view edits');
  assert.equal(s.edit(() => true), undefined, 'no input');
  server.revision = '"r2"';
  server.text = 'v2';
  assert.equal(await s.poll(), 'reloaded');
  assert.ok(ed.calls.includes('reload:v2'));
  assert.equal(s.state.revision, '"r2"');
  server.editing = true;
  assert.equal(await s.poll(), 'editing');
  assert.equal(s.editable(), true);
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  assert.equal(s.editable(), false);
  // An older server says nothing about editing: a plain revision changes nothing of it.
  s.deps.revision = async () => '"r2"';
  assert.equal(await s.poll(), undefined);
  assert.equal(s.state.readonly, true);
});

test('issue #23: the save tick says read-only until Edit here; a refused Edit here changes nothing', async () => {
  const p = pageNote();
  let shown = 0;
  let hidden = 0;
  const ui = {
    status: (text) => p.status(text), conflict: () => {},
    readonly: () => { shown += 1; }, hideReadonly: () => { hidden += 1; },
  };
  const session = savingSession(async () => ({ ok: false, status: 423, revision: null }));
  assert.equal(await saveStep(session, ui), 'readonly');
  assert.equal(shown, 1);
  assert.equal(p.note.textContent, 'Read-only: another view is editing this canvas');
  assert.equal(p.note.className, 'conflict');
  p.status('Ready');
  assert.equal(p.note.hidden, false, 'shown until this view edits again');
  session.deps.claim = async () => ({ ok: false, error: 'busy' });
  assert.equal(await claimStep(session, ui), 'refused');
  assert.equal(session.state.readonly, true);
  assert.equal(hidden, 0);
  session.deps.claim = async () => ({ ok: true });
  assert.equal(await claimStep(session, ui), 'editing');
  assert.equal(session.state.readonly, false);
  assert.equal(hidden, 1);
  assert.equal(p.live.textContent, 'Editing here again');
  p.status('Ready');
  assert.equal(p.note.hidden, true, 'resolved');
});

// ---- issue #23 (R78-03): a view with an unsaved, unfinished, mixed or conflicting edit learns
// that it lost the editing and hands it over; the editing view recovers it.

/**
 * An editor that behaves as the real renderer does around saves and the handover (state.rs):
 * a finished change (`pending`) and an open rename field (`field`) are independent, so a
 * snapshot is taken with a field still open (the mixed case R78 names), while a gesture or a
 * text edit (`gesture`, `typing`) refuses one; `handover()` reports the change and every
 * unfinished edit side by side, without touching anything; `resume_unfinished` takes one up;
 * `reload` closes open fields. `history` stands for the undo history, which nothing may touch.
 */
function realEditor() {
  const ed = editor('gpu');
  Object.assign(ed, {
    pending: undefined, field: null, gesture: false, typing: null, inflight: false,
    history: ['h1'], resumed: [], doc: 'v1',
    // Names of other kinds taken up before `field` (the adapter keeps a Canvas, a Paper and a
    // layer name open at once, each its own draft: run 196).
    held: [],
    save_snapshot() {
      if (this.inflight || this.gesture || this.typing || this.pending === undefined) return undefined;
      this.inflight = true;
      return this.pending;
    },
    save_failed() { this.inflight = false; this.calls.push('save_failed'); },
    save_succeeded() { this.inflight = false; this.pending = undefined; this.calls.push('saved'); },
    can_reload() {
      return !this.inflight && !this.gesture && !this.typing && !this.field && !this.held.length
        && this.pending === undefined;
    },
    dirty() { return this.pending !== undefined; },
    reload(text) {
      this.calls.push(`reload:${text}`); this.doc = text; this.field = null; this.held = []; this.pending = undefined;
    },
    handover() {
      const unfinished = [...this.held];
      if (this.field) unfinished.push(this.field);
      if (this.typing) unfinished.push(this.typing);
      if (this.gesture) unfinished.push({ kind: 'gesture' });
      return JSON.stringify({
        canvas: JSON.stringify({ whole: this.pending === undefined ? this.doc : this.pending }),
        changed: this.pending !== undefined,
        unfinished,
      });
    },
    resume_unfinished(json) {
      const c = JSON.parse(json);
      if ((c.kind === 'title' || c.kind === 'page' || c.kind === 'layer')
        && [this.field, ...this.held].some((x) => x && x.kind === c.kind)) {
        // As the adapter (`DRAFT_HERE`): never over a name of the same kind being typed here.
        return 'the same kind of edit is being typed here; finish it first (nothing here was replaced)';
      }
      this.resumed.push(c);
      if (c.kind === 'title' || c.kind === 'page' || c.kind === 'layer') {
        if (this.field) this.held.push(this.field);
        this.field = c;
      }
      else if (c.kind === 'text') this.typing = c;
      return true;
    },
    pointer_up() { if (this.gesture) { this.gesture = false; this.pending = '{"dragged":1}'; } },
  });
  return ed;
}

/** A Ready session on `realEditor`, with a server that knows the editing and the handover. */
async function viewSession() {
  const ed = realEditor();
  const server = {
    revision: '"r1"', text: 'v1', editing: true, handover: null, view: null, answers: [], puts: [], acts: [],
  };
  const { s } = setup([ed], {
    put: async (snap, rev) => {
      server.puts.push(rev);
      if (server.view) return { ok: false, status: 410, revision: null };
      if (!server.editing) return { ok: false, status: 423, revision: null };
      if (rev !== server.revision) return { ok: false, status: 409, revision: server.revision };
      server.revision = '"saved"';
      return { ok: true, status: 200, revision: server.revision };
    },
    revision: async () => ({
      revision: server.revision, editing: server.editing, handover: server.handover, view: server.view,
    }),
  });
  s.deps.fetchPage = async () => ({ text: server.text, revision: server.revision, editing: server.editing });
  s.deps.claim = async () => { server.editing = true; return { ok: true }; };
  s.deps.answer = async (offer, base) => {
    server.answers.push({ offer, base });
    if (server.editing) return { ok: false, status: 403 };
    server.handover = {
      offers: offer ? [{ id: 'mine', from_here: true, changed: offer.changed, unfinished: offer.unfinished, base }] : [],
      pending: [],
    };
    return { ok: true, status: 200 };
  };
  const act = (name) => async (id) => {
    server.acts.push([name, id]);
    const o = server.handover && server.handover.offers.find((x) => x.id === id);
    if (name === 'apply') {
      if (!o) return { ok: false, status: 404 };
      if (o.base !== server.revision) return { ok: false, status: 409 };
      server.revision = '"applied"';
      server.text = 'applied';
    }
    if (name === 'keep') return { ok: true, status: 200, body: { kept: { id: 'new', name: 'Plan (handed over)' } } };
    server.handover = {
      offers: server.handover.offers.filter((x) => x.id !== id),
      pending: server.handover.pending.filter((x) => x.id !== id),
    };
    // A committed Apply answers the revision it saved (launcher_app.rs / lib.rs `apply_offer`).
    return { ok: true, status: 200, body: name === 'apply' ? { applied: true, revision: server.revision } : {} };
  };
  s.deps.applyOffer = act('apply');
  s.deps.keepOffer = act('keep');
  s.deps.dismissOffer = act('dismiss');
  s.deps.stopWaiting = act('stop');
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"', editing: true }, false);
  assert.equal(s.frame(0), 'ready');
  return { s, ed, server };
}

test('issue #23 (23-B, R78-03): the unfinished Canvas name "D" learns at once that another view took the editing, and hands its actual text over as a continuation', async () => {
  const { s, ed, server } = await viewSession();
  // The counterexample: Canvas name field open with "D" (can_reload false, nothing finished).
  ed.field = { kind: 'title', text: 'D' };
  server.editing = false; // another process opened the canvas
  assert.equal(await s.poll(), 'readonly', 'learned on the next poll, not after the edit ends');
  assert.equal(s.editable(), false, 'input stops');
  assert.deepEqual(server.answers, [{
    offer: { canvas: '', changed: false, unfinished: [{ kind: 'title', text: 'D' }] }, base: '"r1"',
  }], 'the field\'s own text goes to the editing view, nothing else');
  assert.equal(s.state.handed, 'unfinished');
  assert.deepEqual(ed.field, { kind: 'title', text: 'D' }, 'not finished or dropped here');
  assert.deepEqual(ed.history, ['h1'], 'undo history untouched');
  assert.equal(server.puts.length, 0, 'no save tried');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')), 'nothing loaded over it');
  assert.equal(await s.poll(), 'handover', 'its own offer now shows in the record');
  assert.equal(s.state.handover.offers[0].from_here, true);
  assert.equal(await s.poll(), undefined);
  assert.equal(server.answers.length, 1, 'answered once');
  assert.equal(await s.drain(), 'readonly', 'leaving says it isn\'t clean');
});

test('issue #23 (R78-03): a finished change with a rename field still open hands both over, the change and the field\'s text', async () => {
  const { s, ed, server } = await viewSession();
  ed.pending = '{"mine":1}';
  ed.field = { kind: 'page', page: 'p1', text: 'Plans' };
  // As the real renderer: a snapshot is taken even with the field open.
  assert.equal(ed.save_snapshot(), '{"mine":1}');
  ed.save_failed();
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  const { offer } = server.answers[0];
  assert.equal(offer.changed, true);
  assert.equal(offer.canvas, JSON.stringify({ whole: '{"mine":1}' }), 'the whole canvas, as it stands');
  assert.deepEqual(offer.unfinished, [{ kind: 'page', page: 'p1', text: 'Plans' }], 'and the open field is not dropped');
  assert.equal(s.state.handed, 'change');
  assert.equal(ed.pending, '{"mine":1}', 'still unsaved here');
  assert.ok(!ed.calls.includes('saved'));
});

test('issue #23 (R78-03): a drag under way when the editing moved still gets its release, and what it made is handed over then', async () => {
  const { s, ed, server } = await viewSession();
  ed.gesture = true;
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  assert.deepEqual(server.answers[0].offer.unfinished, [{ kind: 'gesture' }], 'the editing view is told a drag is under way');
  assert.equal(s.edit(() => 'input'), undefined, 'nothing new starts');
  // The person releases the pointer: it reaches the editor (`settle`), and the drag ends there.
  assert.equal(s.settle((e) => { e.pointer_up(); return 'released'; }), 'released');
  assert.equal(ed.gesture, false);
  assert.equal(await s.poll(), 'handed', 'what the drag made is handed over now');
  assert.equal(server.answers[1].offer.changed, true);
  assert.deepEqual(server.answers[1].offer.unfinished, []);
});

test('issue #23 (R78-03): a clean view the editing view waits for answers that it has nothing, once', async () => {
  const { s, server } = await viewSession();
  server.editing = false;
  server.handover = { offers: [], pending: [{ id: 'p', from_here: true, since: 1, ended: false }] };
  assert.equal(await s.poll(), 'readonly');
  assert.deepEqual(server.answers, [{ offer: null, base: '"r1"' }], 'an answer with nothing');
  assert.equal(s.state.handed, 'nothing');
  server.handover = null;
  assert.equal(await s.poll(), 'handover');
  assert.equal(server.answers.length, 1);
});

test('issue #23 (R78-03/04): the editing view applies a handed-over change and continues its unfinished edit here; a conflicting one is kept as a new canvas, never merged', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = {
    offers: [
      { id: 'a', from_here: false, changed: true, canvas: true, base: '"r1"', unfinished: [{ kind: 'title', text: 'D' }] },
      { id: 'b', from_here: false, changed: true, canvas: true, base: '"r0"', unfinished: [] },
    ],
    pending: [{ id: 'w', from_here: false, since: 1, ended: false }],
  };
  assert.equal(await s.poll(), 'handover', 'the editing view is told, in its own view');
  assert.equal(s.offers().length, 2);
  assert.equal(s.waiting().length, 1);
  ed.pending = '{"own":1}';
  assert.equal(await s.applyOffer('a'), 'unsaved', 'its own change first: nothing applied');
  assert.deepEqual(server.acts, []);
  ed.pending = undefined;
  assert.equal(await s.applyOffer('a'), 'applied');
  assert.ok(ed.calls.includes('reload:applied'), 'loaded here at once');
  assert.deepEqual(ed.resumed, [{ kind: 'title', text: 'D' }], 'its unfinished name is open here, as typed there');
  assert.deepEqual(ed.field, { kind: 'title', text: 'D' }, 'open, not finished or saved');
  // The one made on an older version: Apply is a conflict (nothing written), Keep makes a copy.
  ed.field = null;
  assert.equal(await s.applyOffer('b'), 'conflict');
  assert.equal(server.revision, '"applied"', 'nothing overwritten');
  assert.equal(await s.keepOffer('b'), 'kept');
  assert.deepEqual(s.lastBody.kept, { id: 'new', name: 'Plan (handed over)' });
  // A view that never answered: stop waiting, explicitly.
  assert.equal(await s.stopWaiting('w'), 'stopped');
  assert.deepEqual(server.acts, [['apply', 'a'], ['apply', 'b'], ['keep', 'b'], ['stop', 'w']]);
});

// ---- R79-04 (run 189): after a committed Apply, its continuation is only taken up onto the
// canvas it was applied to, never over a newer local draft, and is kept until it is.

const titleOffer = (id, unfinished = [{ kind: 'title', text: 'D' }]) => ({
  offers: [{ id, from_here: false, changed: true, canvas: true, base: '"r1"', unfinished }],
  pending: [],
});

test('R189 (R79-04 A): a name typed here while Apply is answered is never replaced; the continuation waits to be continued', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('a');
  assert.equal(await s.poll(), 'handover');
  const gate = deferred();
  const apply = s.deps.applyOffer;
  s.deps.applyOffer = async (id) => { await gate.promise; return apply(id); };
  const applying = s.applyOffer('a');
  ed.field = { kind: 'title', text: 'X' }; // typed here meanwhile
  gate.resolve();
  const r = await applying;
  assert.deepEqual(ed.field, { kind: 'title', text: 'X' }, 'the newer local name is kept');
  assert.equal(r, 'applied-unopened');
  assert.equal(server.revision, '"applied"', 'the change is committed');
  assert.ok(!ed.calls.includes('reload:applied'), 'not loaded over the name being typed');
  assert.deepEqual(s.appliedPending(), [{ id: 'a', unfinished: [{ kind: 'title', text: 'D' }], applied: true }]);
  assert.match(s.lastRefusals.join(' '), /being typed here/);
  // Never applied twice: it isn't offered for Apply again.
  assert.equal(s.offers().length, 0);
  assert.equal(await s.applyOffer('a'), undefined);
  // Still typing: Continue here waits.
  assert.equal(await s.resumeOffer('a'), 'untaken');
  assert.deepEqual(ed.field, { kind: 'title', text: 'X' });
  // Finished here (its own save or Escape): Continue here loads the applied canvas, then opens it.
  ed.field = null;
  assert.equal(await s.resumeOffer('a'), 'resumed');
  assert.ok(ed.calls.includes('reload:applied'));
  assert.deepEqual(ed.field, { kind: 'title', text: 'D' });
  assert.deepEqual(s.appliedPending(), []);
  assert.deepEqual(server.acts, [['apply', 'a']], 'committed once');
});

test('R189 (R79-04 B): a committed Apply whose canvas couldn\'t be loaded keeps its raw Theme value, and continues it once loaded', async () => {
  const { s, ed, server } = await viewSession();
  // The real adapter's packet (state.rs `put_use`) for 20 Regular typed to 2 in Theme's Size.
  const regular = (size) => ({ size, bold: false, italic: false, family: 'Inter' });
  const raw = { kind: 'theme-metric', metric: 'size', page: 'p', text: '2', style: regular(200), from: regular(2000),
    targets: [{ node: 1, whole: true, ranges: [[0, 15]] }, { node: 2, whole: false, ranges: [[4, 9]] }] };
  // As the real adapter answers: refused onto the old canvas (`resume_style_field`).
  ed.resume_unfinished = function (json) {
    this.resumed.push(JSON.parse(json));
    return this.doc === 'applied' ? ''
      : 'the layers its change is for don\'t have that text style here (its change wasn\'t applied here: apply it first)';
  };
  server.handover = titleOffer('t', [raw]);
  await s.poll();
  const fetch = s.deps.fetchPage;
  s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await s.applyOffer('t'), 'applied-unopened');
  assert.equal(server.revision, '"applied"', 'committed');
  assert.deepEqual(ed.resumed, [], 'never tried onto the canvas it wasn\'t applied to');
  assert.match(s.lastRefusals.join(' '), /couldn't be loaded here yet/);
  assert.deepEqual(s.appliedPending(), [{ id: 't', unfinished: [raw], applied: true }], 'the raw value is kept, exactly');
  // A retry while still offline: still kept, and nothing is applied again.
  assert.equal(await s.resumeOffer('t'), 'untaken');
  assert.deepEqual(server.acts, [['apply', 't']]);
  // Loaded: taken up there. A field's value opens over its next frames, so it stays listed.
  s.deps.fetchPage = fetch;
  assert.equal(await s.resumeOffer('t'), 'resumed');
  assert.deepEqual(ed.resumed, [raw]);
  assert.ok(s.continued.has('t'), 'not taken up twice');
  assert.deepEqual(s.appliedPending().map((p) => p.id), ['t'], 'kept until dismissed');
  assert.equal(await s.dismissOffer('t'), 'dismissed');
  assert.deepEqual(s.appliedPending(), []);
  assert.deepEqual(server.acts, [['apply', 't']], 'dismissing it here applies nothing');
});

test('R189 (R79-04): an Apply whose view loses the editing or is disposed while it loads takes nothing up, and hands the continuation on', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('a');
  await s.poll();
  const fetch = s.deps.fetchPage;
  // Another view takes the editing while the applied canvas loads.
  s.deps.fetchPage = async () => {
    const p = await fetch();
    server.editing = false;
    return { ...p, editing: false };
  };
  assert.equal(await s.applyOffer('a'), 'applied-unopened');
  assert.equal(s.state.readonly, true);
  assert.equal(ed.field, null, 'nothing opened in a read-only view');
  // Run 189 handed the continuation on as a plain edit; R81-02 (run 192) kept it here instead,
  // since that drops the revision it requires. R82-01 (run 193): it goes on as a kept recovery,
  // with that revision, never as a plain edit; once the record acknowledges it, leaving is clean.
  s.deps.fetchPage = fetch;
  server.handover = { offers: [], pending: [{ id: 'w', from_here: true, since: 1, ended: false }] };
  await s.poll();
  const sent = server.answers.flatMap((a) => (a.offer ? a.offer.unfinished : []));
  assert.deepEqual(sent, [{ kind: 'recovery', v: 1, id: 'a', state: 'applied', revision: '"applied"',
    packets: [{ kind: 'title', text: 'D' }] }], 'with its revision, never as a plain edit');
  assert.deepEqual(s.appliedPending()[0].unfinished, [{ kind: 'title', text: 'D' }], 'still here too');
  assert.equal(await s.drain(), 'clean', 'the record holds it now');
  s.resume();
  // Disposed while loading: nothing is taken up, by it or anything else.
  const v = await viewSession();
  v.server.handover = titleOffer('b');
  await v.s.poll();
  const gate = deferred();
  const fetch2 = v.s.deps.fetchPage;
  v.s.deps.fetchPage = async () => { await gate.promise; return fetch2(); };
  const applying = v.s.applyOffer('b');
  await new Promise((r) => { setTimeout(r, 0); });
  v.s.dispose();
  gate.resolve();
  assert.equal(await applying, undefined);
  assert.equal(v.ed.field, null);
  assert.ok(!v.ed.calls.includes('reload:applied'));
});

test('R189 (R79-04): a committed Apply the server couldn\'t remove from its record is never offered for Apply again', async () => {
  const { s, server } = await viewSession();
  server.handover = titleOffer('a', []);
  await s.poll();
  const apply = s.deps.applyOffer;
  s.deps.applyOffer = async (id) => {
    const r = await apply(id);
    // The record kept it (its removal failed after the save).
    server.handover = titleOffer('a', []);
    return { ...r, body: { ...r.body, left: 'the handover record could not be updated' } };
  };
  assert.equal(await s.applyOffer('a'), 'applied-left');
  await s.poll();
  assert.equal(await s.applyOffer('a'), undefined, 'not applied twice');
  assert.deepEqual(server.acts, [['apply', 'a']]);
});

// ---- R80 (run 190): a kept continuation opens only on the version it was applied to, and
// leaving never silently drops it or an Apply still being answered.

// A layer's name being typed there, as the real adapter hands it over (state.rs: its `base`).
const LAYER_D = { kind: 'layer', page: 'p1', node: 5, text: 'D', base: 'Old' };

test('R190 (R80-01): a kept continuation is never opened over a newer accepted version than the one it was applied to', async () => {
  // 1. The read after Apply returns a newer accepted change (the agent renamed that layer X).
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('a', [LAYER_D]);
  await s.poll();
  const fetch = s.deps.fetchPage;
  s.deps.fetchPage = async () => {
    server.revision = '"agent-x"';
    server.text = 'layer renamed X';
    return fetch();
  };
  assert.equal(await s.applyOffer('a'), 'applied-unopened');
  assert.equal(ed.doc, 'layer renamed X', 'the newer accepted version is what shows');
  assert.equal(s.state.revision, '"agent-x"');
  assert.deepEqual(ed.resumed, [], 'D isn\'t opened over it');
  assert.match(s.lastRefusals.join(' '), /newer change was saved/);
  assert.deepEqual(s.appliedPending(), [{ id: 'a', unfinished: [LAYER_D], applied: true }], 'kept, exactly');
  s.deps.fetchPage = fetch;
  assert.equal(await s.resumeOffer('a'), 'untaken', 'and not on a retry either');
  assert.deepEqual(ed.resumed, []);
  assert.deepEqual(server.acts, [['apply', 'a']], 'applied once');
  // 2. The read fails; the poll then loads the applied version; a change made here is saved
  //    (a newer accepted version); Continue here refuses.
  const v = await viewSession();
  v.server.handover = titleOffer('b', [LAYER_D]);
  await v.s.poll();
  const fetch2 = v.s.deps.fetchPage;
  v.s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await v.s.applyOffer('b'), 'applied-unopened');
  v.s.deps.fetchPage = fetch2;
  assert.equal(await v.s.poll(), 'reloaded');
  assert.equal(v.s.state.revision, '"applied"');
  v.ed.pending = '{"renamed":"X"}';
  assert.equal(await v.s.save(), 'saved');
  assert.equal(v.server.revision, '"saved"');
  assert.equal(await v.s.resumeOffer('b'), 'untaken');
  assert.deepEqual(v.ed.resumed, [], 'X isn\'t typed over');
  assert.match(v.s.lastRefusals.join(' '), /newer change was saved/);
  assert.deepEqual(v.s.appliedPending()[0].unfinished, [LAYER_D]);
  assert.deepEqual(v.server.acts, [['apply', 'b']]);
});

test('R190 (R80-02): leaving keeps the view while a committed Apply\'s continuation isn\'t open here', async () => {
  const { s, ed, server } = await viewSession();
  // A unique unfinished name; the view that handed it over is gone.
  const name = { kind: 'title', text: 'Dz-190' };
  server.handover = titleOffer('a', [name]);
  await s.poll();
  s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await s.applyOffer('a'), 'applied-unopened');
  assert.ok(ed.can_reload(), 'the editor itself is clean');
  assert.equal(await s.drain(), 'pending', 'not clean: the only copy of it is here');
  assert.ok(!ed.freed);
  assert.equal(s.holdsWork(), true, 'the page\'s leave guard counts it');
  s.resume();
  assert.deepEqual(s.appliedPending()[0].unfinished, [name]);
  // Explicitly dismissed: then it leaves clean.
  assert.equal(await s.dismissOffer('a'), 'dismissed');
  assert.equal(s.holdsWork(), false);
  assert.equal(await s.drain(), 'clean');
  assert.deepEqual(server.acts, [['apply', 'a']]);
});

test('R190 (R80-02): leaving while an Apply or its load is answered waits for its outcome', async () => {
  const held = async (base = '"r1"') => {
    const v = await viewSession();
    v.server.handover = {
      offers: [{ id: 'a', from_here: false, changed: true, canvas: true, base, unfinished: [{ kind: 'title', text: 'Dz' }] }],
      pending: [],
    };
    await v.s.poll();
    const gate = deferred();
    const apply = v.s.deps.applyOffer;
    v.s.deps.applyOffer = async (id) => { await gate.promise; return apply(id); };
    return { ...v, gate };
  };
  const tick = () => new Promise((r) => { setTimeout(r, 0); });
  // Committed: the view stays, the continuation kept.
  const c = await held();
  const applying = c.s.applyOffer('a');
  let left = false;
  const leaving = c.s.drain().then((r) => { left = true; return r; });
  await tick();
  assert.equal(left, false, 'it waits for the Apply\'s answer');
  c.gate.resolve();
  assert.equal(await leaving, 'pending');
  assert.equal(await applying, 'applied-unopened');
  assert.equal(c.ed.field, null, 'nothing opened while leaving');
  assert.deepEqual(c.s.appliedPending()[0].unfinished, [{ kind: 'title', text: 'Dz' }]);
  assert.deepEqual(c.server.acts, [['apply', 'a']]);
  // Refused (made on an older version): nothing was committed, so it leaves clean.
  const r = await held('"r0"');
  const refusing = r.s.applyOffer('a');
  const leavingR = r.s.drain();
  r.gate.resolve();
  assert.equal(await refusing, 'conflict');
  assert.equal(await leavingR, 'clean');
  assert.equal(r.server.revision, '"r1"', 'nothing written');
  // Committed, its load still answering: it waits for that too.
  const l = await held();
  const load = deferred();
  const fetch = l.s.deps.fetchPage;
  l.s.deps.fetchPage = async () => { await load.promise; return fetch(); };
  const applyingL = l.s.applyOffer('a');
  l.gate.resolve();
  await tick();
  let leftL = false;
  const leavingL = l.s.drain().then((x) => { leftL = true; return x; });
  await tick();
  assert.equal(leftL, false, 'it waits for the load');
  load.resolve();
  assert.equal(await leavingL, 'pending');
  assert.equal(await applyingL, 'applied-unopened');
});

// ---- run 191: an Apply whose answer was lost keeps what this view holds of it.

// A unique Canvas name being typed there, as the real adapter hands it over (with its `base`).
const UNKNOWN_NAME = { kind: 'title', base: 'Canvas', text: 'Dz-190-unknown-æøå' };

test('R191: an Apply whose answer is lost after it committed keeps its unfinished edit, never calls leaving clean, and never applies again', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('u', [UNKNOWN_NAME]);
  assert.equal(await s.poll(), 'handover');
  const apply = s.deps.applyOffer;
  // The server commits it and removes the offer; its answer never arrives.
  s.deps.applyOffer = async (id) => { await apply(id); throw new Error('the answer was lost'); };
  assert.equal(await s.applyOffer('u'), 'unknown', 'neither applied nor refused: not known');
  assert.match(s.lastRefusals.join(' '), /isn't known whether/);
  assert.equal(server.revision, '"applied"', 'it did commit, there');
  // Polling learns the offer is gone and the newer revision; neither proves what happened.
  assert.equal(await s.poll(), 'handover');
  assert.equal(await s.poll(), undefined);
  assert.deepEqual(s.offers(), []);
  assert.deepEqual(s.uncertainPending(), [{ id: 'u', unfinished: [UNKNOWN_NAME], uncertain: true }],
    'the exact packet, kept');
  assert.equal(s.holdsWork(), true, 'the page\'s leave guard counts it');
  assert.equal(await s.drain(), 'uncertain', 'leaving isn\'t clean: this is its only copy');
  s.resume();
  // Never opened against an unknown outcome, and never applied again on its own.
  assert.equal(await s.resumeOffer('u'), 'untaken');
  assert.deepEqual(ed.resumed, []);
  assert.equal(await s.applyOffer('u'), undefined);
  assert.deepEqual(server.acts, [['apply', 'u']], 'exactly one Apply');
  // The person's explicit choice drops it; then leaving is clean.
  assert.equal(await s.dismissOffer('u'), 'dismissed');
  assert.deepEqual(s.uncertainPending(), []);
  assert.equal(s.holdsWork(), false);
  assert.equal(await s.drain(), 'clean');
  assert.deepEqual(server.acts, [['apply', 'u']], 'dismissing it here sends nothing');
});

test('R191: an Apply lost without committing stays the record\'s offer; a known refusal stays a refusal', async () => {
  const { s, server } = await viewSession();
  server.handover = titleOffer('u', [UNKNOWN_NAME]);
  await s.poll();
  // Nothing committed, nothing removed, and no answer.
  s.deps.applyOffer = async () => { throw new Error('the answer was lost'); };
  assert.equal(await s.applyOffer('u'), 'unknown');
  // Before a poll the cached record proves nothing: leaving keeps the view.
  assert.equal(await s.drain(), 'uncertain');
  s.resume();
  // A poll after it still lists the offer. That isn't proof of what the Apply did (R81-01, run
  // 192: the answer may be from before it, or from while it still runs), so the kept edit stays
  // this view's work, and isn't opened by Continue here either; run 191 called this clean.
  await s.poll();
  assert.equal(s.offers().length, 1);
  assert.equal(s.offers()[0].uncertain, true, 'shown as possibly applied');
  assert.equal(await s.drain(), 'uncertain', 'a listed offer settles nothing');
  s.resume();
  assert.equal(await s.resumeOffer('u'), 'untaken');
  // The person's own choice ends it: Dismiss (here and from the record, while it lists it).
  assert.equal(await s.dismissOffer('u'), 'dismissed');
  assert.equal(await s.drain(), 'clean');
  // A definite refusal is a refusal, not an unknown, and keeps nothing extra.
  const v = await viewSession();
  v.server.handover = { offers: [{ id: 'x', from_here: false, changed: true, canvas: true, base: '"r0"',
    unfinished: [UNKNOWN_NAME] }], pending: [] };
  await v.s.poll();
  assert.equal(await v.s.applyOffer('x'), 'conflict');
  assert.deepEqual(v.s.uncertainPending(), []);
  assert.equal(v.server.revision, '"r1"', 'nothing written');
  assert.equal(await v.s.drain(), 'clean');
});

// Run 191 handed the kept edit on as a plain unfinished edit and left clean; R81-02 (run 192)
// showed that drops its unknown outcome, so it was kept here; R82-01 (run 193) hands it on as
// a kept recovery, still unknown, never as a plain edit.
test('R191/R192/R193: a view that loses the editing after a lost Apply answer hands its edit on only as an unknown recovery', async () => {
  const { s, server } = await viewSession();
  server.handover = titleOffer('u', [UNKNOWN_NAME]);
  await s.poll();
  const apply = s.deps.applyOffer;
  s.deps.applyOffer = async (id) => { await apply(id); throw new Error('the answer was lost'); };
  assert.equal(await s.applyOffer('u'), 'unknown');
  server.editing = false;
  server.handover = { offers: [], pending: [{ id: 'w', from_here: true, since: 1, ended: false }] };
  await s.poll();
  const sent = server.answers.flatMap((a) => (a.offer ? a.offer.unfinished : []));
  assert.deepEqual(sent, [{ kind: 'recovery', v: 1, id: 'u', state: 'unknown', packets: [UNKNOWN_NAME] }],
    'never stripped of its unknown outcome');
  assert.deepEqual(s.uncertainPending().map((u) => u.unfinished), [[UNKNOWN_NAME]], 'still here too');
  assert.equal(await s.drain(), 'clean', 'the record holds it now');
});

// ---- R81 (run 192): no poll is proof of what an unanswered Apply did, and what this view keeps
// is never handed on stripped of what guards it.

// A Design-panel X value typed there, as the real adapter hands it over (its layer's `bases`).
const INSPECTOR_X = { kind: 'inspector', page: 'p1', ids: [5], label: 'X  ', nth: 0, text: '24',
  bases: [{ id: 5, name: 'Rectangle' }] };

test('R192 (R81-01): a poll answered from before an unanswered Apply never makes its kept edit safe to leave', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('u', [UNKNOWN_NAME]);
  await s.poll();
  // A poll starts while the offer is still there; its answer is held in transport.
  const held = deferred();
  const revision = s.deps.revision;
  s.deps.revision = async () => {
    const snapshot = await revision();
    s.deps.revision = revision;
    await held.promise;
    return JSON.parse(JSON.stringify(snapshot));
  };
  const polling = s.poll();
  // Apply commits there and removes the offer; its answer is lost.
  const apply = s.deps.applyOffer;
  s.deps.applyOffer = async (id) => { await apply(id); throw new Error('the answer was lost'); };
  assert.equal(await s.applyOffer('u'), 'unknown');
  // The old answer arrives, still listing the offer.
  held.resolve();
  await polling;
  assert.equal(s.offers().length, 1, 'the old answer lists it');
  assert.equal(s.holdsWork(), true, 'still this view\'s work');
  assert.equal(await s.drain(), 'uncertain', 'never clean from a poll');
  s.resume();
  assert.deepEqual(s.uncertainPending().map((u) => u.unfinished), [[UNKNOWN_NAME]]);
  assert.equal(await s.resumeOffer('u'), 'untaken', 'and never opened because the record lists it');
  assert.deepEqual(ed.resumed, []);
  assert.deepEqual(server.acts, [['apply', 'u']]);
});

test('R192 (R81-01): an Apply that timed out while it still runs keeps its edit, whatever a fresh poll shows', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = titleOffer('u', [UNKNOWN_NAME]);
  await s.poll();
  // The request times out here; the server is still working on it.
  const commit = deferred();
  const apply = s.deps.applyOffer;
  let started = 0;
  s.deps.applyOffer = async (id) => {
    started += 1;
    commit.promise.then(() => apply(id));
    return { ok: false, status: 0 };
  };
  assert.equal(await s.applyOffer('u'), 'unknown');
  // A fresh poll still sees the offer (the commit hasn't happened yet).
  await s.poll();
  assert.equal(s.offers().length, 1);
  assert.equal(await s.drain(), 'uncertain');
  s.resume();
  // Then the commit lands and removes it: the kept edit is still here.
  commit.resolve();
  await new Promise((r) => { setTimeout(r, 0); });
  await s.poll();
  assert.deepEqual(s.offers(), []);
  assert.deepEqual(s.uncertainPending().map((u) => u.unfinished), [[UNKNOWN_NAME]]);
  assert.equal(await s.drain(), 'uncertain');
  assert.equal(started, 1, 'sent once, never again on its own');
  assert.deepEqual(ed.resumed, []);
});

// R81-02 (run 192) asserted these were not handed on at all; R82-01 (run 193) hands them on
// as kept recoveries with their guards. What stays asserted: never a plain edit, exactly the
// guarded form, and the view's copy released only once the record acknowledges it.
const plainKinds = ['title', 'page', 'layer', 'text', 'inspector', 'theme-metric'];
test('R192/R193 (R81-02): a read-only view never hands on an applied or unanswered edit stripped of what guards it', async () => {
  // Unanswered (committed there): handed on only as an unknown recovery.
  const { s, server } = await viewSession();
  server.handover = titleOffer('u', [INSPECTOR_X]);
  await s.poll();
  const apply = s.deps.applyOffer;
  s.deps.applyOffer = async (id) => { await apply(id); throw new Error('the answer was lost'); };
  assert.equal(await s.applyOffer('u'), 'unknown');
  server.editing = false;
  server.handover = { offers: [], pending: [{ id: 'w', from_here: true, since: 1, ended: false }] };
  await s.poll();
  const sent = server.answers.flatMap((a) => (a.offer ? a.offer.unfinished : []));
  assert.ok(!sent.some((c) => plainKinds.includes(c.kind)), 'never as a plain edit');
  assert.deepEqual(sent, [{ kind: 'recovery', v: 1, id: 'u', state: 'unknown', packets: [INSPECTOR_X] }]);
  assert.equal(await s.drain(), 'clean', 'the record holds it now');
  // Applied, its revision required, its load failed: handed on only with that revision.
  const v = await viewSession();
  v.server.handover = titleOffer('a', [INSPECTOR_X]);
  await v.s.poll();
  const fetch = v.s.deps.fetchPage;
  v.s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await v.s.applyOffer('a'), 'applied-unopened');
  v.s.deps.fetchPage = fetch;
  v.server.editing = false;
  v.server.handover = { offers: [], pending: [{ id: 'w', from_here: true, since: 1, ended: false }] };
  await v.s.poll();
  const sent2 = v.server.answers.flatMap((a) => (a.offer ? a.offer.unfinished : []));
  assert.ok(!sent2.some((c) => plainKinds.includes(c.kind)));
  assert.deepEqual(sent2, [{ kind: 'recovery', v: 1, id: 'a', state: 'applied', revision: '"applied"', packets: [INSPECTOR_X] }]);
  assert.deepEqual(v.s.appliedPending()[0].unfinished, [INSPECTOR_X], 'exactly, here too');
  assert.equal(await v.s.drain(), 'clean');
});

// ---- R82-01 (run 193): what a view keeps of an applied or unanswered offer reaches the view
// that edits now, with its constraints, through the ordinary guarded handover.

/**
 * One canvas's server record, as library.rs keeps it, shared by several views (each a real
 * Session on `realEditor`): the editing view (`editor`, the newest to open or claim), offers
 * with the view they came from (`from_here` per reader), pending views, and the guarded
 * actions. An answer replaces that view's own offer (`take_offers_from`); a claim removes the
 * claimer's own offers; Apply commits once on its base, else 409; Keep makes a new canvas of
 * the offer's settled canvas and removes the whole offer, as lib.rs `keep_offer` does; a
 * non-editing view's action is 423.
 *
 * `guarded` (run 194, lib.rs `offer_guard`): an Apply, Keep or Dismiss of an offer carrying
 * kept recoveries must say it resolves every one (`resolved`), else 426 and nothing changes;
 * then each is recorded as a receipt `{offer, recovery}` in the same record write; and an
 * answer mixing a recovery with anything else is refused (400). Unguarded is the backend
 * before run 194 (and an older backend process sharing the record). `answerGate`/`claimGate`
 * hold an answer's acknowledgement (after its record write) or a claim (before it).
 */
function sharedRecord({ guarded = true } = {}) {
  const srv = { revision: '"r1"', text: 'v1', editor: null, offers: [], pending: [], acts: [], answers: [], next: 0,
    failAnswer: null, resolved: [], kept: [], answerGate: null, claimGate: null, guarded };
  const as = (view) => ({
    offers: srv.offers.map((o) => ({ ...JSON.parse(JSON.stringify(o)), from_here: o.from === view })),
    pending: srv.pending.map((p) => ({ ...p, from_here: p.from === view })),
    ...(guarded ? { resolved: JSON.parse(JSON.stringify(srv.resolved)) } : {}),
  });
  const recoveryIds = (o) => o.unfinished.filter((c) => c && c.kind === 'recovery' && typeof c.id === 'string').map((c) => c.id);
  // The guard of a destructive action on offer `o` (guarded only): 426 unless it resolves them.
  const guard = (o, resolved) => {
    const ids = recoveryIds(o);
    if (!guarded || !ids.length) return null;
    if (!ids.every((r) => Array.isArray(resolved) && resolved.includes(r))) return { ok: false, status: 426 };
    return null;
  };
  const receipts = (o) => {
    if (!guarded) return;
    for (const r of recoveryIds(o)) srv.resolved.push({ offer: o.id, recovery: r });
  };
  srv.take = (view) => {
    if (srv.editor && srv.editor !== view) srv.pending.push({ id: `p${++srv.next}`, from: srv.editor, since: 1, ended: false });
    srv.editor = view;
  };
  const deps = (view) => ({
    revision: async () => ({ revision: srv.revision, editing: srv.editor === view, handover: as(view), view: null }),
    fetchPage: async () => ({ text: srv.text, revision: srv.revision, editing: srv.editor === view }),
    put: async (snap, rev) => {
      if (srv.editor !== view) return { ok: false, status: 423, revision: null };
      if (rev !== srv.revision) return { ok: false, status: 409, revision: srv.revision };
      srv.revision = `"s${++srv.next}"`;
      return { ok: true, status: 200, revision: srv.revision };
    },
    answer: async (offer, base) => {
      srv.answers.push({ view, offer: JSON.parse(JSON.stringify(offer)), base });
      if (srv.failAnswer) return srv.failAnswer;
      if (srv.editor === view) return { ok: false, status: 403 };
      if (guarded && offer && offer.unfinished.some((c) => c && c.kind === 'recovery')
        && (offer.canvas || offer.changed || offer.unfinished.some((c) => !c || c.kind !== 'recovery'))) {
        return { ok: false, status: 400 };
      }
      srv.pending = srv.pending.filter((p) => p.from !== view);
      srv.offers = srv.offers.filter((o) => o.from !== view);
      let made = null;
      if (offer) {
        made = `o${++srv.next}`;
        srv.offers.push({ id: made, from: view, changed: offer.changed, canvas: Boolean(offer.canvas),
          base, unfinished: JSON.parse(JSON.stringify(offer.unfinished)) });
      }
      if (srv.answerGate) await srv.answerGate;
      return { ok: true, status: 200, body: { offer: made } };
    },
    applyOffer: async (id, resolved) => {
      srv.acts.push([view, 'apply', id]);
      if (srv.editor !== view) return { ok: false, status: 423 };
      const o = srv.offers.find((x) => x.id === id);
      if (!o) return { ok: false, status: 404 };
      const refused = guard(o, resolved);
      if (refused) return refused;
      if (!o.changed || !o.canvas) return { ok: false, status: 404 };
      if (o.base !== srv.revision) return { ok: false, status: 409 };
      srv.revision = `"applied${++srv.next}"`;
      srv.text = 'applied';
      srv.offers = srv.offers.filter((x) => x.id !== id);
      receipts(o);
      return { ok: true, status: 200, body: { applied: true, revision: srv.revision } };
    },
    dismissOffer: async (id, resolved) => {
      srv.acts.push([view, 'dismiss', id]);
      if (srv.editor !== view) return { ok: false, status: 423 };
      const o = srv.offers.find((x) => x.id === id);
      if (!o) return { ok: false, status: 404 };
      const refused = guard(o, resolved);
      if (refused) return refused;
      srv.offers = srv.offers.filter((x) => x.id !== id);
      receipts(o);
      return { ok: true, status: 200, body: {} };
    },
    keepOffer: async (id, resolved) => {
      srv.acts.push([view, 'keep', id]);
      if (srv.editor !== view) return { ok: false, status: 423 };
      const o = srv.offers.find((x) => x.id === id);
      if (!o) return { ok: false, status: 404 };
      const refused = guard(o, resolved);
      if (refused) return refused;
      if (!o.canvas) return { ok: false, status: 404 };
      // A new canvas of its settled canvas only; the whole offer goes (lib.rs `keep_offer`).
      srv.kept.push({ from: o.id, base: o.base });
      srv.offers = srv.offers.filter((x) => x.id !== id);
      receipts(o);
      return { ok: true, status: 200, body: { kept: { id: `k${++srv.next}`, name: 'Plan (handed over)' } } };
    },
    stopWaiting: async (id) => {
      srv.pending = srv.pending.filter((p) => p.id !== id);
      return { ok: true, status: 200, body: {} };
    },
    claim: async () => {
      if (srv.claimGate) await srv.claimGate;
      srv.take(view);
      srv.offers = srv.offers.filter((o) => o.from !== view);
      return { ok: true };
    },
  });
  srv.open = async (view, Ctor = Session) => {
    srv.take(view);
    const ed = realEditor();
    const { s: made } = setup([ed]);
    const s = Ctor === Session ? made : new Ctor(made.deps);
    Object.assign(s.deps, deps(view));
    await s.start({ id: `canvas-${view}` }, { text: srv.text, revision: srv.revision, editing: true }, false);
    assert.equal(s.frame(0), 'ready');
    return { s, ed };
  };
  /** Every place the record or a view still holds `text` (a raw packet's), for loss checks. */
  srv.holders = (views, text) => {
    const inRecord = srv.offers.some((o) => JSON.stringify(o.unfinished).includes(text));
    const inViews = views.filter(({ s }) => JSON.stringify([...s.applied.values(), ...s.uncertain.values()]).includes(text));
    return { inRecord, inViews: inViews.length };
  };
  return srv;
}

// A Canvas name being typed in the view that handed it over (A), as the real adapter exports it.
const NAME_D = { kind: 'title', base: 'Canvas', text: 'Dz-193-æøå' };

/** B applies A's offer (a finished change and NAME_D); the read after it fails. */
async function appliedInB(srv) {
  srv.offers.push({ id: 'a', from: 'A', changed: true, canvas: true, base: srv.revision, unfinished: [NAME_D] });
  const b = await srv.open('B');
  await b.s.poll();
  const fetch = b.s.deps.fetchPage;
  b.s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await b.s.applyOffer('a'), 'applied-unopened');
  b.s.deps.fetchPage = fetch;
  return b;
}

test('R193 (R82-01): an applied, unopened edit reaches the newest view with its revision; it continues there without touching the old view', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  const applied = srv.revision;
  assert.equal(b.s.holdsWork(), true);
  // C opens the canvas normally: it edits now, and waits for B.
  const c = await srv.open('C');
  // B, read-only now, answers its ordinary poll: its kept edit goes with its constraints.
  await b.s.poll();
  assert.equal(b.s.state.readonly, true);
  const sent = srv.answers.at(-1).offer.unfinished;
  assert.deepEqual(sent, [{ kind: 'recovery', v: 1, id: 'a', state: 'applied', revision: applied, packets: [NAME_D] }],
    'exactly what it keeps: its identity, state, required revision and raw packet');
  assert.equal(await b.s.drain(), 'clean', 'the record holds it now (its answer was acknowledged)');
  b.s.resume();
  // C sees it and continues it, on exactly that revision, without B's UI.
  assert.equal(await c.s.poll(), 'handover');
  const offer = c.s.offers().find((o) => o.unfinished.some((u) => u.kind === 'recovery'));
  assert.ok(offer, 'listed for the view that edits');
  assert.equal(await c.s.resumeOffer(offer.id), 'resumed');
  assert.deepEqual(c.ed.field, NAME_D, 'the raw packet, exactly');
  assert.deepEqual(srv.acts.filter(([, a]) => a === 'apply').length, 1, 'never applied again');
  assert.deepEqual(srv.offers, [], 'taken up there: it leaves the record');
  // B learns it was taken up (a poll after its acknowledged answer) and lets it go.
  await b.s.poll();
  assert.deepEqual(b.s.appliedPending(), []);
  assert.equal(await b.s.drain(), 'clean');
});

test('R193 (R82-01): a kept edit is never opened over a newer version, and its guard survives another hop', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  const applied = srv.revision;
  const c = await srv.open('C');
  await b.s.poll();
  // A newer accepted change (C's own save, at R2) before C continues it.
  await c.s.poll();
  c.ed.pending = '{"x":200}';
  assert.equal(await c.s.save(), 'saved');
  assert.notEqual(srv.revision, applied);
  const offer = c.s.offers().find((o) => o.unfinished.some((u) => u.kind === 'recovery'));
  assert.equal(await c.s.resumeOffer(offer.id), 'untaken');
  assert.match(c.s.lastRefusals.join(' '), /newer change was saved/);
  assert.equal(c.ed.field, null, 'nothing opened over it');
  assert.equal(srv.offers.length, 1, 'and nothing dropped: it stays listed');
  // D opens: C is read-only now; D sees the same kept edit with the same guard.
  const d = await srv.open('D');
  await c.s.poll();
  await d.s.poll();
  const atD = d.s.offers().find((o) => o.unfinished.some((u) => u.kind === 'recovery'));
  assert.deepEqual(atD.unfinished, [{ kind: 'recovery', v: 1, id: 'a', state: 'applied', revision: applied, packets: [NAME_D] }]);
  assert.equal(await d.s.resumeOffer(atD.id), 'untaken');
  assert.equal(d.ed.field, null);
  assert.equal(srv.acts.filter(([, a]) => a === 'apply').length, 1);
});

test('R193 (R82-01): an unanswered Apply\'s kept edit reaches the newest view still unknown: never opened, only dismissed there', async () => {
  const srv = sharedRecord();
  srv.offers.push({ id: 'a', from: 'A', changed: true, canvas: true, base: srv.revision, unfinished: [NAME_D] });
  const b = await srv.open('B');
  await b.s.poll();
  const apply = b.s.deps.applyOffer;
  b.s.deps.applyOffer = async (id) => { await apply(id); throw new Error('the answer was lost'); };
  assert.equal(await b.s.applyOffer('a'), 'unknown');
  const c = await srv.open('C');
  await b.s.poll();
  assert.deepEqual(srv.answers.at(-1).offer.unfinished, [{ kind: 'recovery', v: 1, id: 'a', state: 'unknown', packets: [NAME_D] }]);
  await c.s.poll();
  const offer = c.s.offers().find((o) => o.unfinished.some((u) => u.kind === 'recovery'));
  assert.equal(await c.s.resumeOffer(offer.id), 'untaken');
  assert.match(c.s.lastRefusals.join(' '), /isn't known/);
  assert.equal(c.ed.field, null);
  // Dismissed there, deliberately: B lets it go after its next poll.
  assert.equal(await c.s.dismissOffer(offer.id), 'dismissed');
  await b.s.poll();
  assert.deepEqual(b.s.uncertainPending(), []);
  assert.equal(srv.acts.filter(([, a]) => a === 'apply').length, 1);
});

test('R193 (R82-01): until its answer is acknowledged, and again once it edits, the old view keeps its only copy', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  await srv.open('C');
  // The answer is refused: nothing is released.
  srv.failAnswer = { ok: false, status: 503 };
  await b.s.poll();
  assert.equal(await b.s.drain(), 'pending');
  b.s.resume();
  // Acknowledged: released to the record. Then B takes the editing back (its own offers leave
  // the record with the claim): it holds the only copy again.
  srv.failAnswer = null;
  await b.s.poll();
  assert.equal(await b.s.drain(), 'clean');
  b.s.resume();
  assert.equal(await b.s.claim(), 'editing');
  await b.s.poll();
  assert.deepEqual(b.s.appliedPending().map((p) => p.unfinished), [[NAME_D]], 'kept, exactly');
  assert.equal(await b.s.drain(), 'pending');
});

// Run 193 had B hand its own change over with its kept recovery in one offer; run 194 (R83-02/
// 03) never mixes them (an older editor could apply or keep the change and drop the recovery).
// Such a mixed offer can still be in a record from an editor before run 194: applying it keeps
// the recovery, with its own revision, and hands it on unchanged.
test('R193/R194 (R82-01): applying a mixed offer (from before run 194) keeps its recovery, with its own revision, and hands it on unchanged', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  const applied = srv.revision;
  const recovery = { kind: 'recovery', v: 1, id: 'a', state: 'applied', revision: applied, packets: [NAME_D] };
  const c = await srv.open('C');
  // B's own answer is a recovery-only offer now; the older producer's mixed one is in the record.
  await b.s.poll();
  srv.offers = srv.offers.filter((o) => o.from !== 'B');
  srv.offers.push({ id: 'm', from: 'B193', changed: true, canvas: true, base: srv.revision, unfinished: [recovery] });
  // C applies its change: a new commit; the kept recovery is C's to hold now, as it was.
  await c.s.poll();
  const offer = c.s.offers().find((o) => o.changed);
  assert.equal(await c.s.applyOffer(offer.id), 'applied');
  assert.deepEqual(c.s.appliedPending(), [{ id: 'a', unfinished: [NAME_D], applied: true }]);
  assert.equal(await c.s.resumeOffer('a'), 'untaken', 'its own revision, not C\'s commit');
  assert.match(c.s.lastRefusals.join(' '), /newer change was saved/);
  assert.equal(await c.s.drain(), 'pending', 'C holds its only copy now');
  c.s.resume();
  // D opens; C hands it on unchanged.
  await srv.open('D');
  await c.s.poll();
  assert.deepEqual(srv.answers.at(-1).offer.unfinished, [recovery]);
  assert.equal(srv.acts.filter(([, a]) => a === 'apply').length, 2, 'A\'s offer once, B\'s change once');
});

test('R193 (R82-01): a recipient never takes up a recovery it doesn\'t support, and the representation isn\'t a kind older editors take up', async () => {
  // Readers before run 193 take up only these kinds (session.js `movable`, runs 186–192).
  const older = ['title', 'page', 'layer', 'text', 'inspector', 'theme-metric'];
  assert.ok(!older.includes('recovery'), 'so an older reader lists it but can\'t open it');
  const srv = sharedRecord();
  srv.offers.push({ id: 'f', from: 'F', changed: false, canvas: false, base: srv.revision,
    unfinished: [{ kind: 'recovery', v: 2, id: 'x', state: 'applied', revision: srv.revision, packets: [NAME_D] }] });
  const c = await srv.open('C');
  await c.s.poll();
  assert.equal(await c.s.resumeOffer('f'), 'untaken');
  assert.match(c.s.lastRefusals.join(' '), /newer editor/);
  assert.deepEqual(c.ed.resumed, [], 'nothing stripped down and opened');
  assert.equal(srv.offers.length, 1, 'kept in the record');
});

// ---- R83 (run 194): a kept recovery's only copy survives a Claim racing its answer, a Keep,
// and an older editor acting on the record.

const isRec = (c) => Boolean(c) && c.kind === 'recovery';
const RECOVERY_D = (state, revision) => ({ kind: 'recovery', v: 1, id: 'a', state,
  ...(state === 'applied' ? { revision } : {}), packets: [NAME_D] });

/** B with A's offer applied once and its NAME_D kept: `applied` (the read after it failed) or `unknown` (no answer). */
async function keptInB(srv, state) {
  if (state === 'applied') return appliedInB(srv);
  srv.offers.push({ id: 'a', from: 'A', changed: true, canvas: true, base: srv.revision, unfinished: [NAME_D] });
  const b = await srv.open('B');
  await b.s.poll();
  const apply = b.s.deps.applyOffer;
  b.s.deps.applyOffer = async (id, r) => { await apply(id, r); throw new Error('the answer was lost'); };
  assert.equal(await b.s.applyOffer('a'), 'unknown');
  return b;
}

test('R194: the older reader fixture is the exact frozen run-192 Session', () => {
  const bytes = readFileSync(new URL('./legacy/session-192.mjs', import.meta.url));
  assert.equal(bytes.length, 48667);
  assert.equal(createHash('sha256').update(bytes).digest('hex'),
    '44a0eee1b7fb9af2657e108b2e52a266e9240f40d4bb70551db77d6098fa6ab3');
});

for (const state of ['applied', 'unknown']) {
  test(`R194 (R83-01): a Claim racing a late answer acknowledgement never releases the only copy (${state})`, async () => {
    const srv = sharedRecord();
    const b = await keptInB(srv, state);
    const applied = state === 'applied' ? srv.revision : undefined;
    const c = await srv.open('C');
    // A first answer is refused (transient): B shows read-only with Edit here, still holding it.
    srv.failAnswer = { ok: false, status: 503 };
    await b.s.poll();
    assert.equal(b.s.state.readonly, true);
    srv.failAnswer = null;
    // The next answer writes the record; its acknowledgement is held in transport.
    const ack = deferred();
    srv.answerGate = ack.promise;
    const polling = b.s.poll();
    await tick();
    assert.ok(srv.offers.some((o) => o.from === 'B' && o.unfinished.some(isRec)), 'written');
    // Edit here while that answer is outstanding; the claim itself held too.
    const go = deferred();
    srv.claimGate = go.promise;
    const claiming = b.s.claim();
    await tick();
    // The old acknowledgement arrives first, then the claim succeeds (and removes B's offer).
    srv.answerGate = null;
    ack.resolve();
    await polling;
    srv.claimGate = null;
    go.resolve();
    assert.equal(await claiming, 'editing');
    assert.ok(!srv.offers.some((o) => o.from === 'B'), 'the claim removed its own offer');
    // Leaving at once is never clean, and the next poll keeps it too.
    assert.equal(b.s.holdsWork(), true);
    assert.equal(await b.s.drain(), state === 'applied' ? 'pending' : 'uncertain');
    b.s.resume();
    await b.s.poll();
    assert.deepEqual(srv.holders([b, c], NAME_D.text), { inRecord: false, inViews: 1 });
    assert.deepEqual(b.s.recoveries(), [RECOVERY_D(state, applied)], 'exactly as kept');
    assert.equal(srv.acts.filter(([, a]) => a === 'apply').length, 1);
  });
}

test('R194 (R83-02): a kept edit is handed over on its own, never with its view\'s other change; that change follows once it is taken up', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  const applied = srv.revision;
  await b.s.poll();
  assert.equal(b.s.state.revision, srv.revision, 'B loads the applied version, the edit still kept');
  b.ed.pending = '{"b":1}';
  const c = await srv.open('C');
  await b.s.poll();
  assert.deepEqual(srv.answers.at(-1).offer, { canvas: '', changed: false, unfinished: [RECOVERY_D('applied', applied)] },
    'on its own: nothing an older editor could apply or keep');
  await c.s.poll();
  const offer = c.s.offers().find((o) => o.unfinished.some(isRec));
  assert.equal(offer.canvas, false);
  assert.equal(offer.changed, false);
  // C continues it on its version: a receipt, and B lets it go; then B's own change follows.
  assert.equal(await c.s.resumeOffer(offer.id), 'resumed');
  assert.deepEqual(c.ed.field, NAME_D);
  await b.s.poll();
  assert.deepEqual(b.s.appliedPending(), []);
  const next = srv.answers.at(-1).offer;
  assert.equal(next.changed, true);
  assert.ok(!next.unfinished.some(isRec));
  // Keeping that change is ordinary Keep (nothing kept rides with it).
  await c.s.poll();
  const change = c.s.offers().find((o) => o.changed);
  assert.equal(await c.s.keepOffer(change.id), 'kept');
  assert.equal(srv.kept.length, 1);
});

test('R194 (R83-02): Keep of a mixed offer (from an editor before run 194) keeps its recovery here, guarded; one it can\'t hold is refused before anything happens', async () => {
  const srv = sharedRecord();
  const c = await srv.open('C');
  const recovery = RECOVERY_D('applied', '"r1"');
  srv.offers.push({ id: 'm', from: 'B193', changed: true, canvas: true, base: srv.revision, unfinished: [recovery] });
  await c.s.poll();
  assert.equal(await c.s.keepOffer('m'), 'kept');
  assert.equal(srv.kept.length, 1, 'its canvas kept as a new canvas');
  assert.deepEqual(c.s.appliedPending(), [{ id: 'a', unfinished: [NAME_D], applied: true }], 'its recovery held here, as it was');
  assert.deepEqual(srv.resolved, [{ offer: 'm', recovery: 'a' }]);
  assert.equal(await c.s.drain(), 'pending', 'C holds its only copy now');
  c.s.resume();
  // An unknown-version recovery can't be held to its guard: nothing is kept or removed.
  srv.offers.push({ id: 'n', from: 'B9', changed: true, canvas: true, base: srv.revision,
    unfinished: [{ ...recovery, v: 2, id: 'z' }] });
  await c.s.poll();
  assert.equal(await c.s.keepOffer('n'), 'unsupported');
  assert.equal(srv.kept.length, 1);
  assert.ok(srv.offers.some((o) => o.id === 'n'));
});

for (const guarded of [false, true]) {
  test(`R194 (R83-03): the exact run-192 editor can't consume a kept recovery by Apply or Continue (${guarded ? 'current' : 'older'} backend)`, async () => {
    const srv = sharedRecord({ guarded });
    const b = await appliedInB(srv);
    await b.s.poll();
    assert.equal(b.s.state.revision, srv.revision);
    // B also has an unsaved drawing change, and a new Canvas name typed of its own.
    b.ed.pending = '{"b":1}';
    const c = await srv.open('C', Session192);
    await b.s.poll();
    await c.s.poll();
    // The old editor does what its banner offers for B's offer: Apply when it has a change,
    // else Continue here (its old code dismisses an offer when nothing in it was taken up).
    const offer = c.s.offers().find((o) => o.from_here === false && !o.applied);
    if (offer.changed) await c.s.applyOffer(offer.id);
    else await c.s.resumeOffer(offer.id);
    // B polls (and answers again if its offer went without a receipt), then leaves.
    await b.s.poll();
    const where = srv.holders([b], NAME_D.text);
    assert.ok(where.inRecord || where.inViews, `the raw packet is still somewhere: ${JSON.stringify(where)}`);
    const left = await b.s.drain();
    if (left === 'clean') {
      assert.ok(srv.offers.some((o) => o.from === 'B' && o.unfinished.some(isRec)), 'clean only while the record holds it');
    }
    assert.equal(srv.acts.filter(([v, a]) => v === 'C' && a === 'apply').length, 0, 'the old editor applied nothing of it');
  });
}

test('R194 (R83-03): on the current backend the run-192 editor can\'t dismiss a recovery either; it stays in the record when its view closes', async () => {
  const srv = sharedRecord();
  const b = await appliedInB(srv);
  const c = await srv.open('C', Session192);
  await b.s.poll();
  assert.equal(await b.s.drain(), 'clean', 'B closes once the record holds it');
  await c.s.poll();
  const offer = c.s.offers()[0];
  // Its Dismiss (and its Continue here, which dismisses an offer it can't take anything from).
  assert.equal(await c.s.dismissOffer(offer.id), 'failed');
  assert.equal(await c.s.resumeOffer(offer.id), 'untaken');
  assert.ok(srv.offers.some((o) => o.id === offer.id), 'still in the record');
  assert.deepEqual(srv.resolved, []);
});

// ---- R84-01 (run 195): an older producer (the exact run-193 Session) and an older consumer
// (the exact run-192 Session), each through ITS OWN older server, sharing one canvas's handover
// record (its raw bytes) with this version's server and Session.

/**
 * One canvas's records as bytes, and two servers' handover over them: `old` is the server of
 * runs 192/193 (transcribed from their library.rs and lib.rs, which the Rust tests run as is:
 * `library_compat_tests.rs`), `current` this version's (`format: 2`, run 195) or run 194's
 * (`format: 1`: the older form plus receipts, removed when empty), with its guards.
 */
function sharedBytes({ format = 2 } = {}) {
  const k = { bytes: null, editor: null, revision: '"r1"', text: 'v1', next: 0, acts: [], answers: [], kept: [] };
  const hex = () => (++k.next).toString(16).padStart(16, '0');
  const isId = (i) => typeof i === 'string' && /^[0-9a-f]{16}$/.test(i);
  const u64 = (n) => Number.isInteger(n) && n >= 0;
  // library.rs `from_json` of runs 192/193 (2082–2115): `offers` and `pending` at the top, every
  // field present; anything else (an unknown key too) is ignored.
  const readTop = (v) => {
    if (!v || !Array.isArray(v.offers) || !Array.isArray(v.pending)) return null;
    const offers = v.offers.map((o) => (o && isId(o.id) && typeof o.from === 'string' && typeof o.base === 'string'
      && typeof o.canvas === 'boolean' && typeof o.changed === 'boolean' && Array.isArray(o.unfinished) && u64(o.at)
      ? { id: o.id, from: o.from, base: o.base, canvas: o.canvas, changed: o.changed, unfinished: o.unfinished, at: o.at } : null));
    const pending = v.pending.map((p) => (p && isId(p.id) && typeof p.from === 'string' && u64(p.since)
      && typeof p.ended === 'boolean' ? { id: p.id, from: p.from, since: p.since, ended: p.ended } : null));
    if (offers.includes(null) || pending.includes(null)) return null;
    const resolved = v.resolved === undefined ? [] : Array.isArray(v.resolved) ? v.resolved : null;
    return resolved && { offers, pending, resolved };
  };
  const parse = () => { try { return JSON.parse(k.bytes); } catch { return undefined; } };
  const old = {
    // `handover_of` 376–386: none is empty; unreadable is an error.
    read: () => (k.bytes === null ? { offers: [], pending: [], resolved: [] } : readTop(parse())),
    // `to_json` 2068–2079 (receipts dropped) and `set_handover` 390–402 (removed when empty).
    write: (h) => {
      k.bytes = h.offers.length || h.pending.length ? JSON.stringify({ offers: h.offers, pending: h.pending }) : null;
    },
    receipts: false,
  };
  const current = {
    read: () => {
      if (k.bytes === null) return { offers: [], pending: [], resolved: [] };
      const v = parse();
      if (format === 1 || !v || !('format' in v)) return readTop(v);
      return v.format === 2 ? readTop(v.handover) : null;
    },
    write: (h) => {
      if (format === 1) {
        k.bytes = h.offers.length || h.pending.length || h.resolved.length ? JSON.stringify(h) : null;
      } else {
        k.bytes = JSON.stringify({ format: 2, handover: h });
      }
    },
    receipts: true,
  };
  const recoveryIds = (o) => o.unfinished.filter((c) => c && c.kind === 'recovery' && typeof c.id === 'string').map((c) => c.id);
  const deps = (view, be) => {
    const unreadable = { ok: false, status: 403, body: { error: 'this canvas\'s handover record is unreadable' } };
    // `handover` 1893–1917 (a hint: null when unreadable or empty).
    const shown = () => {
      const h = be.read();
      if (!h || (!h.offers.length && !h.pending.length && !h.resolved.length)) return null;
      return {
        offers: h.offers.map((o) => ({ ...JSON.parse(JSON.stringify(o)), changed: o.changed && o.canvas, from_here: o.from === view })),
        pending: h.pending.map((p) => ({ ...p, from_here: p.from === view })),
        ...(be.receipts ? { resolved: JSON.parse(JSON.stringify(h.resolved)) } : {}),
      };
    };
    // `remove_offer` 1943–1960 (this version's with its receipts, run 194).
    const remove = (o, resolved) => {
      const h = be.read();
      h.offers = h.offers.filter((x) => x.id !== o.id);
      if (be.receipts) for (const r of recoveryIds(o)) if ((resolved || []).includes(r)) h.resolved.push({ offer: o.id, recovery: r });
      be.write(h);
    };
    // lib.rs `apply_offer`/`keep_offer`/`dismiss_offer` (891–970): the editing view, the offer
    // under its hold, then (this version, run 194) its guard.
    const find = (id, resolved) => {
      if (k.editor !== view) return { refused: { ok: false, status: 423 } };
      const h = be.read();
      if (!h) return { refused: unreadable };
      const o = h.offers.find((x) => x.id === id);
      if (!o) return { refused: { ok: false, status: 404 } };
      if (be.receipts && !recoveryIds(o).every((r) => Array.isArray(resolved) && resolved.includes(r))) {
        return { refused: { ok: false, status: 426 } };
      }
      return { o };
    };
    return {
      revision: async () => ({ revision: k.revision, editing: k.editor === view, handover: shown(), view: null }),
      fetchPage: async () => ({ text: k.text, revision: k.revision, editing: k.editor === view }),
      put: async (snap, rev) => {
        if (k.editor !== view) return { ok: false, status: 423, revision: null };
        if (rev !== k.revision) return { ok: false, status: 409, revision: k.revision };
        k.revision = `"s${hex()}"`;
        return { ok: true, status: 200, revision: k.revision };
      },
      // `answer` 1839–1889 (and lib.rs `check_offer`: this version refuses a mixed one, 400).
      answer: async (offer, base) => {
        k.answers.push({ view, offer: JSON.parse(JSON.stringify(offer)), base });
        if (k.editor === view) return { ok: false, status: 403 };
        if (be.receipts && offer && offer.unfinished.some((c) => c && c.kind === 'recovery')
          && (offer.canvas || offer.changed || offer.unfinished.some((c) => !c || c.kind !== 'recovery'))) {
          return { ok: false, status: 400 };
        }
        const h = be.read();
        if (!h) return unreadable;
        h.pending = h.pending.filter((p) => p.from !== view);
        h.offers = h.offers.filter((o) => o.from !== view);
        let made = null;
        if (offer) {
          made = hex();
          h.offers.push({ id: made, from: view, base, canvas: Boolean(offer.canvas), changed: offer.changed,
            unfinished: JSON.parse(JSON.stringify(offer.unfinished)), at: 1 });
        }
        be.write(h);
        return { ok: true, status: 200, body: { offer: made } };
      },
      applyOffer: async (id, resolved) => {
        k.acts.push([view, 'apply', id]);
        const { o, refused } = find(id, resolved);
        if (refused) return refused;
        if (!o.changed || !o.canvas) return { ok: false, status: 404 };
        if (o.base !== k.revision) return { ok: false, status: 409 };
        k.revision = `"applied${hex()}"`;
        k.text = 'applied';
        remove(o, resolved);
        return { ok: true, status: 200, body: { applied: true, revision: k.revision } };
      },
      keepOffer: async (id, resolved) => {
        k.acts.push([view, 'keep', id]);
        const { o, refused } = find(id, resolved);
        if (refused) return refused;
        if (!o.canvas) return { ok: false, status: 404 };
        k.kept.push({ from: o.id });
        remove(o, resolved);
        return { ok: true, status: 200, body: { kept: { id: hex(), name: 'Plan (handed over)' } } };
      },
      dismissOffer: async (id, resolved) => {
        k.acts.push([view, 'dismiss', id]);
        const { o, refused } = find(id, resolved);
        if (refused) return refused;
        remove(o, resolved);
        return { ok: true, status: 200, body: {} };
      },
      stopWaiting: async (id) => {
        if (k.editor !== view) return { ok: false, status: 423 };
        const h = be.read();
        if (!h) return unreadable;
        if (!h.pending.some((p) => p.id === id)) return { ok: false, status: 404 };
        h.pending = h.pending.filter((p) => p.id !== id);
        be.write(h);
        return { ok: true, status: 200, body: {} };
      },
      // `claim` 1692–1757: taken; then the record, unless it can't be read (a note, untouched).
      claim: async () => {
        const before = k.editor;
        k.editor = view;
        const h = be.read();
        if (!h) return { ok: true, note: 'Editing was taken here, but the handover record couldn\'t be read' };
        h.pending = h.pending.filter((p) => p.from !== view);
        h.offers = h.offers.filter((o) => o.from !== view);
        if (before && before !== view && !h.pending.some((p) => p.from === before)) {
          h.pending.push({ id: hex(), from: before, since: 1, ended: false });
        }
        be.write(h);
        return { ok: true };
      },
    };
  };
  /** View `view` opens through server `be` ('old' or 'current') with Session `Ctor`. */
  k.open = async (view, be, Ctor) => {
    const d = deps(view, be === 'old' ? old : current);
    await d.claim();
    const ed = realEditor();
    const { s: made } = setup([ed]);
    const s = Ctor === Session ? made : new Ctor(made.deps);
    Object.assign(s.deps, d);
    await s.start({ id: `canvas-${view}` }, { text: k.text, revision: k.revision, editing: true }, false);
    assert.equal(s.frame(0), 'ready');
    return { s, ed, d };
  };
  /** A view's server alone (another view of an older or this version's process). */
  k.server = (view, be) => deps(view, be === 'old' ? old : current);
  k.record = () => (k.bytes === null ? null : JSON.parse(k.bytes));
  /** Whether the record's offers hold `text` (a raw packet's), whichever form it is in. */
  k.inRecord = (text) => k.bytes !== null && k.bytes.includes(JSON.stringify(text).slice(1, -1));
  return k;
}

const NAME_E = { kind: 'title', base: 'Canvas', text: 'E-195' };

/**
 * B (the run-193 Session, its own older server) applied A's offer, whose read after it failed:
 * NAME_D is kept, applied on R1 and unopened. B then loads R1 and makes its own change: an
 * unsaved drawing change (`changed`) or a Canvas name of its own (`unfinished`).
 */
async function oldBKeeps(k, variant) {
  const a = k.server('A', 'old');
  await a.claim();
  const b = await k.open('B', 'old', Session193);
  const sent = await a.answer({ canvas: '{"whole":"a"}', changed: true, unfinished: [NAME_D] }, k.revision);
  await b.s.poll();
  const fetch = b.s.deps.fetchPage;
  b.s.deps.fetchPage = async () => { throw new Error('offline'); };
  assert.equal(await b.s.applyOffer(sent.body.offer), 'applied-unopened');
  b.s.deps.fetchPage = fetch;
  await b.s.poll();
  assert.equal(b.s.state.revision, k.revision, 'B shows R1 (A\'s change applied), NAME_D still kept');
  const r1 = k.revision;
  if (variant === 'changed') b.ed.pending = '{"b":1}';
  else b.ed.field = NAME_E;
  return { b, r1 };
}

test('R195: the older producer and reader fixtures are the exact frozen sources', () => {
  for (const [path, length, sha] of [
    ['./legacy/session-193.mjs', 56063, 'd4276a964ecc85f912dae74b7641bf12e01ac0173cccc86d8c48decde1a34683'],
    ['../../openpaperdesign-serve/src/library_192.rs', 92310, '1e1c2595889e3544307ff8980fdf650d8924955126d10452630081cca5fc7ff2'],
  ]) {
    const bytes = readFileSync(new URL(path, import.meta.url));
    assert.equal(bytes.length, length, path);
    assert.equal(createHash('sha256').update(bytes).digest('hex'), sha, path);
  }
});

// Each schedule runs on run 194's record form too (`format: 1`), where it is R84-01's loss: the
// same old code then consumes the recovery and B lets its copy go.
for (const [variant, format] of [['changed', 1], ['unfinished', 1], ['changed', 2], ['unfinished', 2]]) {
  test(`R195 (R84-01): once this version wrote the record, the run-193 producer's mixed answer through its own server is refused before anything is written, and the run-192 reader on its own finds nothing to consume (${variant}, record form ${format})`, async () => {
    const k = sharedBytes({ format });
    const { b, r1 } = await oldBKeeps(k, variant);
    const kept = [{ kind: 'recovery', v: 1, id: k.acts[0][2], state: 'applied', revision: r1, packets: [NAME_D] }];
    assert.deepEqual(b.s.recoveries(), kept);
    // C (this version) opens normally and takes the editing; it hasn't adopted anything of B's.
    const c = await k.open('C', 'current', Session);
    const joined = k.bytes;
    // B, read-only now, answers through its own server: its mixed offer.
    await b.s.poll();
    const sent = k.answers.at(-1);
    assert.equal(sent.view, 'B');
    assert.deepEqual(sent.offer.unfinished, variant === 'changed' ? kept : [NAME_E, ...kept], 'the run-193 producer mixes them');
    assert.equal(sent.offer.changed, variant === 'changed');
    // D (the run-192 Session, its own server) opens: the newest view.
    const d = await k.open('D', 'old', Session192);
    await d.s.poll();
    const offered = d.s.offers();
    for (const o of offered) {
      if (o.changed) await d.s.applyOffer(o.id);
      else await d.s.resumeOffer(o.id);
    }
    // B polls again (and answers again), then would leave.
    await b.s.poll();
    const outcome = {
      dOffered: offered.length,
      dActed: k.acts.filter(([v]) => v === 'D').map(([, a]) => a),
      recordAsCWroteIt: k.bytes === joined,
      bKeeps: b.s.recoveries(),
      inRecord: k.inRecord(NAME_D.text),
    };
    if (format === 1) {
      assert.deepEqual(outcome, { dOffered: 1, dActed: [variant === 'changed' ? 'apply' : 'dismiss'],
        recordAsCWroteIt: false, bKeeps: [], inRecord: false }, 'run 194: nowhere any more');
      return;
    }
    assert.deepEqual(outcome, { dOffered: 0, dActed: [], recordAsCWroteIt: true, bKeeps: kept, inRecord: false });
    assert.equal(k.answers.filter((a) => a.view === 'B').length >= 2, true, 'B answered again, refused again');
    // Exactly as kept, still counted as B's only copy (never acknowledged): not clean.
    assert.equal(b.s.holdsWork(), true);
    assert.deepEqual(variant === 'changed' ? b.ed.pending : b.ed.field, variant === 'changed' ? '{"b":1}' : NAME_E, 'B\'s own work is still there');
    assert.equal(await b.s.drain(), 'readonly', 'leaving B says it holds work');
    b.s.resume();
    await c.s.poll();
    assert.deepEqual(c.s.offers(), []);
  });

  test(`R195 (R84-01): a mixed offer the run-193 producer handed over before this version joined is carried over whole; the run-192 reader can't consume it, and this version's newest view recovers it from the record (${variant}, record form ${format})`, async () => {
    const k = sharedBytes({ format });
    const { b, r1 } = await oldBKeeps(k, variant);
    const id = k.acts[0][2];
    const kept = { kind: 'recovery', v: 1, id, state: 'applied', revision: r1, packets: [NAME_D] };
    // Another older view takes the editing; B answers through its own server: acknowledged.
    await k.server('X', 'old').claim();
    await b.s.poll();
    assert.equal(b.s.state.readonly, true);
    const m = k.record().offers.find((o) => o.from === 'B');
    assert.deepEqual(m.unfinished, variant === 'changed' ? [kept] : [NAME_E, kept]);
    // C (this version) opens: the record is written again, M in it.
    await k.open('C', 'current', Session);
    const carried = k.bytes;
    // D (run 192, its own server) opens and does what its banner offers.
    const d = await k.open('D', 'old', Session192);
    await d.s.poll();
    const offered = d.s.offers();
    for (const o of offered) {
      if (o.changed) await d.s.applyOffer(o.id);
      else await d.s.resumeOffer(o.id);
    }
    // Even asked directly, its server consumes nothing.
    const direct = await Promise.all([d.d.applyOffer(m.id), d.d.keepOffer(m.id), d.d.dismissOffer(m.id)]);
    // B's next poll (its server shows no record now) lets its copy go.
    await b.s.poll();
    const outcome = { dOffered: offered.length, direct: direct.map((r) => r.status), recordAsCWroteIt: k.bytes === carried,
      bKeeps: b.s.recoveries().length, inRecord: k.inRecord(NAME_D.text) };
    if (format === 1) {
      assert.deepEqual(outcome, { dOffered: 1, direct: [404, 404, 404], recordAsCWroteIt: false, bKeeps: 0, inRecord: false },
        'run 194: nowhere any more');
      return;
    }
    assert.deepEqual(outcome, { dOffered: 0, direct: [403, 403, 403], recordAsCWroteIt: true, bKeeps: 0, inRecord: true },
      'the record holds it');
    // In this version's form, M whole.
    assert.equal(k.record().format, 2);
    assert.deepEqual(k.record().handover.offers.find((o) => o.id === m.id), m);
    // The newest view of this version: M exactly, without B's or D's UI.
    const e = await k.open('E', 'current', Session);
    await e.s.poll();
    const atE = e.s.offers().find((o) => o.id === m.id);
    assert.deepEqual(atE.unfinished, m.unfinished, 'the raw envelope: id, state, revision, packets');
    if (variant === 'changed') {
      assert.equal(await e.s.applyOffer(m.id), 'applied');
      assert.deepEqual(e.s.appliedPending(), [{ id, unfinished: [NAME_D], applied: true }], 'held here, as it was');
      assert.equal(await e.s.resumeOffer(id), 'untaken', 'only on its own revision, never E\'s commit');
      assert.equal(await e.s.drain(), 'pending');
      assert.deepEqual(k.record().handover.resolved, [{ offer: m.id, recovery: id }]);
    } else {
      // Two Canvas names (B's E and the kept D) can't both be open here: one Canvas name field
      // (the adapter's `DRAFT_HERE`, run 196; before, the stand-in editor took both). E opens;
      // D isn't opened over it, and the offer stays in the record as it was, no receipt.
      assert.equal(await e.s.resumeOffer(m.id), 'partial');
      assert.deepEqual(e.ed.resumed, [NAME_E], 'E, exactly');
      assert.match(e.s.lastRefusals.join(' '), /being typed here/);
      assert.deepEqual(k.record().handover.offers.find((o) => o.id === m.id), m, 'still listed, whole');
      assert.deepEqual(k.record().handover.resolved, []);
      assert.equal(k.inRecord(NAME_D.text), true);
    }
    assert.equal(k.record().format, 2);
  });
}

test('R195 (R84-01): emptied, the record keeps its form; an older server still answers nothing into it', async () => {
  const k = sharedBytes();
  const c = await k.open('C', 'current', Session);
  assert.deepEqual(k.record(), { format: 2, handover: { offers: [], pending: [], resolved: [] } });
  const b = await k.open('B', 'old', Session193);
  await c.s.poll();
  assert.equal(c.s.state.readonly, true);
  assert.equal(await c.s.claim(), 'editing');
  await c.s.poll();
  const waiting = k.record().handover.pending[0];
  assert.equal(await c.s.stopWaiting(waiting.id), 'stopped');
  const empty = k.bytes;
  assert.deepEqual(k.record(), { format: 2, handover: { offers: [], pending: [], resolved: [] } });
  b.ed.pending = '{"b":1}';
  await b.s.poll();
  assert.equal(k.answers.at(-1).view, 'B');
  assert.equal(k.bytes, empty, 'refused before anything was written');
  assert.equal(await b.s.drain(), 'readonly');
});

// ---- R86-01 (run 197): Continue here of an offer whose two Canvas names can't be open at once
// (an ordinary E and a kept recovery's D, the admitted legacy mixed form): E opens, D waits; once
// E is finished or cancelled, the same view continues D alone, exactly, and only then is the offer
// accounted for. The adapter's half (real egui: the refusal, Escape, D alone) is
// `state_tests.rs` `r197_a_second_canvas_name_waits_and_then_continues_alone`.

const R197_E = { kind: 'title', base: 'Plan', text: 'E-197 — æøå' };
const R197_D = { kind: 'title', base: 'Plan', text: 'D-197 — recovered æøå' };

async function r197Partial() {
  const srv = sharedRecord();
  const c = await srv.open('C');
  const recovery = { kind: 'recovery', v: 1, id: 'original197-title', state: 'applied', revision: srv.revision, packets: [R197_D] };
  const m = { id: 'm', from: 'B193', changed: false, canvas: false, base: srv.revision, unfinished: [R197_E, recovery] };
  srv.offers.push(JSON.parse(JSON.stringify(m)));
  await c.s.poll();
  let t = 0;
  c.s.deps.wait = async () => { c.s.frame((t += 16)); };
  // E opens; D is refused by the one Canvas name draft (the adapter's DRAFT_HERE).
  assert.equal(await c.s.resumeOffer('m'), 'partial');
  assert.deepEqual(c.ed.resumed, [R197_E]);
  assert.match(c.s.lastRefusals.join(' '), /being typed here/);
  assert.deepEqual(srv.offers, [m], 'the whole offer, as it was');
  assert.deepEqual(srv.resolved, []);
  return { srv, c, m, recovery };
}

test('R197 (R86-01): after E is cancelled, the same view continues the waiting D alone, exactly, and only then accounts for the offer', async () => {
  const { srv, c, m } = await r197Partial();
  // Still continuable here, and said to be partly open: E opened, D waiting.
  assert.equal(c.s.continued.has('m'), false);
  assert.deepEqual(c.s.offerProgress('m'), { opened: [R197_E], waiting: [R197_D] });
  // While E is open, Continue again opens nothing over it (and never E twice).
  assert.equal(await c.s.resumeOffer('m'), 'unsaved');
  assert.deepEqual(c.ed.resumed, [R197_E]);
  // The person cancels E (Escape): nothing was saved, Plan and R1 as they were.
  c.ed.field = null;
  assert.equal(c.ed.can_reload(), true);
  const r1 = srv.revision;
  // Continue here again: D alone, its raw packet exactly; E isn't taken up again.
  assert.equal(await c.s.resumeOffer('m'), 'resumed');
  assert.deepEqual(c.ed.resumed, [R197_E, R197_D]);
  assert.deepEqual(c.ed.field, R197_D);
  assert.equal(srv.revision, r1, 'nothing saved');
  // Every intention accounted for: the offer goes, with the kept recovery's receipt.
  assert.deepEqual(srv.offers, []);
  assert.deepEqual(srv.resolved, [{ offer: 'm', recovery: 'original197-title' }]);
  assert.equal(c.s.continued.has('m'), true);
  // D is being typed here: leaving isn't clean.
  assert.equal(c.s.holdsWork(), true);
  assert.equal(srv.acts.filter(([, a]) => a === 'apply').length, 0);
  assert.ok(m);
});

test('R197 (R86-01): after E was finished and saved, the waiting D (typed over the old name, on R1) is refused without effects and stays listed', async () => {
  const { srv, c, m } = await r197Partial();
  // E finished (Enter) and saved: a newer accepted version.
  c.ed.field = null;
  c.ed.pending = '{"name":"E-197 — æøå"}';
  assert.equal(await c.s.save(), 'saved');
  const r2 = srv.revision;
  assert.equal(await c.s.resumeOffer('m'), 'untaken');
  assert.match(c.s.lastRefusals.join(' '), /newer change was saved/);
  assert.deepEqual(c.ed.resumed, [R197_E], 'D never opened over the newer name');
  assert.deepEqual(srv.offers, [m], 'still listed, whole: not lost');
  assert.deepEqual(srv.resolved, []);
  assert.equal(srv.revision, r2);
  assert.deepEqual(c.s.offerProgress('m'), { opened: [R197_E], waiting: [R197_D] });
});

// ---- Run 196 (BRIEF-196 B): the parent's run-195 runtime. Continue here took up a legacy
// mixed offer's Paper name and its kept recovery's Canvas name; the frames after it finished the
// Paper name, unasked, and the offer was already dismissed. The adapter's part is
// `state_tests.rs` `r196_two_names_continued_together_stay_unfinished_until_each_is_finished`
// (real egui frames); here: the offer is dismissed only once both are still open after frames.

const R196_E = { kind: 'page', page: '2542e8f2749f19ed', base: 'Paper 1', text: 'E195 — Paper æøå' };
const R196_D = { kind: 'title', base: 'OPD23 protocol195 — disposable æøå', text: 'D195 — recovered æøå' };

for (const finishes of [false, true]) {
  test(`R196: Continue here dismisses a mixed offer of two names only once both are still open after the editor's frames (${finishes ? 'a frame finishes one' : 'both stay open'})`, async () => {
    const srv = sharedRecord();
    const c = await srv.open('C');
    const recovery = { kind: 'recovery', v: 1, id: 'original195-title', state: 'applied', revision: srv.revision, packets: [R196_D] };
    srv.offers.push({ id: 'm', from: 'B193', changed: false, canvas: false, base: srv.revision, unfinished: [R196_E, recovery] });
    await c.s.poll();
    if (finishes) {
      // As the run-195 adapter's frames did: the Paper's name finished (no longer handed on).
      const frame = c.ed.frame.bind(c.ed);
      c.ed.frame = (t) => {
        frame(t);
        c.ed.held = c.ed.held.filter((x) => x.kind !== 'page');
        if (c.ed.field && c.ed.field.kind === 'page') c.ed.field = null;
      };
    }
    let t = 0;
    c.s.deps.wait = async () => { c.s.frame((t += 16)); };
    const r = await c.s.resumeOffer('m');
    assert.deepEqual(c.ed.resumed, [R196_E, R196_D], 'both raw packets, exactly, in order');
    if (finishes) {
      assert.equal(r, 'partial');
      assert.match(c.s.lastRefusals.join(' '), /finished or changed before it was open/);
      assert.deepEqual(srv.offers.map((o) => o.id), ['m'], 'still listed');
      assert.deepEqual(srv.offers[0].unfinished, [R196_E, recovery], 'as it was');
      assert.deepEqual(srv.resolved, [], 'no receipt: B keeps its copy');
      assert.equal(srv.acts.filter(([, a]) => a === 'dismiss').length, 0);
    } else {
      assert.equal(r, 'resumed');
      assert.deepEqual(srv.offers, []);
      assert.deepEqual(srv.resolved, [{ offer: 'm', recovery: 'original195-title' }]);
    }
  });
}

test('issue #23 (R78-03): Continue here takes up an unfinished-only offer and then dismisses it', async () => {
  const { s, ed, server } = await viewSession();
  server.handover = {
    offers: [{ id: 'u', from_here: false, changed: false, canvas: false, base: '"r1"',
      unfinished: [{ kind: 'text', page: 'p1', node: 7, text: 'Hel', caret: 3 }, { kind: 'field' }] }],
    pending: [],
  };
  assert.equal(await s.poll(), 'handover');
  assert.equal(await s.applyOffer('u'), undefined, 'nothing finished to apply');
  assert.equal(await s.resumeOffer('u'), 'resumed');
  assert.deepEqual(ed.typing, { kind: 'text', page: 'p1', node: 7, text: 'Hel', caret: 3 }, 'the text edit is open here');
  assert.deepEqual(server.acts, [['dismiss', 'u']]);
  assert.equal(s.offers().length, 0);
});

test('R186: Edit here says the server\'s own outcome: taken with its note, or refused with its reason (busy, closed, fenced), never a bare conflict', async () => {
  const { s, server } = await viewSession();
  const seen = [];
  const ui = {
    status: (t) => seen.push(t), hideReadonly: () => {}, handover: () => {}, readonly: () => {},
  };
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  // Refused: each answer's own reason and status reach the person.
  for (const [status, error] of [
    [503, 'the canvas is busy right now; nothing was changed. Try again'],
    [410, 'this view of the canvas was closed here'],
    [503, 'writes are disabled: the library is Broken'],
  ]) {
    s.deps.claim = async () => ({ ok: false, status, error });
    assert.equal(await claimStep(s, ui), 'refused');
    assert.equal(s.lastClaim.status, status);
    assert.equal(seen.at(-1), `Editing wasn't taken here: ${error}`);
    assert.equal(s.state.readonly, true, 'still read-only');
  }
  // An old-style bare status says only that it didn't work.
  s.deps.claim = async () => ({ ok: false, status: 409, error: 'claim 409' });
  assert.equal(await claimStep(s, ui), 'refused');
  assert.equal(seen.at(-1), 'Editing couldn\'t be taken back here; try again');
  // Taken, with what of its bookkeeping didn't happen (committed: it edits here now).
  s.deps.claim = async () => { server.editing = true; return { ok: true, note: 'Editing was taken here, but its handover record could not be written' }; };
  assert.equal(await claimStep(s, ui), 'editing');
  assert.equal(s.state.readonly, false);
  assert.equal(seen.at(-1), `${OPD_EDITING_STATUS}. Editing was taken here, but its handover record could not be written`);
});

test('R186 (R78-03): a field value\'s outcome is only ever read from the editor that took it up', async () => {
  const { s, ed, server } = await viewSession();
  const field = { kind: 'theme-metric', style: { size: 2000, bold: false, italic: false, family: 'Inter' }, metric: 'line-height', page: 'p1', text: '15' };
  server.handover = { offers: [{ id: 'f', from_here: false, changed: false, canvas: false, base: '"r1"', unfinished: [field] }], pending: [] };
  assert.equal(await s.poll(), 'handover');
  ed.resume_unfinished = () => '';
  ed.resume_outcome = () => 'still opening';
  assert.equal(await s.resumeOffer('f'), 'resumed');
  assert.equal(s.lastFieldTaken, true);
  assert.deepEqual(server.acts, [], 'its offer stays listed');
  assert.equal(await s.resumeOffer('f'), undefined, 'not taken up twice');
  // Still opening when the editor is replaced: nothing is said for the other one.
  const waits = [];
  s.deps.wait = async () => { waits.push(1); s.ed = realEditor(); };
  assert.equal(await s.fieldOutcome('f'), undefined);
  assert.equal(waits.length, 1);
  assert.ok(s.continued.has('f'), 'unchanged');
  // The same editor's reason: then it can be continued again.
  s.ed = ed;
  ed.resume_outcome = () => 'its field isn\'t shown here (the layers or the panel aren\'t), so it wasn\'t opened';
  assert.match(await s.fieldOutcome('f'), /isn't shown here/);
  assert.ok(!s.continued.has('f'));
  // Bounded: one that stays opening gives up quietly.
  ed.resume_outcome = () => 'still opening';
  s.deps.wait = async () => {};
  assert.equal(await s.fieldOutcome('f', 3), undefined);
});

test('issue #23 (R78-01): a view the server no longer knows can\'t save, claim or hand over, and says so', async () => {
  const { s, ed, server } = await viewSession();
  ed.pending = '{"mine":1}';
  server.view = 'released';
  assert.equal(await s.poll(), 'unidentified');
  assert.equal(s.state.readonly, true);
  assert.equal(await s.claim(), undefined, 'no Edit here');
  assert.equal(server.answers.length, 0, 'no handover from an unknown view');
  assert.equal(await s.save(), undefined);
  assert.equal(server.puts.length, 0);
  assert.equal(ed.pending, '{"mine":1}', 'still on screen');
});

test('issue #23: an unsaved change handed over stays here unsaved; Edit here then saves it itself on its own base, a conflict, never an overwrite', async () => {
  const { s, ed, server } = await viewSession();
  ed.pending = '{"mine":1}';
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  assert.equal(server.answers[0].base, '"r1"', 'on the revision it was made on');
  assert.equal(ed.pending, '{"mine":1}');
  assert.equal(server.puts.length, 0, 'no save of an unauthorized view');
  server.revision = '"theirs"';
  assert.equal(await s.poll(), 'handover', 'its own offer shows in the record');
  assert.equal(s.state.behind, true, 'and the save made there is learned, nothing loaded');
  assert.equal(await s.poll(), undefined);
  assert.equal(server.answers.length, 1, 'the same is answered once');
  assert.equal(await s.claim(), 'editing');
  assert.equal(s.state.handed, null);
  assert.deepEqual(s.state.handover.offers, [], 'its own offer left with the claim');
  assert.equal(await s.save(), 'conflict');
  assert.equal(server.puts.at(-1), '"r1"');
  assert.equal(server.revision, '"theirs"', 'nothing overwritten');
});

test('issue #23: a view in conflict learns read-only and hands its change over; leaving hands it over first', async () => {
  const { s, ed, server } = await viewSession();
  ed.pending = '{"mine":1}';
  server.revision = '"r2"';
  assert.equal(await s.save(), 'conflict');
  server.editing = false;
  server.handover = { offers: [{ id: 'other', from_here: false, changed: true, base: '"r0"', unfinished: [] }], pending: [] };
  assert.equal(await s.poll(), 'readonly', 'a conflict no longer hides the loss of the editing');
  assert.equal(server.answers.length, 1, 'several views\' offers wait side by side');
  assert.equal(s.state.conflict, true, 'the conflict stays');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')));
  // Leaving: answered already, still not clean.
  assert.equal(await s.drain(), 'conflict');
  assert.equal(ed.pending, '{"mine":1}', 'kept here too');
});

test('issue #23: a read-only view can\'t act on what others handed over', async () => {
  const { s, server } = await viewSession();
  server.editing = false;
  server.handover = { offers: [{ id: 'a', from_here: false, changed: true, canvas: true, base: '"r1"', unfinished: [] }], pending: [] };
  assert.equal(await s.poll(), 'readonly');
  assert.equal(await s.applyOffer('a'), undefined);
  assert.equal(await s.keepOffer('a'), undefined);
  assert.equal(await s.dismissOffer('a'), undefined);
  assert.deepEqual(server.acts, []);
});

test('issue #23: Edit here brings a conflict banner back, and the handover steps say what happened', async () => {
  const { s, ed, server } = await viewSession();
  ed.pending = '{"mine":1}';
  server.revision = '"r2"';
  assert.equal(await s.save(), 'conflict');
  server.editing = false;
  assert.equal(await s.poll(), 'readonly');
  const seen = [];
  const ui = {
    status: (t) => seen.push(`status:${t}`), conflict: () => seen.push('conflict'),
    hideReadonly: () => seen.push('hideReadonly'), handover: () => seen.push('handover'),
    readonly: () => seen.push('readonly'),
  };
  assert.equal(await claimStep(s, ui), 'editing');
  assert.deepEqual(seen, ['hideReadonly', 'conflict', 'status:Editing here again']);
  // Handover steps, in an editing view that is clean.
  assert.equal(await s.discard(), 'discarded');
  seen.length = 0;
  server.revision = '"r2"';
  s.state.revision = '"r2"';
  s.state.handover = { offers: [{ id: 'x', from_here: false, changed: true, canvas: true, base: '"r0"', unfinished: [] }], pending: [] };
  server.handover = s.state.handover;
  assert.equal(await offerStep(s, ui, 'apply', 'x'), 'conflict');
  assert.equal(await offerStep(s, ui, 'dismiss', 'x'), 'dismissed');
  assert.deepEqual(seen, [
    'handover',
    'status:Not applied: the canvas changed since that change was made; nothing was overwritten. Keep it as a new canvas, or dismiss it',
    'handover', 'status:Dismissed the change handed over',
  ]);
});
