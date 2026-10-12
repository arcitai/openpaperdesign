// Tests for static/session.js with mocked editors, canvases and DOM (no browser, no wasm).
// Run: node --test crates/openpaperdesign-web/tests/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Session } from '../static/session.js';

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

// ---- a terminal error while a save is in flight (static/page.js saveStep + frameStep)

import { saveStep } from '../static/page.js';

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

test('never reloads over unsaved edits; the next save is merged by the server', async () => {
  const merged = { ok: true, status: 200, revision: '"r3"', merged: true, body: 'merged-page' };
  let sentWith = null;
  const { s, ed, server } = await readySession({
    put: async (snap, rev) => { sentWith = rev; return merged; },
  });
  ed.pending = '{"mine":1}'; // an unsaved local edit
  server.revision = '"r2"';
  server.text = 'agent-change';
  assert.equal(await s.poll(), 'behind');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')), 'unsaved work not replaced');
  assert.equal(s.state.behind, true);
  // The save goes out with this tab's base revision, and the merged result is loaded.
  assert.equal(await s.save(), 'merged');
  assert.equal(sentWith, '"r1"');
  assert.ok(ed.calls.includes('reload:merged-page'));
  assert.equal(s.state.revision, '"r3"');
});

test('edits made while a merged save was in flight keep the old base for the next merge', async () => {
  const answer = deferred();
  const { s, ed } = await readySession({ put: async () => answer.promise });
  ed.pending = '{"first":1}';
  const saving = s.save();
  ed.canReload = false; // another edit started meanwhile
  answer.resolve({ ok: true, status: 200, revision: '"r3"', merged: true, body: 'merged-page' });
  assert.equal(await saving, 'merged');
  assert.ok(!ed.calls.some((c) => c.startsWith('reload')), 'newer local edit not overwritten');
  assert.equal(s.state.revision, '"r1"', 'base kept, so the next save merges again');
});

test('a conflict is reported with its layers and stops saving and reloading', async () => {
  const { s, ed, server } = await readySession({
    put: async () => ({ ok: false, status: 409, conflicts: ['7', 'root'] }),
  });
  ed.pending = '{"mine":1}';
  assert.equal(await s.save(), 'conflict');
  assert.deepEqual(s.state.conflicts, ['7', 'root']);
  assert.ok(ed.calls.includes('save_failed'));
  server.revision = '"r9"';
  assert.equal(await s.poll(), undefined, 'no silent reload after a conflict');
  assert.equal(await s.save(), undefined, 'no more saves until the user decides');
});

test('no polling before Ready or while switching', async () => {
  const ed = editor('gpu', { ready: false });
  let asked = 0;
  const { s } = setup([ed], { revision: async () => { asked += 1; return '"r2"'; } });
  await s.start({ id: 'canvas0' }, { text: 'v1', revision: '"r1"' }, false);
  assert.equal(await s.poll(), undefined);
  assert.equal(asked, 0);
});
