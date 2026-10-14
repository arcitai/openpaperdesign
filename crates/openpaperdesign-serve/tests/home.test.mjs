// The home view (src/home.html) against a synthetic MCP Apps host: a fake parent window and a
// minimal fake DOM. Protocol and state behaviour only; this is not a real Codex host or
// browser, and it proves nothing about rendering or real accessibility.
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

function el(tag = 'div', doc) {
  const attrs = {};
  const e = { tag, hidden: false, disabled: false, className: '', value: '', type: '', onclick: null, onsubmit: null, children: [] };
  e.append = (...c) => { e.children.push(...c); };
  e.setAttribute = (k, v) => { attrs[k] = String(v); };
  e.getAttribute = (k) => (k in attrs ? attrs[k] : null);
  e.focus = () => { if (doc) doc.activeElement = e; };
  // Setting textContent clears children, as in the DOM.
  let text = '';
  Object.defineProperty(e, 'textContent', { get: () => text, set: (v) => { text = String(v); e.children = []; } });
  return e;
}
const all = (e) => [e, ...e.children.flatMap(all)];
const texts = (e) => all(e).map((n) => n.textContent).filter(Boolean);
const buttons = (e) => all(e).filter((n) => n.tag === 'button');

const IDS = ['title', 'back', 'conn', 'live', 'home', 'start', 'inventory', 'projects', 'refresh', 'inventory-details',
  'inventory-detail', 'chosen', 'chosen-name', 'chosen-path', 'chosen-details', 'clear', 'scope', 'open', 'new', 'current',
  'new-form', 'name', 'create', 'cancel-new', 'result', 'result-title', 'result-state', 'result-fields', 'result-list',
  'canvas', 'ask-open', 'canvas-state', 'result-details', 'result-detail', 'msg-state', 'prompt'];

function mount() {
  const document = { activeElement: null };
  const els = Object.fromEntries(IDS.map((id) => [id, el('div', document)]));
  for (const id of ['back', 'result', 'prompt', 'canvas', 'ask-open', 'refresh', 'inventory-details', 'chosen', 'new-form', 'result-details']) {
    els[id].hidden = true;
  }
  const props = {};
  const rootAttrs = {};
  const documentElement = {
    style: { setProperty: (k, v) => { props[k] = v; } },
    setAttribute: (k, v) => { rootAttrs[k] = String(v); },
  };
  const sent = [];
  let listener = null;
  // Copy into this realm so deep equality is not confused by the VM context's prototypes.
  const parent = { postMessage: (m) => sent.push(JSON.parse(JSON.stringify(m))) };
  const forbidden = () => { throw new Error('storage or network access'); };
  const window = { parent, addEventListener: (n, fn) => { if (n === 'message') listener = fn; } };
  for (const k of ['localStorage', 'sessionStorage', 'indexedDB', 'fetch', 'XMLHttpRequest']) {
    Object.defineProperty(window, k, { get: forbidden });
  }
  Object.assign(document, { getElementById: (id) => els[id], createElement: (tag) => el(tag, document), documentElement });
  Object.defineProperty(document, 'cookie', { get: forbidden, set: forbidden });
  vm.runInNewContext(script, { window, document, setTimeout: () => 0, JSON, String, RegExp, Array, Map, Promise, Object, Number });
  const from = (m, source = parent) => listener({ source, data: { jsonrpc: '2.0', ...m } });
  const last = (method) => [...sent].reverse().find((m) => m.method === method);
  return { els, sent, from, last, parent, document, props, rootAttrs };
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
const lastMessage = (v) => v.last('ui/message').params.content[0].text;
/// Answer the last ui/message so the next action is no longer busy.
async function ack(v, reply = { result: {} }) {
  v.from({ id: v.last('ui/message').id, ...reply });
  await flush();
}
const chooseFolder = async (v, project, path) => {
  const b = buttons(v.els.projects).find((n) => n.getAttribute('aria-label') === `Choose ${project}, ${path}`);
  assert.ok(b, `a Choose button for ${path}`);
  b.onclick();
  await flush();
  return b;
};

// ---- connection and inventory ----

test('initialize claims only display modes, then announces initialized and reads the inventory', async () => {
  const v = mount();
  assert.equal(v.sent[0].method, 'ui/initialize');
  assert.deepEqual(v.sent[0].params.appCapabilities, { availableDisplayModes: ['inline', 'fullscreen'] });
  assert.ok(v.els.open.disabled && v.els.new.disabled && v.els.current.disabled, 'no action before Codex answers');
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized', 'tools/call']);
  onlyAllowed(v);
  assert.equal(v.els.conn.textContent, 'Ready');
  assert.ok(!v.els.open.disabled);
});

test('rendered for a model status or editor result, the view does not read the inventory', async () => {
  const v = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_status' } } } });
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized']);
  const home = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_home' } } } });
  assert.ok(methods(home).includes('tools/call'));
});

test('a failed initialize keeps the actions disabled, says so and sends nothing else', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, error: { code: -32603, message: 'at /Users/someone/private', data: { token: TOKEN } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  assert.ok(v.els.open.disabled && v.els.new.disabled && v.els.current.disabled);
  assert.equal(v.els.conn.getAttribute('data-state'), 'error');
  assert.match(v.els.inventory.textContent, /Couldn't connect to Codex \(code -32603\)/);
  const shown = JSON.stringify(texts(v.els.inventory)) + v.els.live.textContent;
  assert.ok(!shown.includes('/Users/') && !shown.includes(TOKEN));
});

test('native projects keep every folder; same-named projects stay apart; nothing is chosen for you', async () => {
  const v = await initialized();
  assert.equal(v.els.inventory.textContent, 'Your Codex projects on this computer:');
  const [acme, beta] = v.els.projects.children;
  assert.deepEqual(texts(acme), ['Acme site', '2 folders · project …e_acme', '/w/acme', 'Choose', '/w/acme-docs', 'Choose']);
  assert.deepEqual(texts(beta), ['Acme site', '/w/beta · project …e_beta', 'Choose']);
  assert.deepEqual(buttons(v.els.projects).map((b) => b.getAttribute('aria-label')),
    ['Choose Acme site, /w/acme', 'Choose Acme site, /w/acme-docs', 'Choose Acme site, /w/beta']);
  assert.equal(v.last('ui/message'), undefined, 'listing sends nothing');
  assert.equal(v.els.chosen.hidden, true);
  assert.equal(v.els.refresh.hidden, false);
});

test('unavailable inventories explain why, keep the chat-project path open and offer refresh', async () => {
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
    assert.ok(!v.els.open.disabled, 'this chat\'s project can still be used');
  }
  const coded = await initialized({ inventory: { available: false, reason: 'protocol' } });
  assert.equal(coded.els['inventory-details'].hidden, false, 'the reason stays available as a detail');
  assert.equal(coded.els['inventory-detail'].textContent, 'Reason: protocol');
  const none = await initialized({ inventory: { ...LISTED, projects: [] } });
  assert.match(none.els.inventory.textContent, /^Codex has no saved projects on this computer yet\./);
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
  const none = await initialized({ inventory: { ...LISTED, projects: [], complete: true } });
  assert.match(none.els.inventory.textContent, /^Codex has no saved projects on this computer yet\./);
});

test('refresh reads the inventory again and keeps the chosen folder marked', async () => {
  const v = await initialized();
  await chooseFolder(v, 'Acme site', '/w/acme-docs');
  v.els.refresh.onclick();
  await flush();
  assert.equal(v.sent.filter((m) => m.method === 'tools/call').length, 2);
  assert.equal(v.els.inventory.textContent, 'Reading your Codex projects…');
  assert.ok(v.els.refresh.disabled || v.els.refresh.hidden, 'no second refresh while reading');
  v.from({ id: v.last('tools/call').id, result: toolText(LISTED) });
  await flush();
  const pressed = buttons(v.els.projects).filter((b) => b.getAttribute('aria-pressed') === 'true');
  assert.deepEqual(pressed.map((b) => b.getAttribute('aria-label')), ['Choose Acme site, /w/acme-docs']);
  onlyAllowed(v);
});

// ---- choosing a folder and acting on it ----

test('choosing a folder only selects it; the next action sends that exact folder as intent', async () => {
  const v = await initialized();
  const b = await chooseFolder(v, 'Acme site', '/w/acme-docs');
  assert.equal(v.last('ui/message'), undefined, 'choosing sends nothing');
  assert.equal(b.getAttribute('aria-pressed'), 'true');
  assert.equal(v.els.chosen.hidden, false);
  assert.equal(v.els.scope.hidden, true);
  assert.equal(v.els['chosen-name'].textContent, 'Acme site');
  assert.equal(v.els['chosen-path'].textContent, '/w/acme-docs');
  assert.deepEqual(texts(v.els['chosen-details']), ['Folder', '/w/acme-docs', 'Codex project id', 'prj_fixture_acme', 'Source', 'codex-app-server · this computer']);
  assert.equal(v.document.activeElement, v.els.open, 'focus moves to the next action');
  v.els.open.onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /^Use OpenPaperDesign in my Codex project "Acme site", in the folder given below\. First call openpaperdesign_status\./);
  assert.match(text, /Continue only if the workspace it reports is exactly the "folder" value below; if it isn't, tell me to open that folder's project in Codex and stop, without trying to switch folders/);
  assert.match(text, /Never pass force to bind_project/);
  assert.deepEqual(selection(text), { codex_project_id: 'prj_fixture_acme', source: 'codex-app-server', host: 'this computer', folder: '/w/acme-docs' });
  onlyAllowed(v);
});

test('new canvas and current canvas carry the chosen folder too; clearing returns to this chat\'s project', async () => {
  const v = await initialized();
  await chooseFolder(v, 'Acme site', '/w/beta');
  v.els.new.onclick();
  assert.equal(v.els['new-form'].hidden, false);
  assert.equal(v.els.new.getAttribute('aria-expanded'), 'true');
  assert.equal(v.document.activeElement, v.els.name);
  assert.equal(v.last('ui/message'), undefined, 'opening the form sends nothing');
  v.els.name.value = '  Landing "page"\u0007 `x`  ';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  let text = lastMessage(v);
  assert.match(text, /^Create a new OpenPaperDesign canvas named "Landing page x" in my Codex project "Acme site", in the folder given below\./);
  assert.match(text, /Continue only if the workspace it reports is exactly the "folder" value below/);
  assert.match(text, /root DESIGN\.md \(use an existing design\.md as is; never add a second file that differs only in case\)/);
  assert.equal(selection(text).folder, '/w/beta');
  await ack(v);
  v.els.current.onclick();
  await flush();
  text = lastMessage(v);
  assert.match(text, /Don't change anything\. Tell me whether that workspace is exactly the "folder" value below\./);
  assert.equal(selection(text).folder, '/w/beta');
  await ack(v);
  v.els.clear.onclick();
  assert.equal(v.els.chosen.hidden, true);
  assert.equal(v.els.scope.hidden, false);
  v.els.open.onclick();
  await flush();
  text = lastMessage(v);
  assert.match(text, /^Open an existing OpenPaperDesign canvas in this workspace\./);
  assert.ok(!text.includes(DATA), 'no folder data without a choice');
  assert.ok(buttons(v.els.projects).every((b) => b.getAttribute('aria-pressed') === 'false'));
});

test('a new canvas without a name asks for one; cancel closes the form without sending', async () => {
  const v = await initialized();
  v.els.new.onclick();
  v.els['cancel-new'].onclick();
  assert.equal(v.els['new-form'].hidden, true);
  assert.equal(v.els.new.getAttribute('aria-expanded'), 'false');
  assert.equal(v.document.activeElement, v.els.new);
  assert.equal(v.last('ui/message'), undefined);
  v.els.new.onclick();
  v.els.name.value = '';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  assert.match(lastMessage(v), /Ask me for its name first/);
});

test('while a message is in flight the actions are busy, so nothing is sent twice', async () => {
  const v = await initialized();
  v.els.open.onclick();
  await flush();
  assert.equal(v.els.start.getAttribute('aria-busy'), 'true');
  assert.ok(v.els.open.disabled && v.els.new.disabled && v.els.current.disabled && v.els.create.disabled);
  assert.equal(v.els['msg-state'].textContent, 'Sending to this chat…');
  v.els.current.onclick();
  await flush();
  assert.equal(v.sent.filter((m) => m.method === 'ui/message').length, 1);
  await ack(v);
  assert.equal(v.els.start.getAttribute('aria-busy'), 'false');
  assert.ok(!v.els.open.disabled);
  assert.match(v.els['msg-state'].textContent, /^Sent to this chat\./);
  assert.doesNotMatch(v.els['msg-state'].textContent, /opened|loaded|saved/i, 'a sent request is not an opened project');
  assert.equal(v.els.live.textContent, v.els['msg-state'].textContent, 'announced');
});

test('a refused or unanswered message is reported with a retry, not shown as sent', async () => {
  const v = await initialized();
  v.els.open.onclick();
  await flush();
  await ack(v, { error: { code: -32601, message: 'no' } });
  assert.equal(v.els['msg-state'].textContent, "Codex didn't accept the message (code -32601). Try again.");
  assert.ok(!v.els.open.disabled, 'retry possible');
});

test('a valid root and id with a backtick reach the agent exactly (R1-02)', async () => {
  const odd = { ...ACME, id: 'prj`fixture', roots: [{ path: '/w/acme`docs' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [odd] } });
  assert.ok(texts(v.els.projects).includes('/w/acme`docs'), 'shown as is');
  buttons(v.els.projects)[0].onclick();
  v.els.open.onclick();
  await flush();
  const sel = selection(lastMessage(v));
  assert.equal(sel.folder, '/w/acme`docs');
  assert.equal(sel.codex_project_id, 'prj`fixture');
});

test('a hostile name is cleaned for display; the exact path is only data', async () => {
  const evil = { ...ACME, name: 'A"B`C\u0007\n', roots: [{ path: '/w/x"y\nz' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [evil] } });
  buttons(v.els.projects)[0].onclick();
  v.els.open.onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /project "A B C", in the folder given below/);
  assert.equal(text.split('\n').length, 2, 'instructions plus one data line; no injected line');
  assert.equal(selection(text).folder, '/w/x"y\nz', 'exact, escaped as JSON');
});

test('without message support actions show text to paste and send nothing', async () => {
  const v = await initialized({ caps: {} });
  assert.equal(v.els.conn.textContent, 'Ready (copy mode)');
  assert.match(v.els['msg-state'].textContent, /doesn't let apps send messages/);
  await chooseFolder(v, 'Acme site', '/w/acme');
  v.els.open.onclick();
  await flush();
  assert.equal(v.last('ui/message'), undefined);
  assert.equal(v.els.prompt.hidden, false);
  assert.match(v.els.prompt.textContent, /^Use OpenPaperDesign in my Codex project/);
  onlyAllowed(v);
});

// ---- host presentation ----

test('host theme and standard style variables apply as presentation only, with partial updates merged', async () => {
  const v = await initialized({ context: { theme: 'dark', styles: { variables: {
    '--color-background-primary': '#101010', '--color-text-primary': 'rgb(240 240 240)', '--font-sans': 'Inter, system-ui',
    '--color-text-secondary': 'url(https://example.invalid/x)', '--color-border-primary': 'red; background: url(x)',
    '--made-up': '#000', '--color-ring-primary': 'x'.repeat(201),
  } } } });
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  assert.deepEqual(v.props, { '--color-background-primary': '#101010', '--color-text-primary': 'rgb(240 240 240)', '--font-sans': 'Inter, system-ui' },
    'only standard names this view uses, with plain values; no url(), injected declarations or oversized values');
  v.from({ method: 'ui/notifications/host-context-changed', params: { theme: 'light' } });
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.equal(v.props['--color-background-primary'], '#101010', 'partial update keeps earlier styles');
  v.from({ method: 'ui/notifications/host-context-changed', params: { styles: { variables: { '--color-background-primary': '#fafafa' } } } });
  assert.equal(v.props['--color-background-primary'], '#fafafa');
  assert.equal(v.rootAttrs['data-theme'], 'light');
  const before = { ...v.props };
  v.from({ method: 'ui/notifications/host-context-changed', params: { styles: { css: { fonts: '@import url(https://fonts.invalid/x.css);' } } } });
  assert.deepEqual(v.props, before, 'font-loading CSS is never applied');
  assert.equal(methods(v).filter((m) => m === 'tools/call').length, 1, 'host context grants nothing and triggers no call');
});

test('host context from another window is ignored', async () => {
  const v = await initialized();
  v.from({ method: 'ui/notifications/host-context-changed', params: { theme: 'dark' } }, { postMessage() {} });
  assert.equal(v.rootAttrs['data-theme'], undefined);
});

// ---- delivered results ----

const result = (obj, isError = false) => ({ method: 'ui/notifications/tool-result', params: { content: [{ type: 'text', text: typeof obj === 'string' ? obj : JSON.stringify(obj) }], isError } });
const fields = (v) => v.els['result-fields'].children.map((c) => c.textContent);

test('the home tool result keeps the home view', async () => {
  const v = await initialized();
  v.from(result({ surface: 'home', version: '0.1.0' }));
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.result.hidden, true);
  assert.equal(v.els.back.hidden, true);
});

test('an admitted status result shows its folder and canvas, and Back returns home', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', workspace_permission_mode: 'managed-workspace-write', bound_project: '/w/acme/design/Landing' }));
  assert.equal(v.els.home.hidden, true);
  assert.equal(v.els.back.hidden, false);
  assert.equal(v.els['result-state'].textContent, 'This chat can use its project.');
  assert.deepEqual(fields(v), ['Folder', '/w/acme', 'Canvas', '/w/acme/design/Landing']);
  assert.ok(!texts(v.els.result).some((t) => /managed-workspace-write/.test(t)), 'no permission profile in the normal flow');
  assert.equal(v.document.activeElement, v.els['result-title']);
  assert.match(v.els.live.textContent, /This chat can use its project/);
  v.els.back.onclick();
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.result.hidden, true);
  assert.equal(v.els.back.hidden, true);
  assert.equal(v.document.activeElement, v.els.title);
});

test('a refused or retired status shows plain recovery, keeps the reason as a detail and no folder', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'retired', workspace_usable: false, workspace: '/w/old', workspace_problem: 'Codex workspace changed', bound_project: null }));
  assert.equal(v.els['result-state'].className, 'bad');
  assert.match(v.els['result-state'].textContent, /^This chat can't use a project folder\. Open the project in Codex and start from its chat\./);
  assert.deepEqual(fields(v), []);
  assert.equal(v.els['result-details'].hidden, false);
  assert.equal(v.els['result-detail'].textContent, 'Codex workspace changed');
  assert.ok(!texts(v.els.result).includes('/w/old'));
  v.from(result('Missing Codex workspace context. Use a real model tool call.\nmore', true));
  assert.equal(v.els['result-state'].textContent, "The agent's request was refused. Nothing was changed.");
  assert.equal(v.els['result-detail'].textContent, 'Missing Codex workspace context. Use a real model tool call.');
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

test('a canvases result lists this project\'s canvases; choosing one asks for a guarded open', async () => {
  const v = await initialized();
  v.from(result(CANVASES));
  assert.equal(v.els['result-title'].textContent, 'Canvases');
  assert.equal(v.els['result-state'].textContent, 'Canvases in this project:');
  assert.deepEqual(fields(v), ['Folder', '/w/acme']);
  const list = v.els['result-list'].children;
  assert.deepEqual(texts(list[0]), ['Alpha', 'design/Alpha · 2 pages', 'Open']);
  assert.deepEqual(texts(list[1]), ['Beta', 'design/Beta · 1 page · open now', 'Open']);
  assert.equal(list[2].textContent, "Skipped 1 folder that isn't a usable canvas.");
  assert.equal(buttons(list[0])[0].getAttribute('aria-label'), 'Open Alpha');
  assert.equal(v.last('ui/message'), undefined, 'listing sends nothing');
  buttons(list[0])[0].onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /^Open the OpenPaperDesign canvas "Alpha" described below\. First call openpaperdesign_status\./);
  assert.match(text, /Continue only if the workspace it reports is exactly the "workspace" value below; otherwise tell me and stop\. Bind the "project" value with bind_project\./);
  assert.match(text, /read the page you will show with get_tree_summary \(nodeId root and that page's pageId; the bind summary doesn't count\)/);
  assert.match(text, /Then call open_editor, open the URL it returns in the in-app browser, and tell me whether the canvas loaded\./);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'design/Alpha', canvas_id: 'c-alpha' });
  assert.match(v.els['msg-state'].textContent, /Sending to this chat/);
  onlyAllowed(v);
});

test('an empty or partial canvases result says so', async () => {
  const v = await initialized();
  v.from(result({ ...CANVASES, canvases: [], skipped: [] }));
  assert.equal(v.els['result-state'].textContent, 'No canvases under design/ in this project yet.');
  assert.equal(v.els['result-list'].children.length, 0);
  v.from(result({ ...CANVASES, complete: false }));
  assert.equal(v.els['result-state'].textContent, 'The first 2 canvases in this project:');
});

test('an empty but partial canvases result says more may exist, not that there are none', async () => {
  const v = await initialized();
  v.from(result({ ...CANVASES, canvases: [], skipped: [], complete: false }));
  const text = v.els['result-state'].textContent;
  assert.match(text, /stopped early before finding any canvases in design\/; more may exist/);
  assert.doesNotMatch(text, /No canvases under design\/ in this project yet/);
  assert.deepEqual(fields(v), ['Folder', '/w/acme']);
  assert.equal(v.els['result-list'].children.length, 0);
});

test('an open_editor result names its own canvas, project and folder, with an agent fallback', async () => {
  const v = await initialized();
  v.from(result(OPENED));
  assert.equal(v.els['result-title'].textContent, 'Canvas ready');
  assert.deepEqual(fields(v), ['Canvas', 'Beta (1 page)', 'Project', 'design/Beta', 'Folder', '/w/acme', 'Address', 'local, port 41234']);
  assert.equal(v.els.canvas.hidden, false);
  assert.equal(v.els['ask-open'].hidden, false);
  assert.ok(!JSON.stringify(texts(v.els.result)).includes(TOKEN), 'the token is not displayed');
  v.els['ask-open'].onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /^Open the OpenPaperDesign canvas described below in the in-app browser\. First call openpaperdesign_status\./);
  assert.match(text, /exactly the "workspace" value below and its bound project is exactly the "project_path" value; otherwise tell me and stop\. Call open_editor, open the URL it returns in the in-app browser, and tell me whether the canvas loaded\./);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'design/Beta', project_path: '/w/acme/design/Beta' });
  assert.ok(!text.includes(TOKEN), 'the agent gets a fresh link, not this one');
  onlyAllowed(v);
});

test('each result replaces the last; cancelled calls and Back leave nothing to open', async () => {
  const v = await initialized();
  v.from(result(OPENED));
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  assert.equal(v.els.canvas.hidden, true);
  assert.equal(v.els['ask-open'].hidden, true);
  assert.deepEqual(fields(v), ['Folder', '/w/acme', 'Canvas', 'none open yet']);
  v.from(result(OPENED));
  v.from({ method: 'ui/notifications/tool-cancelled', params: { reason: 'user' } });
  assert.equal(v.els.canvas.hidden, true);
  assert.equal(v.els['ask-open'].hidden, true);
  assert.deepEqual(fields(v), []);
  assert.equal(v.els['result-state'].textContent, 'The tool call was cancelled; there is nothing to open.');
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.last('ui/open-link'), undefined);
  v.from(result(OPENED));
  v.els.back.onclick();
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.last('ui/open-link'), undefined, 'Back drops the old link');
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
  assert.ok(v.els.canvas.disabled, 'busy while Codex answers');
  await flush();
  const open = v.last('ui/open-link');
  assert.deepEqual(open.params, { url: URL_OK });
  v.from({ id: open.id, result: {} });
  await flush();
  assert.ok(!v.els.canvas.disabled);
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/);
  assert.equal(v.els.live.textContent, v.els['canvas-state'].textContent);
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

test('the view source calls only the inventory tool and stores nothing', () => {
  assert.equal(script.split("'tools/call'").length, 2, 'one tools/call site');
  assert.match(script, /rpc\('tools\/call', \{ name: 'openpaperdesign_projects', arguments: \{\} \}\)/);
  for (const banned of ['resources/read', 'localStorage', 'sessionStorage', 'indexedDB', 'document.cookie', 'fetch(', 'XMLHttpRequest', 'innerHTML',
    'openpaperdesign_status\'', 'bind_project\'', 'create_project\'', 'open_editor\'', '@import', 'url(']) {
    assert.ok(!html.includes(banned) || banned === 'url(', banned);
    if (banned !== 'url(') assert.ok(!script.includes(banned), banned);
  }
  assert.ok(!/@import|@font-face|https?:\/\/(?!127\.0\.0\.1)/.test(html.slice(0, html.indexOf('<script>'))), 'no external stylesheet or font');
});
