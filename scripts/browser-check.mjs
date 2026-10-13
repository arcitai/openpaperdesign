#!/usr/bin/env node
// Drive a real headless Chromium against a running openpaperdesign-serve and check what
// happened. Dependency-free (Node >= 22 for the global WebSocket), talks CDP directly.
//
//   node scripts/browser-check.mjs <url-with-#token> <out-dir> [edit|verify] [chromium] [expect.json]
//
// edit:   drag a node, draw a rectangle with the toolbar tool, Delete it, undo (Ctrl+Z), redo
//         and undo (toolbar), screenshot, reload. Ends with the moved node and the new
//         rectangle on the page. Every step is asserted.
// agent:  an agent edits over MCP while the page is open. Its new shape must appear without a
//         reload; then the page drags a node (saved), the agent moves its shape, and both
//         changes must be in the page and survive a reload. Needs the same URL (its token).
// verify: load, report the backend and top-level nodes, screenshot. With expect.json (the
//         result-edit.json of an earlier edit run, e.g. before a server restart) the moved
//         node, the rectangle and the revision must match it.
// Any failed assertion, console error (including GPU validation messages), page exception,
// missing GPU adapter or WebGPU-to-WebGL fallback fails the run (exit 1). The canvas must keep
// its size and position through every save. Results, including the backend that ran, go to
// <out-dir>/result-<mode>.json. The chromium sandbox stays on.
// Default backend: WebGPU when the browser offers it. For WebGL2 put `?backend=webgl` before
// the `#token=` fragment of the URL.
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { McpClient } from './mcp-client.mjs';

const [url, outDir, mode = 'edit', chromium = '/usr/bin/chromium', expectFile] = process.argv.slice(2);
if (!url || !outDir || !['edit', 'verify', 'agent'].includes(mode)) {
  console.error('usage: browser-check.mjs <url> <out-dir> [edit|verify|agent] [chromium] [expect.json]');
  process.exit(2);
}
mkdirSync(outDir, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = [];
const errors = [];
const note = (m) => { log.push(m); console.error(m); };

class CheckFailed extends Error {}
function check(cond, msg, detail) {
  if (!cond) throw new CheckFailed(`${msg}${detail === undefined ? '' : `: ${JSON.stringify(detail)}`}`);
}
const near = (a, b, tol) => Math.abs(a - b) <= tol;
const sameRect = (a, b) => a && b && a.length === 4 && a.every((v, i) => near(v, b[i], 1e-6));

const profile = mkdtempSync(join(process.env.TMPDIR || tmpdir(), 'opd-chromium-'));
const browser = spawn(chromium, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', '--window-size=1280,860', 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let stderr = '';
browser.stderr.on('data', (d) => { stderr += d; });

async function devtoolsPort() {
  for (let i = 0; i < 100; i++) {
    try {
      return readFileSync(join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0];
    } catch { await sleep(100); }
    if (browser.exitCode !== null) throw new Error(`chromium exited ${browser.exitCode}: ${stderr}`);
  }
  throw new Error(`no DevToolsActivePort; chromium stderr:\n${stderr}`);
}

let ws; let nextId = 1; const pending = new Map();
function cdp(method, params = {}) {
  const id = nextId++;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject, method }));
}

async function ev(expression) {
  const r = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(`${expression}: ${JSON.stringify(r.exceptionDetails)}`);
  return r.result.value;
}

async function waitFor(expression, what, ms = 20000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const v = await ev(expression).catch(() => false);
    if (v) return v;
    await sleep(50);
  }
  const s = await ev('JSON.stringify(window.__opd || null)').catch(() => 'n/a');
  throw new CheckFailed(`timeout waiting for ${what}; state ${s}`);
}

const mouse = (type, x, y, extra = {}) => cdp('Input.dispatchMouseEvent',
  { type, x, y, button: 'left', buttons: type === 'mouseReleased' ? 0 : 1, clickCount: 1, ...extra });
async function drag(x0, y0, x1, y1, steps = 12) {
  await mouse('mouseMoved', x0, y0, { buttons: 0, button: 'none' });
  await mouse('mousePressed', x0, y0);
  for (let i = 1; i <= steps; i++) {
    await mouse('mouseMoved', x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps);
    await sleep(16);
  }
  await mouse('mouseReleased', x1, y1);
}
async function click(x, y) {
  await mouse('mouseMoved', x, y, { buttons: 0, button: 'none' });
  await mouse('mousePressed', x, y);
  await mouse('mouseReleased', x, y);
}
async function key(k, code, vk, modifiers = 0) {
  await cdp('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: k, code, windowsVirtualKeyCode: vk, modifiers });
  await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, modifiers });
}
async function clickButton(selector) {
  const r = await ev(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()`);
  await click(r[0], r[1]);
}

const nodes = async () => JSON.parse(await ev('window.__opdEditor.root_nodes()'));
const byId = (list, id) => list.find((n) => n.id === id);
const ids = (list) => list.map((n) => n.id).sort((a, b) => a - b);
const saves = () => ev('window.__opd.saves');
const canvasRect = () => ev('(() => { const r = document.getElementById("canvas").getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()');
let canvasAtLoad = null;
async function settled(before, what) {
  await waitFor(`window.__opd.saves > ${before} && !window.__opd.saving && !window.__opdEditor.dirty()`, what);
  check(!(await ev('window.__opd.conflict')), `unexpected conflict after ${what}`);
  const now = await canvasRect();
  check(JSON.stringify(now) === JSON.stringify(canvasAtLoad), `canvas moved or resized by ${what}`, { load: canvasAtLoad, now });
}
async function shot(name) {
  const { data } = await cdp('Page.captureScreenshot', { format: 'png' });
  const path = join(outDir, name);
  writeFileSync(path, Buffer.from(data, 'base64'));
  return path;
}
async function load() {
  await waitFor('window.__opd && (window.__opd.ready || window.__opd.error)', 'page ready', 60000);
  const st = JSON.parse(await ev('JSON.stringify(window.__opd)'));
  check(!st.error, 'page reported an error', st.error);
  check(st.backend, 'no GPU backend reported');
  check(!st.fallback, 'WebGPU failed and the page fell back to WebGL', st.fallback);
  return st;
}

const result = { mode, url: url.replace(/token=[0-9a-f]+/, 'token=<redacted>'), steps: [] };
try {
  const port = await devtoolsPort();
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((t) => t.type === 'page');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const p = pending.get(msg.id); pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`${p.method}: ${msg.error.message}`)); else p.resolve(msg.result);
    } else if (msg.method === 'Runtime.consoleAPICalled') {
      const text = msg.params.args.map((a) => a.value ?? a.description).join(' ');
      note(`console.${msg.params.type}: ${text}`);
      if (msg.params.type === 'error' || msg.params.type === 'assert') errors.push(`console.${msg.params.type}: ${text}`);
    } else if (msg.method === 'Runtime.exceptionThrown') {
      const d = msg.params.exceptionDetails;
      const text = d.exception?.description || d.text;
      note(`exception: ${text}`);
      errors.push(`exception: ${text}`);
    } else if (msg.method === 'Log.entryAdded' && msg.params.entry.level === 'error') {
      const e = msg.params.entry;
      note(`log.${e.source}: ${e.text}`);
      errors.push(`log.${e.source}: ${e.text}`);
    }
  };
  await cdp('Runtime.enable');
  await cdp('Log.enable');
  await cdp('Page.enable');
  result.userAgent = (await cdp('Browser.getVersion')).product;
  await cdp('Page.navigate', { url });
  const st = await load();
  result.backend = st.backend;
  result.requestedBackend = new URL(url).searchParams.get('backend') || 'default (WebGPU if offered)';
  result.revisionLoaded = st.revision;
  result.webgpuApi = await ev("typeof navigator.gpu !== 'undefined'");
  await sleep(300);
  const canvas = await canvasRect();
  canvasAtLoad = canvas;
  result.canvas = canvas;
  const at = (sx, sy) => [canvas[0] + sx, canvas[1] + sy];
  result.nodesLoaded = await nodes();
  check(result.nodesLoaded.length > 0, 'page has no top-level nodes');
  result.shotLoaded = await shot(`browser-${mode}-loaded.png`);

  if (mode === 'verify' && expectFile) {
    const want = JSON.parse(readFileSync(expectFile, 'utf8'));
    check(want.final, 'expect file has no final state', expectFile);
    check(result.nodesLoaded.length === want.final.count, 'node count differs from the edit run',
      [result.nodesLoaded.length, want.final.count]);
    check(sameRect(byId(result.nodesLoaded, want.final.moved.id)?.world, want.final.moved.world),
      'moved node differs from the edit run');
    check(sameRect(byId(result.nodesLoaded, want.final.rect.id)?.world, want.final.rect.world),
      'new rectangle differs from the edit run');
    check(st.revision === want.revision, 'revision differs from the edit run', [st.revision, want.revision]);
  }

  if (mode === 'edit') {
    // 1. Drag a childless top-level node that is fully on screen.
    const inView = (n) => n.screen[0] > 0 && n.screen[1] > 0 && n.screen[0] + n.screen[2] < canvas[2] - 100
      && n.screen[1] + n.screen[3] < canvas[3] - 100 && n.screen[2] > 24 && n.screen[3] > 24;
    const target = result.nodesLoaded.find((n) => n.children === 0 && inView(n))
      || result.nodesLoaded.find(inView);
    check(target, 'no top-level node on screen to drag');
    const zoom = target.screen[2] / target.world[2];
    const [cx, cy] = at(target.screen[0] + target.screen[2] / 2, target.screen[1] + target.screen[3] / 2);
    let n0 = await saves();
    await drag(cx, cy, cx + 80, cy + 60);
    await settled(n0, 'save after drag');
    const afterDrag = await nodes();
    const moved = byId(afterDrag, target.id);
    const dx = moved.world[0] - target.world[0];
    const dy = moved.world[1] - target.world[1];
    result.steps.push({ step: 'drag', node: target.id, kind: target.kind, before: target.world, after: moved.world });
    // Upstream snaps moves to edges/pixels, so allow a few screen pixels of slack.
    check(near(dx, 80 / zoom, 8 / zoom) && near(dy, 60 / zoom, 8 / zoom), 'drag did not move the node by the drag distance', { dx, dy, zoom });
    check(near(moved.world[2], target.world[2], 1e-6) && near(moved.world[3], target.world[3], 1e-6), 'drag resized the node');
    check(JSON.stringify(ids(afterDrag)) === JSON.stringify(ids(result.nodesLoaded)), 'drag changed the set of nodes');
    check(await ev('window.__opdEditor.selection_len()') === 1, 'dragged node is not selected');

    // 2. Rectangle tool from the toolbar, drawn on an empty spot.
    const covered = (x, y) => (n) => !(x + 160 < n.screen[0] || x > n.screen[0] + n.screen[2]
      || y + 120 < n.screen[1] || y > n.screen[1] + n.screen[3]);
    let spot = null;
    for (let y = 30; y < canvas[3] - 150 && !spot; y += 20) {
      for (let x = 30; x < canvas[2] - 190 && !spot; x += 20) {
        if (!afterDrag.some(covered(x, y))) spot = [x, y];
      }
    }
    check(spot, 'no empty spot for a new rectangle');
    await clickButton('[data-tool="rect"]');
    check(await ev('window.__opdEditor.tool()') === 'rect', 'toolbar did not select the rectangle tool');
    n0 = await saves();
    const [sx, sy] = at(spot[0] + 10, spot[1] + 10);
    await drag(sx, sy, sx + 140, sy + 100);
    await settled(n0, 'save after rectangle');
    const withRect = await nodes();
    const created = withRect.find((n) => !afterDrag.some((c) => c.id === n.id));
    result.steps.push({ step: 'create-rect', count: [afterDrag.length, withRect.length], node: created });
    check(withRect.length === afterDrag.length + 1 && created && created.kind === 'rect', 'rectangle not created', created);
    check(near(created.world[2], 140 / zoom, 8 / zoom) && near(created.world[3], 100 / zoom, 8 / zoom), 'rectangle size', created.world);

    // 3. Delete key removes the (selected) new rectangle.
    n0 = await saves();
    await key('Delete', 'Delete', 46);
    await settled(n0, 'save after delete');
    let now = await nodes();
    result.steps.push({ step: 'delete', count: now.length });
    check(!byId(now, created.id) && JSON.stringify(ids(now)) === JSON.stringify(ids(afterDrag)), 'Delete did not remove exactly the rectangle');

    // 4. Ctrl+Z brings it back; toolbar Redo removes it; toolbar Undo restores it.
    const restored = async (what) => {
      now = await nodes();
      result.steps.push({ step: what, count: now.length });
      check(sameRect(byId(now, created.id)?.world, created.world) && now.length === withRect.length,
        `${what} did not restore the rectangle`);
      check(sameRect(byId(now, target.id)?.world, moved.world), `${what} touched the dragged node`);
    };
    n0 = await saves();
    await key('z', 'KeyZ', 90, 2);
    await settled(n0, 'save after undo');
    await restored('undo (Ctrl+Z)');
    n0 = await saves();
    await clickButton('[data-action="redo"]');
    await settled(n0, 'save after redo');
    now = await nodes();
    result.steps.push({ step: 'redo (toolbar)', count: now.length });
    check(!byId(now, created.id) && now.length === afterDrag.length, 'toolbar Redo did not remove the rectangle');
    n0 = await saves();
    await clickButton('[data-action="undo"]');
    await settled(n0, 'save after toolbar undo');
    await restored('undo (toolbar)');
    result.final = { moved: byId(now, target.id), rect: byId(now, created.id), count: now.length };
    result.revision = await ev('window.__opd.revision');
    result.saves = await saves();
    result.shotEdited = await shot('browser-edit-done.png');

    // 5. Reload: the page comes back from disk with the same content and revision.
    await cdp('Page.reload', { ignoreCache: true });
    await sleep(200);
    const st2 = await load();
    await sleep(300);
    const reloaded = await nodes();
    result.reload = { count: reloaded.length, revision: st2.revision };
    check(st2.revision === result.revision, 'revision after reload differs from the last save', [st2.revision, result.revision]);
    check(reloaded.length === result.final.count, 'node count changed across reload');
    check(sameRect(byId(reloaded, target.id)?.world, result.final.moved.world), 'moved node differs after reload');
    check(sameRect(byId(reloaded, created.id)?.world, result.final.rect.world), 'rectangle differs after reload');
    result.shotReloaded = await shot('browser-edit-reloaded.png');
  }
  if (mode === 'agent') {
    const mcp = await new McpClient(url).connect();
    const pageId = await ev('window.__opd.pageId');
    const keyOf = (n) => `${pageId.slice(0, 6)}:${n}`;
    const idOf = (text) => Number(JSON.parse(text).id.split(':')[1]);
    await ev('window.__opdNoReload = 1');
    // 1. The agent adds a shape beside the content; the page shows it without reloading.
    const before = await nodes();
    const right = Math.max(...before.map((n) => n.world[0] + n.world[2])) + 80;
    const top = Math.min(...before.map((n) => n.world[1]));
    // An agent reads a page in its session before it may change it.
    const read = await mcp.call('get_basic_info', {});
    check(read.ok, 'agent get_basic_info failed', read.text);
    const made = await mcp.call('create_shape', { type: 'ellipse', x: right, y: top, width: 80, height: 60 });
    check(made.ok, 'agent create_shape failed', made.text);
    const agentNode = idOf(made.text);
    await waitFor(`JSON.parse(window.__opdEditor.root_nodes()).some((n) => n.id === ${agentNode})`, 'the agent shape in the page', 10000);
    check(await ev('window.__opdNoReload === 1'), 'the page reloaded instead of updating in place');
    result.steps.push({ step: 'agent create_shape shown live', node: agentNode });

    // 2. One after the other: the page drags one of its nodes (saved), then the agent reads
    //    and moves its shape. (Overlapping edits are refused by design and covered by unit tests.)
    const inView = (n) => n.id !== agentNode && n.children === 0 && n.screen[0] > 0 && n.screen[1] > 0
      && n.screen[0] + n.screen[2] < canvas[2] - 100 && n.screen[1] + n.screen[3] < canvas[3] - 100
      && n.screen[2] > 24 && n.screen[3] > 24;
    const now = await nodes();
    const target = now.find(inView);
    check(target, 'no node on screen to drag');
    const zoom = target.screen[2] / target.world[2];
    const [cx, cy] = at(target.screen[0] + target.screen[2] / 2, target.screen[1] + target.screen[3] / 2);
    const n0 = await saves();
    await drag(cx, cy, cx + 60, cy + 40);
    await settled(n0, 'the page save of the drag');
    // As a real agent must: read the page after the person's save before changing it (a
    // change to a page changed since the session last read it is refused as a conflict).
    const reread = await mcp.call('get_basic_info', {});
    check(reread.ok, 'agent get_basic_info failed', reread.text);
    const moved = await mcp.call('update_styles', { updates: [{ nodeIds: [keyOf(agentNode)], styles: { left: `${right + 200}px` } }] });
    check(moved.ok, 'agent update_styles failed', moved.text);
    await waitFor(`(() => { const n = JSON.parse(window.__opdEditor.root_nodes()).find((n) => n.id === ${agentNode}); return n && Math.abs(n.world[0] - ${right + 200}) < 0.5; })()`, 'the agent move in the page', 10000);
    const after = await nodes();
    const t2 = byId(after, target.id);
    check(Math.abs(t2.world[0] - target.world[0] - 60 / zoom) <= 8 / zoom, 'the page drag was lost', { before: target.world, after: t2.world });
    check(!(await ev('window.__opd.conflict')), 'unexpected conflict');
    result.steps.push({ step: 'page drag, then agent move', target: target.id, agentNode, saves: await saves() });

    // 3. Both survive a reload.
    await cdp('Page.reload', { ignoreCache: true });
    await sleep(200);
    await load();
    await sleep(300);
    const reloaded = await nodes();
    check(sameRect(byId(reloaded, target.id)?.world, t2.world), 'page drag lost after reload');
    check(Math.abs(byId(reloaded, agentNode)?.world[0] - (right + 200)) < 0.5, 'agent move lost after reload');
    result.shotAgent = await shot('browser-agent-done.png');
  }
  check(errors.length === 0, 'console errors or page exceptions', errors);
  result.ok = true;
} catch (e) {
  result.ok = false;
  result.error = String(e && e.stack && !(e instanceof CheckFailed) ? e.stack : e.message || e);
} finally {
  result.errors = errors;
  result.log = log;
  writeFileSync(join(outDir, `result-${mode}.json`), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  try { ws && ws.close(); } catch { /* closing */ }
  browser.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
  process.exit(result.ok ? 0 : 1);
}
