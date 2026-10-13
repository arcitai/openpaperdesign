// The home view (src/home.html) against a synthetic MCP Apps host: a fake parent window and a
// minimal fake DOM. Protocol behaviour only; this is not a real Codex host or browser.
// Run: node --test crates/openpaperdesign-serve/tests/home.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');
const script = html.slice(html.indexOf('<script>') + 8, html.lastIndexOf('</script>'));
const flush = () => new Promise((r) => setImmediate(r));
const TOKEN = 'ab'.repeat(32);
const URL_OK = `http://127.0.0.1:41234/#token=${TOKEN}`;
const ALLOWED = new Set(['ui/initialize', 'ui/notifications/initialized', 'ui/message', 'ui/open-link']);

function el() {
  const e = { textContent: '', hidden: false, disabled: false, className: '', value: '', onclick: null, children: [] };
  e.append = (...c) => { e.children.push(...c); };
  return e;
}

function mount() {
  const ids = ['conn', 'home', 'inventory', 'open', 'current', 'name', 'new', 'msg-state', 'prompt',
    'result', 'result-state', 'result-fields', 'canvas', 'canvas-state'];
  const els = Object.fromEntries(ids.map((id) => [id, el()]));
  els.result.hidden = true; els.prompt.hidden = true; els.canvas.hidden = true;
  const sent = [];
  let listener = null;
  // Copy into this realm so deep equality is not confused by the VM context's prototypes.
  const parent = { postMessage: (m) => sent.push(JSON.parse(JSON.stringify(m))) };
  const forbidden = () => { throw new Error('storage or network access'); };
  const window = { parent, addEventListener: (n, fn) => { if (n === 'message') listener = fn; } };
  for (const k of ['localStorage', 'sessionStorage', 'indexedDB', 'fetch', 'XMLHttpRequest']) {
    Object.defineProperty(window, k, { get: forbidden });
  }
  const document = { getElementById: (id) => els[id], createElement: () => el() };
  Object.defineProperty(document, 'cookie', { get: forbidden, set: forbidden });
  vm.runInNewContext(script, { window, document, setTimeout: () => 0, JSON, String, RegExp, Array, Map, Promise });
  const from = (m, source = parent) => listener({ source, data: { jsonrpc: '2.0', ...m } });
  const last = (method) => [...sent].reverse().find((m) => m.method === method);
  return { els, sent, from, last, parent };
}

async function initialized(caps = { message: { text: {} } }) {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: caps, hostContext: {} } });
  await flush();
  return v;
}

const methods = (v) => v.sent.filter((m) => m.method).map((m) => m.method);
const onlyUiMethods = (v) => assert.ok(methods(v).every((m) => ALLOWED.has(m)), methods(v).join());

test('initialize claims only display modes, then announces initialized', async () => {
  const v = mount();
  assert.equal(v.sent[0].method, 'ui/initialize');
  assert.deepEqual(v.sent[0].params.appCapabilities, { availableDisplayModes: ['inline', 'fullscreen'] });
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized']);
  assert.match(v.els.conn.textContent, /asks the agent in this chat/);
});

test('a failed initialize disables the actions and sends nothing else', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, error: { code: -32603, message: 'at /Users/someone/private', data: { token: TOKEN } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  assert.ok(v.els.open.disabled && v.els.new.disabled && v.els.current.disabled);
  assert.match(v.els.conn.textContent, /Couldn't connect to Codex \(code -32603\)/);
  assert.ok(!v.els.conn.textContent.includes('/Users/') && !v.els.conn.textContent.includes(TOKEN));
});

test('with message support each action sends one user message to the active chat', async () => {
  const v = await initialized();
  for (const [id, want] of [['open', /bind_project/], ['current', /Don't change anything/], ['new', /create_project/]]) {
    v.els[id].onclick();
    await flush();
    const m = v.last('ui/message');
    assert.equal(m.params.role, 'user');
    assert.equal(m.params._meta, undefined, 'defaults: active chat, sent');
    assert.equal(m.params.content.length, 1);
    assert.equal(m.params.content[0].type, 'text');
    assert.match(m.params.content[0].text, /openpaperdesign_status/);
    assert.match(m.params.content[0].text, want);
    v.from({ id: m.id, result: {} });
    await flush();
    assert.match(v.els['msg-state'].textContent, /^Sent to this chat/);
  }
  onlyUiMethods(v);
});

test('the new canvas message carries a cleaned name and the design method', async () => {
  const v = await initialized();
  v.els.name.value = '  Landing "page"\u0007 `x`  ';
  v.els.new.onclick();
  await flush();
  const text = v.last('ui/message').params.content[0].text;
  assert.match(text, /named "Landing page x"/);
  assert.match(text, /root DESIGN\.md \(use an existing design\.md as is; never add a second file that differs only in case\)/);
  v.els.name.value = '';
  v.els.new.onclick();
  await flush();
  assert.match(v.last('ui/message').params.content[0].text, /Ask me for its name first/);
});

test('a refused message is reported, not shown as sent', async () => {
  const v = await initialized();
  v.els.open.onclick();
  await flush();
  v.from({ id: v.last('ui/message').id, error: { code: -32601, message: 'no' } });
  await flush();
  assert.equal(v.els['msg-state'].textContent, "Codex didn't accept the message (code -32601).");
});

test('without message support actions show text to paste and send nothing', async () => {
  const v = await initialized({});
  assert.match(v.els.conn.textContent, /doesn't let apps send messages/);
  v.els.open.onclick();
  await flush();
  assert.equal(v.last('ui/message'), undefined);
  assert.equal(v.els.prompt.hidden, false);
  assert.match(v.els.prompt.textContent, /openpaperdesign_status/);
  onlyUiMethods(v);
});

const result = (obj, isError = false) => ({ method: 'ui/notifications/tool-result', params: { content: [{ type: 'text', text: typeof obj === 'string' ? obj : JSON.stringify(obj) }], isError } });
const fields = (v) => v.els['result-fields'].children.map((c) => c.textContent);

test('the home tool result keeps the home view', async () => {
  const v = await initialized();
  v.from(result({ surface: 'home', version: '0.1.0' }));
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.result.hidden, true);
});

test('an admitted status result shows its workspace and project', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', workspace_permission_mode: 'managed-workspace-write', bound_project: '/w/acme/design/Landing' }));
  assert.equal(v.els.home.hidden, true);
  assert.equal(v.els['result-state'].textContent, 'This chat can use its workspace.');
  assert.deepEqual(fields(v), ['Workspace', '/w/acme', 'Permissions', 'managed-workspace-write', 'Project', '/w/acme/design/Landing']);
});

test('a refused or retired status shows the problem and no workspace or project', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'retired', workspace_usable: false, workspace: '/w/old', workspace_problem: 'Codex workspace changed', bound_project: null }));
  assert.match(v.els['result-state'].textContent, /^This chat can't use a workspace: Codex workspace changed/);
  assert.equal(v.els['result-state'].className, 'bad');
  assert.deepEqual(fields(v), []);
  v.from(result('Missing Codex workspace context. Use a real model tool call.\nmore', true));
  assert.equal(v.els['result-state'].textContent, 'Refused: Missing Codex workspace context. Use a real model tool call.');
});

test('only the exact loopback editor URL can be opened, and the ack is not a load', async () => {
  for (const bad of ['https://127.0.0.1:1/#token=' + TOKEN, 'http://127.0.0.1:1/x#token=' + TOKEN,
    'http://localhost:1/#token=' + TOKEN, 'http://127.0.0.1:1/#token=short', 'http://evil.example/#token=' + TOKEN,
    URL_OK + '&x=1', 'javascript:alert(1)']) {
    const v = await initialized();
    v.from(result({ url: bad }));
    assert.equal(v.els.canvas.hidden, true, bad);
    assert.match(v.els['result-state'].textContent, /won't be opened/);
  }
  const v = await initialized();
  v.from(result({ url: URL_OK, open_in: 'in-app browser' }));
  assert.equal(v.els.canvas.hidden, false);
  assert.deepEqual(fields(v), ['Address', 'local, port 41234']);
  assert.ok(!JSON.stringify(v.els).includes(TOKEN), 'the token is not displayed');
  v.els.canvas.onclick();
  await flush();
  const open = v.last('ui/open-link');
  assert.deepEqual(open.params, { url: URL_OK });
  v.from({ id: open.id, result: {} });
  await flush();
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/);
  onlyUiMethods(v);
});

test('messages from other windows are ignored', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } }, { postMessage() {} });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  v.from(result({ url: URL_OK }), { postMessage() {} });
  assert.equal(v.els.canvas.hidden, true);
});

test('the Codex project section is an honest unavailable state, not a chooser', () => {
  const card = html.slice(html.indexOf('<h2>Codex projects</h2>'), html.indexOf('<h2>Start</h2>'));
  assert.match(card, /Codex keeps a list of your projects, but this version of OpenPaperDesign can't show it yet/);
  assert.doesNotMatch(card, /<(button|input|select|ul|ol)\b/, 'no dead chooser or fallback list');
  assert.match(card, /never takes a folder path from here/);
});

test('the view source never calls server tools or stores anything', () => {
  for (const banned of ['tools/call', 'resources/read', 'localStorage', 'sessionStorage', 'indexedDB', 'document.cookie', 'fetch(', 'XMLHttpRequest', 'innerHTML']) {
    assert.ok(!script.includes(banned), banned);
  }
});
