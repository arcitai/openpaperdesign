#!/usr/bin/env node
// Drive a real headless Chromium against a running openpaperdesign-serve and record what
// happened. Dependency-free (Node >= 22 for the global WebSocket), talks CDP directly.
//
//   node scripts/browser-check.mjs <url-with-#token> <out-dir> [edit|verify] [chromium]
//
// edit:   drag a node, draw a rectangle with the toolbar tool, Delete it, undo (keyboard),
//         redo + undo (toolbar), screenshot, reload and re-read. Ends with the moved node and
//         the new rectangle on the page.
// verify: load, report the backend and top-level nodes, screenshot.
// Results go to <out-dir>/result-<mode>.json; the chromium sandbox is left on.
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const [url, outDir, mode = 'edit', chromium = '/usr/bin/chromium'] = process.argv.slice(2);
if (!url || !outDir) {
  console.error('usage: browser-check.mjs <url> <out-dir> [edit|verify] [chromium]');
  process.exit(2);
}
mkdirSync(outDir, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = [];
const note = (m) => { log.push(m); console.error(m); };

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
  throw new Error(`timeout waiting for ${what}; state ${s}`);
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
const saves = () => ev('window.__opd.saves');
async function settled(before, what) {
  await waitFor(`window.__opd.saves > ${before} && !window.__opd.saving && !window.__opdEditor.dirty()`, what);
}
async function shot(name) {
  const { data } = await cdp('Page.captureScreenshot', { format: 'png' });
  const path = join(outDir, name);
  writeFileSync(path, Buffer.from(data, 'base64'));
  return path;
}
async function load() {
  await waitFor('window.__opd && (window.__opd.ready || window.__opd.error)', 'page ready', 60000);
  const st = await ev('JSON.stringify(window.__opd)');
  if (JSON.parse(st).error) throw new Error(`page error: ${st}`);
  return JSON.parse(st);
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
      note(`console.${msg.params.type}: ${msg.params.args.map((a) => a.value ?? a.description).join(' ')}`);
    } else if (msg.method === 'Runtime.exceptionThrown') {
      note(`exception: ${JSON.stringify(msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text)}`);
    }
  };
  await cdp('Runtime.enable');
  await cdp('Page.enable');
  result.userAgent = (await cdp('Browser.getVersion')).product;
  await cdp('Page.navigate', { url });
  const st = await load();
  result.backend = st.backend;
  result.gpu = await ev(`(() => { const c = document.createElement('canvas'); const g = c.getContext('webgl2'); if (!g) return null; const e = g.getExtension('WEBGL_debug_renderer_info'); return e ? g.getParameter(e.UNMASKED_RENDERER_WEBGL) : g.getParameter(g.RENDERER); })()`);
  result.webgpuApi = await ev("typeof navigator.gpu !== 'undefined'");
  await sleep(300);
  const canvas = await ev('(() => { const r = document.getElementById("canvas").getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()');
  const at = (sx, sy) => [canvas[0] + sx, canvas[1] + sy];
  result.nodesLoaded = await nodes();
  result.shotLoaded = await shot(`browser-${mode}-loaded.png`);

  if (mode === 'edit') {
    // 1. Drag a childless top-level node that is fully on screen.
    const inView = (n) => n.screen[0] > 0 && n.screen[1] > 0 && n.screen[0] + n.screen[2] < canvas[2] - 100
      && n.screen[1] + n.screen[3] < canvas[3] - 100 && n.screen[2] > 24 && n.screen[3] > 24;
    const target = result.nodesLoaded.find((n) => n.children === 0 && inView(n))
      || result.nodesLoaded.find(inView);
    if (!target) throw new Error('no top-level node on screen to drag');
    const [cx, cy] = at(target.screen[0] + target.screen[2] / 2, target.screen[1] + target.screen[3] / 2);
    let n0 = await saves();
    await drag(cx, cy, cx + 80, cy + 60);
    await settled(n0, 'save after drag');
    const afterDrag = (await nodes()).find((n) => n.id === target.id);
    result.steps.push({ step: 'drag', node: target.id, kind: target.kind, before: target.world, after: afterDrag.world,
      selection: await ev('window.__opdEditor.selection_len()') });

    // 2. Rectangle tool from the toolbar, drawn on an empty spot.
    const covered = (x, y) => (n) => !(x + 160 < n.screen[0] || x > n.screen[0] + n.screen[2]
      || y + 120 < n.screen[1] || y > n.screen[1] + n.screen[3]);
    const current = await nodes();
    let spot = null;
    for (let y = 30; y < canvas[3] - 150 && !spot; y += 20) {
      for (let x = 30; x < canvas[2] - 190 && !spot; x += 20) {
        if (!current.some(covered(x, y))) spot = [x, y];
      }
    }
    if (!spot) throw new Error('no empty spot for a new rectangle');
    await clickButton('[data-tool="rect"]');
    const countBefore = current.length;
    n0 = await saves();
    const [sx, sy] = at(spot[0] + 10, spot[1] + 10);
    await drag(sx, sy, sx + 140, sy + 100);
    await settled(n0, 'save after rectangle');
    const afterRect = await nodes();
    const created = afterRect.find((n) => !current.some((c) => c.id === n.id));
    result.steps.push({ step: 'create-rect', count: [countBefore, afterRect.length], node: created });
    if (!created || created.kind !== 'rect') throw new Error('rectangle not created');

    // 3. Delete key removes the (selected) new rectangle.
    n0 = await saves();
    await key('Delete', 'Delete', 46);
    await settled(n0, 'save after delete');
    result.steps.push({ step: 'delete', count: (await nodes()).length });

    // 4. Ctrl+Z (keyboard) brings it back; toolbar Redo removes it; toolbar Undo restores it.
    n0 = await saves();
    await key('z', 'KeyZ', 90, 2);
    await settled(n0, 'save after undo');
    result.steps.push({ step: 'undo (Ctrl+Z)', count: (await nodes()).length });
    n0 = await saves();
    await clickButton('[data-action="redo"]');
    await settled(n0, 'save after redo');
    result.steps.push({ step: 'redo (toolbar)', count: (await nodes()).length });
    n0 = await saves();
    await clickButton('[data-action="undo"]');
    await settled(n0, 'save after toolbar undo');
    const final = await nodes();
    result.steps.push({ step: 'undo (toolbar)', count: final.length });
    result.final = {
      moved: final.find((n) => n.id === target.id),
      rect: final.find((n) => n.id === created.id),
      count: final.length,
    };
    result.revision = await ev('window.__opd.revision');
    result.saves = await saves();
    result.shotEdited = await shot('browser-edit-done.png');

    // 5. Reload: the page comes back from disk with the same content.
    await cdp('Page.reload', { ignoreCache: true });
    await sleep(200);
    await load();
    await sleep(300);
    const reloaded = await nodes();
    result.reload = {
      count: reloaded.length,
      moved: reloaded.find((n) => n.id === target.id),
      rect: reloaded.find((n) => n.id === created.id),
      revision: await ev('window.__opd.revision'),
    };
    result.shotReloaded = await shot('browser-edit-reloaded.png');
  }
  result.ok = true;
} catch (e) {
  result.ok = false;
  result.error = String(e && e.stack ? e.stack : e);
} finally {
  result.log = log;
  writeFileSync(join(outDir, `result-${mode}.json`), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  try { ws && ws.close(); } catch { /* closing */ }
  browser.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
  process.exit(result.ok ? 0 : 1);
}
