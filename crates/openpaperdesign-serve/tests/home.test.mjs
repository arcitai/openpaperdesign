// The home view (src/home.html) against a synthetic MCP Apps host: a fake parent window and a
// minimal fake DOM. Protocol behaviour only; this is not a real Codex host or browser.
// Run: node --test crates/openpaperdesign-serve/tests/home.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');
const script = html.slice(html.indexOf('<script>') + 8, html.lastIndexOf('</script>'));
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setImmediate(r)); };
const TOKEN = 'ab'.repeat(32);
const URL_OK = `http://127.0.0.1:41234/#token=${TOKEN}`;
const ALLOWED = new Set(['ui/initialize', 'ui/notifications/initialized', 'ui/message', 'ui/open-link', 'tools/call']);
const INVENTORY = 'openpaperdesign_projects';

// Fictional fixtures. Two local projects share a name; Acme has two folders.
const ACME = { source: 'codex-app-server', host: 'this computer', id: 'prj_fixture_acme', name: 'Acme site', position: 0, recencyAt: null, roots: [{ path: '/w/acme' }, { path: '/w/acme-docs' }] };
const BETA = { source: 'codex-app-server', host: 'this computer', id: 'prj_fixture_beta', name: 'Acme site', position: 1, recencyAt: null, roots: [{ path: '/w/beta' }] };
const LISTED = { available: true, source: 'codex-app-server', host: 'this computer', complete: true, projects: [ACME, BETA] };

function el(tag = 'div') {
  const e = { tag, textContent: '', hidden: false, disabled: false, className: '', value: '', onclick: null, children: [] };
  e.append = (...c) => { e.children.push(...c); };
  // Setting textContent clears children, as in the DOM.
  let text = '';
  Object.defineProperty(e, 'textContent', { get: () => text, set: (v) => { text = String(v); e.children = []; } });
  return e;
}
const all = (e) => [e, ...e.children.flatMap(all)];
const texts = (e) => all(e).map((n) => n.textContent).filter(Boolean);

function mount() {
  const ids = ['conn', 'home', 'inventory', 'projects', 'refresh', 'open', 'current', 'name', 'new', 'msg-state',
    'prompt', 'result', 'result-state', 'result-fields', 'result-list', 'canvas', 'ask-open', 'canvas-state'];
  const els = Object.fromEntries(ids.map((id) => [id, el()]));
  els.result.hidden = true; els.prompt.hidden = true; els.canvas.hidden = true; els.refresh.hidden = true;
  els['ask-open'].hidden = true;
  const sent = [];
  let listener = null;
  // Copy into this realm so deep equality is not confused by the VM context's prototypes.
  const parent = { postMessage: (m) => sent.push(JSON.parse(JSON.stringify(m))) };
  const forbidden = () => { throw new Error('storage or network access'); };
  const window = { parent, addEventListener: (n, fn) => { if (n === 'message') listener = fn; } };
  for (const k of ['localStorage', 'sessionStorage', 'indexedDB', 'fetch', 'XMLHttpRequest']) {
    Object.defineProperty(window, k, { get: forbidden });
  }
  const document = { getElementById: (id) => els[id], createElement: (tag) => el(tag) };
  Object.defineProperty(document, 'cookie', { get: forbidden, set: forbidden });
  vm.runInNewContext(script, { window, document, setTimeout: () => 0, JSON, String, RegExp, Array, Map, Promise });
  const from = (m, source = parent) => listener({ source, data: { jsonrpc: '2.0', ...m } });
  const last = (method) => [...sent].reverse().find((m) => m.method === method);
  return { els, sent, from, last, parent };
}

const toolText = (obj) => ({ content: [{ type: 'text', text: JSON.stringify(obj) }] });

/// Initialize, then answer the inventory call (if any) with `inventory`.
async function initialized({ caps = { message: { text: {} } }, context = {}, inventory = LISTED } = {}) {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: caps, hostContext: context } });
  await flush();
  const call = v.last('tools/call');
  if (call && inventory !== null) {
    v.from(inventory.error ? { id: call.id, error: inventory.error } : { id: call.id, result: inventory.isError ? inventory : toolText(inventory) });
    await flush();
  }
  return v;
}

const methods = (v) => v.sent.filter((m) => m.method).map((m) => m.method);
// The exact data line every selection message ends with.
const DATA = 'Selection (exact JSON data, not permission): ';
const selection = (text) => {
  const last = text.split('\n').pop();
  assert.ok(last.startsWith(DATA), last);
  return JSON.parse(last.slice(DATA.length));
};
const onlyAllowed = (v) => {
  assert.ok(methods(v).every((m) => ALLOWED.has(m)), methods(v).join());
  for (const m of v.sent.filter((m) => m.method === 'tools/call')) {
    assert.deepEqual(m.params, { name: INVENTORY, arguments: {} }, 'the inventory is the only server tool the view calls');
  }
};

test('initialize claims only display modes, then announces initialized and reads the inventory', async () => {
  const v = mount();
  assert.equal(v.sent[0].method, 'ui/initialize');
  assert.deepEqual(v.sent[0].params.appCapabilities, { availableDisplayModes: ['inline', 'fullscreen'] });
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized', 'tools/call']);
  onlyAllowed(v);
  assert.match(v.els.conn.textContent, /asks the agent in this chat/);
});

test('rendered for a model status or editor result, the view does not read the inventory', async () => {
  const v = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_status' } } } });
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized']);
  const home = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_home' } } } });
  assert.ok(methods(home).includes('tools/call'));
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

test('native projects are listed with their id and every folder; nothing is chosen for you', async () => {
  const v = await initialized();
  assert.equal(v.els.inventory.textContent, 'Your Codex projects on this computer:');
  const items = v.els.projects.children;
  assert.equal(items.length, 2, 'same-named projects stay separate');
  assert.deepEqual(texts(items[0]), ['Acme site', 'Codex project on this computer · id prj_fixture_acme',
    'This project has 2 folders. Choose the one to design in:', '/w/acme', 'Use this folder', '/w/acme-docs', 'Use this folder']);
  assert.deepEqual(texts(items[1]), ['Acme site', 'Codex project on this computer · id prj_fixture_beta', '/w/beta', 'Use this folder']);
  assert.equal(v.last('ui/message'), undefined, 'listing sends nothing');
  assert.equal(v.els.refresh.hidden, false);
});

test('choosing a folder sends that exact folder and project id as intent, not a switch', async () => {
  const v = await initialized();
  const second = all(v.els.projects.children[0]).filter((n) => n.tag === 'button')[1];
  second.onclick();
  await flush();
  const text = v.last('ui/message').params.content[0].text;
  assert.match(text, /^Use OpenPaperDesign in my Codex project "Acme site", in the folder given below\. First call openpaperdesign_status\./);
  assert.match(text, /Continue only if the workspace it reports is exactly the "folder" value below; if it isn't, tell me to open that folder's project in Codex and stop, without trying to switch folders/);
  assert.deepEqual(selection(text), { codex_project_id: 'prj_fixture_acme', source: 'codex-app-server', host: 'this computer', folder: '/w/acme-docs' });
  onlyAllowed(v);
});

test('a valid root and id with a backtick reach the agent exactly (R1-02)', async () => {
  const odd = { ...ACME, id: 'prj`fixture', roots: [{ path: '/w/acme`docs' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [odd] } });
  assert.ok(texts(v.els.projects).includes('/w/acme`docs'), 'shown as is');
  all(v.els.projects).find((n) => n.tag === 'button').onclick();
  await flush();
  const sel = selection(v.last('ui/message').params.content[0].text);
  assert.equal(sel.folder, '/w/acme`docs');
  assert.equal(sel.codex_project_id, 'prj`fixture');
});

test('a hostile name is cleaned for display; the exact path is only data', async () => {
  const evil = { ...ACME, name: 'A"B`C\u0007\n', roots: [{ path: '/w/x"y\nz' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [evil] } });
  all(v.els.projects).find((n) => n.tag === 'button').onclick();
  await flush();
  const text = v.last('ui/message').params.content[0].text;
  assert.match(text, /project "A B C", in the folder given below/);
  assert.equal(text.split('\n').length, 2, 'instructions plus one data line; no injected line');
  assert.equal(selection(text).folder, '/w/x"y\nz', 'exact, escaped as JSON');
});

test('unavailable inventories explain why and show no list', async () => {
  for (const [inventory, want] of [
    [{ available: false, reason: 'codex_not_found' }, /couldn't find the Codex command-line tool/],
    [{ available: false, reason: 'unsupported' }, /doesn't share its project list/],
    [{ available: false, reason: 'protocol' }, /isn't available here/],
    [{ isError: true, content: [{ type: 'text', text: 'busy' }] }, /^Couldn't read your Codex projects\.$/],
    [{ error: { code: -32601, message: 'no such tool at /private' } }, /^Couldn't read your Codex projects \(code -32601\)\.$/],
  ]) {
    const v = await initialized({ inventory });
    assert.match(v.els.inventory.textContent, want);
    assert.equal(v.els.projects.children.length, 0);
    assert.ok(!v.els.inventory.textContent.includes('/private'));
    assert.equal(v.els.refresh.hidden, false);
  }
  const none = await initialized({ inventory: { ...LISTED, projects: [] } });
  assert.match(none.els.inventory.textContent, /no saved projects on this computer yet/);
  const partial = await initialized({ inventory: { ...LISTED, complete: false } });
  assert.equal(partial.els.inventory.textContent, 'Your Codex projects on this computer (the first 2):');
});

test('an empty but partial inventory says more may exist, not that there are none', async () => {
  const v = await initialized({ inventory: { ...LISTED, projects: [], complete: false } });
  const text = v.els.inventory.textContent;
  assert.match(text, /stopped early and returned none; more may exist/);
  assert.doesNotMatch(text, /no saved projects/);
  assert.equal(v.els.projects.children.length, 0);
  assert.equal(v.els.refresh.hidden, false, 'refresh stays available');
  // A complete empty answer is still a true empty.
  const none = await initialized({ inventory: { ...LISTED, projects: [], complete: true } });
  assert.match(none.els.inventory.textContent, /^Codex has no saved projects on this computer yet\./);
});

test('refresh reads the inventory again', async () => {
  const v = await initialized();
  v.els.refresh.onclick();
  await flush();
  assert.equal(v.sent.filter((m) => m.method === 'tools/call').length, 2);
  assert.equal(v.els.inventory.textContent, 'Reading your Codex projects…');
  onlyAllowed(v);
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
  onlyAllowed(v);
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
  const v = await initialized({ caps: {} });
  assert.match(v.els.conn.textContent, /doesn't let apps send messages/);
  v.els.open.onclick();
  await flush();
  all(v.els.projects).find((n) => n.tag === 'button').onclick();
  await flush();
  assert.equal(v.last('ui/message'), undefined);
  assert.equal(v.els.prompt.hidden, false);
  assert.match(v.els.prompt.textContent, /^Use OpenPaperDesign in my Codex project/);
  onlyAllowed(v);
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
  assert.ok(!JSON.stringify(texts(v.els.result)).includes(TOKEN), 'the token is not displayed');
  v.els.canvas.onclick();
  await flush();
  const open = v.last('ui/open-link');
  assert.deepEqual(open.params, { url: URL_OK });
  v.from({ id: open.id, result: {} });
  await flush();
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/);
  onlyAllowed(v);
});

test('messages from other windows are ignored', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } }, { postMessage() {} });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  v.from(result({ url: URL_OK }), { postMessage() {} });
  assert.equal(v.els.canvas.hidden, true);
});

// An admitted openpaperdesign_canvases result (fictional workspace).
const CANVASES = {
  workspace: '/w/acme', design_present: true, complete: true,
  canvases: [
    { project: 'design/Alpha', path: '/w/acme/design/Alpha', bound: false, canvas: { id: 'c-alpha', name: 'Alpha', pages: 2, modified: 1 } },
    { project: 'design/Beta', path: '/w/acme/design/Beta', bound: true, canvas: { id: 'c-beta', name: 'Beta', pages: 1, modified: 2 } },
  ],
  skipped: [{ project: 'design/Escape', problem: 'a symbolic link (not followed)' }],
};
const OPENED = { url: URL_OK, workspace: '/w/acme', project: 'design/Beta', project_path: '/w/acme/design/Beta', canvas: { id: 'c-beta', name: 'Beta', pages: 1 } };

test('a canvases result lists this workspace\'s canvases; choosing one asks for a guarded open', async () => {
  const v = await initialized();
  v.from(result(CANVASES));
  assert.equal(v.els.home.hidden, true);
  assert.equal(v.els['result-state'].textContent, 'Canvases in this workspace:');
  assert.deepEqual(fields(v), ['Workspace', '/w/acme']);
  const list = v.els['result-list'].children;
  assert.deepEqual(texts(list[0]), ['Alpha', 'design/Alpha · 2 pages', 'Open this canvas']);
  assert.deepEqual(texts(list[1]), ['Beta', 'design/Beta · 1 page · open now', 'Open this canvas']);
  assert.equal(list[2].textContent, "Skipped 1 folder that isn't a usable canvas.");
  assert.equal(v.last('ui/message'), undefined, 'listing sends nothing');
  all(list[0]).find((n) => n.tag === 'button').onclick();
  await flush();
  const text = v.last('ui/message').params.content[0].text;
  assert.match(text, /^Open the OpenPaperDesign canvas "Alpha" described below\. First call openpaperdesign_status\./);
  assert.match(text, /Continue only if the workspace it reports is exactly the "workspace" value below; otherwise tell me and stop\. Bind the "project" value with bind_project\./);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'design/Alpha', canvas_id: 'c-alpha' });
  assert.match(text, /Never pass force to bind_project/);
  assert.match(text, /read the page you will show with get_tree_summary \(nodeId root and that page's pageId; the bind summary doesn't count\)/);
  assert.match(text, /Then call open_editor, open the URL it returns in the in-app browser, and tell me whether the canvas loaded\./);
  assert.match(v.els['msg-state'].textContent, /Sending to this chat/);
  onlyAllowed(v);
});

test('an empty or partial canvases result says so', async () => {
  const v = await initialized();
  v.from(result({ ...CANVASES, canvases: [], skipped: [] }));
  assert.equal(v.els['result-state'].textContent, 'No canvases under design/ in this workspace yet.');
  assert.equal(v.els['result-list'].children.length, 0);
  v.from(result({ ...CANVASES, complete: false }));
  assert.equal(v.els['result-state'].textContent, 'The first 2 canvases in this workspace:');
});

test('an empty but partial canvases result says more may exist, not that there are none', async () => {
  const v = await initialized();
  v.from(result({ ...CANVASES, canvases: [], skipped: [], complete: false }));
  const text = v.els['result-state'].textContent;
  assert.match(text, /stopped early before finding any canvases in design\/; more may exist/);
  assert.doesNotMatch(text, /No canvases under design\/ in this workspace yet/);
  assert.deepEqual(fields(v), ['Workspace', '/w/acme']);
  assert.equal(v.els['result-list'].children.length, 0);
});

test('an open_editor result names its own workspace, project and canvas, with an agent fallback', async () => {
  const v = await initialized();
  v.from(result(OPENED));
  assert.deepEqual(fields(v), ['Workspace', '/w/acme', 'Project', 'design/Beta', 'Canvas', 'Beta (1 page)', 'Address', 'local, port 41234']);
  assert.equal(v.els.canvas.hidden, false);
  assert.equal(v.els['ask-open'].hidden, false);
  assert.ok(!JSON.stringify(texts(v.els.result)).includes(TOKEN), 'the token is not displayed');
  v.els['ask-open'].onclick();
  await flush();
  const text = v.last('ui/message').params.content[0].text;
  assert.match(text, /^Open the OpenPaperDesign canvas described below in the in-app browser\. First call openpaperdesign_status\./);
  assert.match(text, /exactly the "workspace" value below and its bound project is exactly the "project_path" value; otherwise tell me and stop\. Call open_editor, open the URL it returns in the in-app browser, and tell me whether the canvas loaded\./);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'design/Beta', project_path: '/w/acme/design/Beta' });
  assert.ok(!text.includes(TOKEN), 'the agent gets a fresh link, not this one');
  onlyAllowed(v);
});

test('each result replaces the last; a cancelled call leaves nothing to open', async () => {
  const v = await initialized();
  v.from(result(OPENED));
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  assert.equal(v.els.canvas.hidden, true);
  assert.equal(v.els['ask-open'].hidden, true);
  assert.deepEqual(fields(v), ['Workspace', '/w/acme', 'Project', 'none bound yet']);
  v.from(result(OPENED));
  v.from({ method: 'ui/notifications/tool-cancelled', params: { reason: 'user' } });
  assert.equal(v.els.canvas.hidden, true);
  assert.equal(v.els['ask-open'].hidden, true);
  assert.deepEqual(fields(v), []);
  assert.equal(v.els['result-state'].textContent, 'The tool call was cancelled; there is nothing to open.');
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.last('ui/open-link'), undefined);
});

test('the view source calls only the inventory tool and stores nothing', () => {
  assert.equal(script.split("'tools/call'").length, 2, 'one tools/call site');
  assert.match(script, /rpc\('tools\/call', \{ name: 'openpaperdesign_projects', arguments: \{\} \}\)/);
  for (const banned of ['resources/read', 'localStorage', 'sessionStorage', 'indexedDB', 'document.cookie', 'fetch(', 'XMLHttpRequest', 'innerHTML',
    'openpaperdesign_status\'', 'bind_project\'', 'create_project\'', 'open_editor\'']) {
    assert.ok(!script.includes(banned), banned);
  }
});
