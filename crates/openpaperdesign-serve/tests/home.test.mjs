// The home view (src/home.html) against a synthetic MCP Apps host: a fake parent window and a
// minimal fake DOM. Protocol and state behaviour only; this is not a real Codex host or
// browser, and it proves nothing about rendering or real accessibility.
// Run: node --test crates/openpaperdesign-serve/tests/home.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');
const script = html.slice(html.lastIndexOf('<script>') + 8, html.lastIndexOf('</script>'));
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setImmediate(r)); };
const TOKEN = 'ab'.repeat(32);
const URL_OK = `http://127.0.0.1:41234/#token=${TOKEN}`;
const ALLOWED = new Set(['ui/initialize', 'ui/notifications/initialized', 'ui/message', 'ui/open-link', 'tools/call']);
const INVENTORY = 'openpaperdesign_projects';

// Fictional fixtures. Two local projects share a name; Acme has two folders.
const ACME = { source: 'codex-app-server', host: 'this computer', id: 'prj_fixture_acme', name: 'Acme site', position: 0, recencyAt: null, roots: [{ path: '/w/acme' }, { path: '/w/acme-docs' }] };
const BETA = { source: 'codex-app-server', host: 'this computer', id: 'prj_fixture_beta', name: 'Acme site', position: 1, recencyAt: null, roots: [{ path: '/w/beta' }] };
const NOTES = { source: 'codex-app-server', host: 'this computer', id: 'prj_fixture_notes', name: 'Field notes', position: 2, recencyAt: null, roots: [{ path: '/w/notes' }] };
const LISTED = { available: true, source: 'codex-app-server', host: 'this computer', complete: true, projects: [ACME, BETA, NOTES] };

function el(tag = 'div', doc) {
  const attrs = {};
  const e = { tag, id: '', title: '', hidden: false, disabled: false, className: '', value: '', type: '', onclick: null,
    onsubmit: null, onkeydown: null, children: [] };
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
const texts = (e) => all(e).filter((n) => n.tag !== 'svg' && n.tag !== 'path').map((n) => n.textContent).filter(Boolean);
const buttons = (e) => all(e).filter((n) => n.tag === 'button');

const IDS = ['back', 'title', 'menu-button', 'menu', 'm-new', 'm-current', 'm-folder', 'm-refresh', 'live', 'notice', 'prompt',
  'home', 'inventory', 'projects', 'pick', 'pick-title', 'pick-list', 'new-form', 'new-title', 'name', 'where-field', 'where',
  'create', 'cancel-new', 'folder-help', 'folder-title', 'folder-refresh', 'folder-done', 'result', 'result-title',
  'result-state', 'result-fields', 'result-list', 'result-actions', 'canvas', 'ask-open', 'canvas-state', 'result-details',
  'result-meta', 'result-detail'];

function mount() {
  const document = { activeElement: null };
  const els = Object.fromEntries(IDS.map((id) => [id, el('div', document)]));
  for (const id of ['back', 'menu', 'prompt', 'pick', 'new-form', 'folder-help', 'result', 'canvas', 'ask-open', 'result-details', 'result-meta', 'result-detail']) {
    els[id].hidden = true;
  }
  const props = {};
  const rootAttrs = {};
  const documentElement = { style: { setProperty: (k, v) => { props[k] = v; } }, setAttribute: (k, v) => { rootAttrs[k] = String(v); } };
  const sent = [];
  let listener = null;
  // Copy into this realm so deep equality is not confused by the VM context's prototypes.
  const parent = { postMessage: (m) => sent.push(JSON.parse(JSON.stringify(m))) };
  const forbidden = () => { throw new Error('storage or network access'); };
  const window = { parent, addEventListener: (n, fn) => { if (n === 'message') listener = fn; } };
  for (const k of ['localStorage', 'sessionStorage', 'indexedDB', 'fetch', 'XMLHttpRequest']) {
    Object.defineProperty(window, k, { get: forbidden });
  }
  Object.assign(document, {
    getElementById: (id) => els[id], createElement: (tag) => el(tag, document),
    createElementNS: (_ns, tag) => el(tag, document), documentElement,
  });
  Object.defineProperty(document, 'cookie', { get: forbidden, set: forbidden });
  vm.runInNewContext(script, { window, document, setTimeout: () => 0, JSON, String, RegExp, Array, Map, Promise, Object, Number });
  const from = (m, source = parent) => listener({ source, data: { jsonrpc: '2.0', ...m } });
  const last = (method) => [...sent].reverse().find((m) => m.method === method);
  const count = (method) => sent.filter((m) => m.method === method).length;
  return { els, sent, from, last, count, parent, document, props, rootAttrs };
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
// The exact data line every targeted request ends with.
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
const USABLE = /First call openpaperdesign_status\. If workspace_usable is false, tell me the workspace_problem and stop/;
const EXACT_FOLDER = /Continue only if the workspace it reports is exactly the "folder" value below; if it isn't, tell me to open that folder's project in Codex and stop, without trying to switch folders/;
async function ack(v, method = 'ui/message', reply = { result: {} }) {
  v.from({ id: v.last(method).id, ...reply });
  await flush();
}
// The rows of the project list, as [name text, hint text, open button, details button].
const rows = (v, list = 'projects') => v.els[list].children.filter((li) => li.children.length).map((li) => {
  const row = li.children[0];
  const [open, more] = row.children;
  const [name, hint] = open.children[1].children;
  return { name: name.textContent, hint: hint ? hint.textContent : '', open, more, li };
});
const row = (v, name, hint = '') => {
  const r = rows(v).find((x) => x.name === name && x.hint === hint);
  assert.ok(r, `row ${name} ${hint}`);
  return r;
};

// ---- connection, inventory, rows ----

test('initialize claims only display modes; the menu waits for Codex; then the inventory is read', async () => {
  const v = mount();
  assert.equal(v.sent[0].method, 'ui/initialize');
  assert.deepEqual(v.sent[0].params.appCapabilities, { availableDisplayModes: ['inline', 'fullscreen'] });
  assert.ok(v.els['menu-button'].disabled, 'no action before Codex answers');
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized', 'tools/call']);
  assert.ok(!v.els['menu-button'].disabled);
  assert.deepEqual(rows(v).map((r) => r.name), ["This chat's project"], 'this chat\'s project is usable before the list arrives');
  onlyAllowed(v);
});

test('rendered for a model result, the view does not read the inventory', async () => {
  const v = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_status' } } } });
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized']);
  const home = await initialized({ context: { toolInfo: { tool: { name: 'openpaperdesign_home' } } } });
  assert.ok(methods(home).includes('tools/call'));
});

test('a failed initialize keeps the menu disabled, says so and sends nothing else', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, error: { code: -32603, message: 'at /Users/someone/private', data: { token: TOKEN } } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  assert.ok(v.els['menu-button'].disabled);
  assert.match(v.els.inventory.textContent, /Couldn't connect to Codex \(code -32603\)/);
  const shown = v.els.inventory.textContent + v.els.live.textContent;
  assert.ok(!shown.includes('/Users/') && !shown.includes(TOKEN));
});

test('rows show a name, a generic icon and only a short hint; same names stay distinguishable', async () => {
  const v = await initialized();
  assert.equal(v.els.inventory.hidden, true, 'no status line in the normal state');
  assert.deepEqual(rows(v).map((r) => [r.name, r.hint]),
    [["This chat's project", ''], ['Acme site', 'acme'], ['Acme site', 'beta'], ['Field notes', '']]);
  for (const r of rows(v)) {
    assert.equal(r.open.children[0].tag, 'svg', 'an icon (generic: Codex sends none)');
    assert.equal(r.open.children[0].getAttribute('aria-hidden'), 'true');
    assert.ok(!/\/w\//.test(r.hint), 'no paths in the ordinary list');
  }
  assert.equal(row(v, 'Acme site', 'acme').open.title, '/w/acme\n/w/acme-docs', 'folders as an optional tooltip');
  assert.equal(v.last('ui/message'), undefined, 'listing sends nothing');
});

test('details are keyboard-reachable buttons that reveal folders, source, host and id', async () => {
  const v = await initialized();
  const r = row(v, 'Acme site', 'acme');
  assert.equal(r.more.getAttribute('aria-label'), 'Details for Acme site, acme');
  assert.equal(r.more.getAttribute('aria-expanded'), 'false');
  const panel = r.li.children[1];
  assert.equal(panel.hidden, true);
  assert.equal(r.more.getAttribute('aria-controls'), panel.id);
  r.more.onclick();
  assert.equal(panel.hidden, false);
  assert.equal(r.more.getAttribute('aria-expanded'), 'true');
  assert.deepEqual(texts(panel), ['Folder 1', '/w/acme', 'Folder 2', '/w/acme-docs', 'Source', 'codex-app-server · this computer', 'Codex project id', 'prj_fixture_acme']);
  assert.equal(v.last('ui/message'), undefined, 'details send nothing');
  r.more.onclick();
  assert.equal(panel.hidden, true);
});

test('equal folder names fall back to a short id', async () => {
  const one = { ...BETA, id: 'prj_aaaaaa_111111', roots: [{ path: '/x/site' }] };
  const two = { ...BETA, id: 'prj_bbbbbb_222222', roots: [{ path: '/y/site' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [one, two] } });
  assert.deepEqual(rows(v).slice(1).map((r) => r.hint), ['…111111', '…222222']);
});

test('unavailable, empty and partial inventories say so; this chat\'s project stays usable', async () => {
  for (const [inventory, want] of [
    [{ available: false, reason: 'codex_not_found' }, /the codex command wasn't found/],
    [{ available: false, reason: 'unsupported' }, /doesn't share its project list/],
    [{ available: false, reason: 'protocol' }, /isn't available here\. You can still use this chat's project\./],
    [{ isError: true, content: [{ type: 'text', text: 'busy' }] }, /^Couldn't read your Codex projects\. Refresh/],
    [{ error: { code: -32601, message: 'no such tool at /private' } }, /^Couldn't read your Codex projects \(code -32601\)\./],
  ]) {
    const v = await initialized({ inventory });
    assert.match(v.els.inventory.textContent, want);
    assert.equal(v.els.inventory.hidden, false);
    assert.ok(!v.els.inventory.textContent.includes('/private'));
    assert.deepEqual(rows(v).map((r) => r.name), ["This chat's project"]);
  }
  const none = await initialized({ inventory: { ...LISTED, projects: [] } });
  assert.match(none.els.inventory.textContent, /^Codex has no saved projects on this computer yet\./);
  const partial = await initialized({ inventory: { ...LISTED, complete: false } });
  assert.equal(partial.els.inventory.textContent, 'Showing the first 3 Codex projects.');
});

test('an empty but partial inventory says more may exist, not that there are none', async () => {
  const v = await initialized({ inventory: { ...LISTED, projects: [], complete: false } });
  assert.match(v.els.inventory.textContent, /stopped early and returned none; more may exist/);
  assert.doesNotMatch(v.els.inventory.textContent, /no saved projects/);
});

// ---- opening a project ----

test('a one-folder project opens directly: one request with the exact folder and both stops', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'beta').open.onclick();
  await flush();
  assert.equal(v.count('ui/message'), 1);
  const text = lastMessage(v);
  assert.match(text, /^Use OpenPaperDesign in my Codex project "Acme site", in the folder given below\./);
  assert.match(text, USABLE);
  assert.match(text, EXACT_FOLDER);
  assert.match(text, /call openpaperdesign_canvases/);
  assert.match(text, /Never pass force to bind_project/);
  assert.deepEqual(selection(text), { codex_project_id: 'prj_fixture_beta', source: 'codex-app-server', host: 'this computer', folder: '/w/beta' });
  assert.equal(v.els.notice.textContent, 'Asking the agent in this chat to open Acme site · beta…');
  await ack(v);
  assert.equal(v.els.notice.textContent, 'Asked the agent in this chat to open Acme site · beta. Its reply appears here when it arrives.');
  assert.doesNotMatch(v.els.notice.textContent, /opened|loaded|saved/i, 'a request is not an opened project');
  assert.equal(v.els.live.textContent, v.els.notice.textContent);
  onlyAllowed(v);
});

test('a project with several folders asks which one, then opens that exact folder', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'acme').open.onclick();
  await flush();
  assert.equal(v.last('ui/message'), undefined, 'ambiguity is asked, not guessed');
  assert.equal(v.els.pick.hidden, false);
  assert.equal(v.els.home.hidden, true);
  assert.equal(v.els.back.hidden, false);
  assert.equal(v.els['pick-title'].textContent, 'Which folder of Acme site · acme?');
  assert.equal(v.document.activeElement, v.els['pick-title']);
  const picks = rows(v, 'pick-list');
  assert.deepEqual(picks.map((p) => [p.name, p.hint]), [['acme', '/w/acme'], ['acme-docs', '/w/acme-docs']]);
  picks[1].open.onclick();
  await flush();
  assert.deepEqual(selection(lastMessage(v)).folder, '/w/acme-docs');
  assert.match(lastMessage(v), EXACT_FOLDER);
});

test('Escape or Back leaves the folder choice without sending', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'acme').open.onclick();
  v.els.pick.onkeydown({ key: 'Escape', preventDefault() {} });
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.pick.hidden, true);
  assert.equal(v.document.activeElement, v.els.title);
  row(v, 'Acme site', 'acme').open.onclick();
  v.els.back.onclick();
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.last('ui/message'), undefined);
});

test('this chat\'s project opens its canvases with the usability stop and no folder data', async () => {
  const v = await initialized();
  row(v, "This chat's project").open.onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /^Open an existing OpenPaperDesign canvas in this workspace\./);
  assert.match(text, USABLE);
  assert.ok(!text.includes(DATA), 'no folder data for this chat\'s own project');
});

test('while a request is in flight rows and the menu are busy, so nothing is sent twice', async () => {
  const v = await initialized();
  row(v, 'Field notes').open.onclick();
  await flush();
  assert.ok(rows(v).every((r) => r.open.disabled) && v.els['menu-button'].disabled);
  row(v, 'Acme site', 'beta').open.onclick();
  await flush();
  assert.equal(v.count('ui/message'), 1);
  await ack(v, 'ui/message', { error: { code: -32601, message: 'no' } });
  assert.equal(v.els.notice.textContent, "Codex didn't accept the request (code -32601). Try again.");
  assert.ok(rows(v).every((r) => !r.open.disabled) && !v.els['menu-button'].disabled, 'retry possible');
});

test('a valid root and id with a backtick reach the agent exactly (R1-02 of #6)', async () => {
  const odd = { ...BETA, id: 'prj`fixture', roots: [{ path: '/w/acme`docs' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [odd] } });
  rows(v)[1].open.onclick();
  await flush();
  const sel = selection(lastMessage(v));
  assert.equal(sel.folder, '/w/acme`docs');
  assert.equal(sel.codex_project_id, 'prj`fixture');
});

test('a hostile name is cleaned for display; the exact path is only data', async () => {
  const evil = { ...BETA, name: 'A"B`C\u0007\n', roots: [{ path: '/w/x"y\nz' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [evil] } });
  rows(v)[1].open.onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, /project "A B C", in the folder given below/);
  assert.equal(text.split('\n').length, 2, 'instructions plus one data line; no injected line');
  assert.equal(selection(text).folder, '/w/x"y\nz');
});

// ---- the New menu ----

test('the New menu opens on its labelled button, moves with arrows and closes on Escape with focus back', async () => {
  const v = await initialized();
  const button = v.els['menu-button'];
  assert.match(html, /id="menu-button" class="btn" type="button" aria-haspopup="menu" aria-expanded="false" aria-controls="menu"/);
  button.onclick();
  assert.equal(v.els.menu.hidden, false);
  assert.equal(button.getAttribute('aria-expanded'), 'true');
  assert.equal(v.document.activeElement, v.els['m-new']);
  const key = (k) => v.els.menu.onkeydown({ key: k, preventDefault() {} });
  key('ArrowDown');
  assert.equal(v.document.activeElement, v.els['m-current']);
  key('End');
  assert.equal(v.document.activeElement, v.els['m-refresh']);
  key('ArrowDown');
  assert.equal(v.document.activeElement, v.els['m-new'], 'wraps around');
  key('ArrowUp');
  assert.equal(v.document.activeElement, v.els['m-refresh']);
  key('Home');
  assert.equal(v.document.activeElement, v.els['m-new']);
  key('Escape');
  assert.equal(v.els.menu.hidden, true);
  assert.equal(button.getAttribute('aria-expanded'), 'false');
  assert.equal(v.document.activeElement, button);
  button.onclick();
  key('Tab');
  assert.equal(v.els.menu.hidden, true, 'Tab closes it too');
  assert.equal(v.last('ui/message'), undefined, 'the menu itself sends nothing');
});

test('New canvas asks where only when there is a choice, and targets the exact folder', async () => {
  const lone = await initialized({ inventory: { ...LISTED, projects: [] } });
  lone.els['m-new'].onclick();
  assert.equal(lone.els['where-field'].hidden, true, 'no destination choice without projects');
  const v = await initialized();
  v.els['menu-button'].onclick();
  v.els['m-new'].onclick();
  assert.equal(v.els['new-form'].hidden, false);
  assert.equal(v.els.menu.hidden, true);
  assert.equal(v.document.activeElement, v.els.name);
  assert.equal(v.els['where-field'].hidden, false);
  assert.deepEqual(v.els.where.children.map((o) => [o.value, o.textContent]),
    [['', "This chat's project"], ['0', 'Acme site · acme · acme'], ['1', 'Acme site · acme · acme-docs'], ['2', 'Acme site · beta'], ['3', 'Field notes']]);
  v.els.name.value = '  Landing "page"\u0007 `x`  ';
  v.els.where.value = '1';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  let text = lastMessage(v);
  assert.match(text, /^Create a new OpenPaperDesign canvas named "Landing page x" in my Codex project "Acme site", in the folder given below\./);
  assert.match(text, USABLE);
  assert.match(text, EXACT_FOLDER);
  assert.match(text, /root DESIGN\.md \(use an existing design\.md as is; never add a second file that differs only in case\)/);
  assert.equal(selection(text).folder, '/w/acme-docs');
  await ack(v);
  v.els.where.value = '';
  v.els.name.value = '';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  text = lastMessage(v);
  assert.match(text, /Ask me for its name first/);
  assert.ok(!text.includes(DATA), 'this chat\'s project: no folder data');
  v.els['cancel-new'].onclick();
  assert.equal(v.els.home.hidden, false);
});

test('Open current canvas asks with the usability stop and never creates or binds', async () => {
  const v = await initialized();
  v.els['m-current'].onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, USABLE);
  assert.match(text, /If no project is bound in this chat, tell me so and stop; don't create or bind anything\./);
  assert.match(text, /read the page you will show with get_tree_summary/);
});

test('Add a folder explains the supported Codex route and refreshes on request', async () => {
  const v = await initialized();
  v.els['m-folder'].onclick();
  assert.equal(v.els['folder-help'].hidden, false);
  assert.equal(v.document.activeElement, v.els['folder-title']);
  assert.equal(v.last('ui/message'), undefined, 'no picker, no request, no path input');
  v.els['folder-refresh'].onclick();
  await flush();
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.count('tools/call'), 2);
  v.els['m-refresh'].onclick();
  await flush();
  assert.equal(v.count('tools/call'), 3);
  onlyAllowed(v);
});

test('an older inventory reply never replaces a newer one', async () => {
  const v = await initialized({ inventory: null });
  const first = v.last('tools/call');
  v.els['m-refresh'].onclick();
  await flush();
  const second = v.last('tools/call');
  v.from({ id: second.id, result: toolText({ ...LISTED, projects: [NOTES] }) });
  await flush();
  v.from({ id: first.id, result: toolText(LISTED) });
  await flush();
  assert.deepEqual(rows(v).map((r) => r.name), ["This chat's project", 'Field notes']);
});

// ---- copy mode (no message.text) ----

test('copy mode shows text for the action just chosen, and drops it when the target changes (F3)', async () => {
  const v = await initialized({ caps: {} });
  assert.match(v.els.notice.textContent, /can't send requests from apps/);
  row(v, 'Acme site', 'beta').open.onclick();
  await flush();
  assert.equal(v.last('ui/message'), undefined);
  assert.equal(v.els.prompt.hidden, false);
  assert.equal(selection(v.els.prompt.textContent).folder, '/w/beta');
  row(v, 'Field notes').open.onclick();
  await flush();
  assert.equal(selection(v.els.prompt.textContent).folder, '/w/notes', 'the new choice replaces the old text');
  row(v, 'Acme site', 'acme').open.onclick();
  assert.equal(v.els.prompt.hidden, true, 'a new target clears the old text');
  assert.equal(v.els.prompt.textContent, '');
  v.els.back.onclick();
  assert.equal(v.els.prompt.hidden, true);
  row(v, "This chat's project").open.onclick();
  await flush();
  assert.ok(!v.els.prompt.textContent.includes(DATA), 'this chat\'s project: no folder');
  v.els['m-new'].onclick();
  assert.equal(v.els.prompt.hidden, true, 'a new intent clears it');
  assert.equal(v.count('ui/message'), 0);
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
  assert.deepEqual(v.props, { '--color-background-primary': '#101010', '--color-text-primary': 'rgb(240 240 240)', '--font-sans': 'Inter, system-ui' });
  v.from({ method: 'ui/notifications/host-context-changed', params: { theme: 'light' } });
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.equal(v.props['--color-background-primary'], '#101010', 'partial update keeps earlier styles');
  const before = { ...v.props };
  v.from({ method: 'ui/notifications/host-context-changed', params: { styles: { css: { fonts: '@import url(https://fonts.invalid/x.css);' } } } });
  assert.deepEqual(v.props, before, 'font-loading CSS is never applied');
  v.from({ method: 'ui/notifications/host-context-changed', params: { theme: 'dark' } }, { postMessage() {} });
  assert.equal(v.rootAttrs['data-theme'], 'light', 'another window is ignored');
  assert.equal(v.count('tools/call'), 1, 'host context grants nothing and triggers no call');
});

// ---- delivered results ----

const result = (obj, isError = false) => ({ method: 'ui/notifications/tool-result', params: { content: [{ type: 'text', text: typeof obj === 'string' ? obj : JSON.stringify(obj) }], isError } });
const fields = (v) => v.els['result-fields'].children.map((c) => c.textContent);
// What Details holds for a result, or null while Details is hidden.
const meta = (v) => (v.els['result-details'].hidden ? null : v.els['result-meta'].children.map((c) => c.textContent));
const OPENED = (name) => ({ url: URL_OK, workspace: '/w/acme', project: 'design/' + name, project_path: '/w/acme/design/' + name, canvas: { id: 'c-' + name, name, pages: 1 } });
const CANVASES = {
  workspace: '/w/acme', design_present: true, complete: true,
  canvases: [
    { project: 'design/Alpha', path: '/w/acme/design/Alpha', bound: false, canvas: { id: 'c-alpha', name: 'Alpha', pages: 2, modified: 1 } },
    { project: 'design/Beta', path: '/w/acme/design/Beta', bound: true, canvas: { id: 'c-beta', name: 'Beta', pages: 1, modified: 2 } },
  ],
  skipped: [{ project: 'design/Escape', problem: 'a symbolic link (not followed)' }],
};

test('a canvases result lists canvases; bound means current, not open; opening one is a guarded request', async () => {
  const v = await initialized();
  v.from(result(CANVASES));
  assert.equal(v.els['result-title'].textContent, 'Canvases');
  assert.equal(v.els.back.hidden, false);
  assert.deepEqual(fields(v), ['Folder', 'acme'], 'concise identity in view');
  assert.deepEqual(meta(v), ['Folder path', '/w/acme'], 'the exact folder under Details');
  const list = rows(v, 'result-list');
  assert.deepEqual(list.map((r) => [r.name, r.hint]), [['Alpha', '2 pages'], ['Beta', '1 page · current']]);
  assert.ok(!texts(v.els.result).some((t) => /open now/.test(t)), 'binding is not a loaded editor');
  assert.equal(v.els['result-list'].children[2].textContent, "Skipped 1 folder that isn't a usable canvas.");
  list[0].open.onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, USABLE);
  assert.match(text, /Continue only if the workspace it reports is exactly the "workspace" value below; otherwise tell me and stop\. Bind the "project" value with bind_project\./);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'design/Alpha', canvas_id: 'c-alpha' });
});

test('empty and cut-off canvas lists say which they are', async () => {
  const v = await initialized();
  v.from(result({ ...CANVASES, canvases: [], skipped: [] }));
  assert.match(v.els['result-state'].textContent, /^No canvases under design\/ in this project yet\./);
  v.from(result({ ...CANVASES, canvases: [], skipped: [], complete: false }));
  assert.match(v.els['result-state'].textContent, /stopped early before finding any canvases in design\/; more may exist/);
  v.from(result({ ...CANVASES, complete: false }));
  assert.equal(v.els['result-state'].textContent, 'The first 2 canvases in this project:');
});

test('a status with a bound canvas offers a guarded resume; without one it offers nothing', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', workspace_permission_mode: 'managed-workspace-write', bound_project: '/w/acme/design/Landing' }));
  assert.equal(v.els['result-state'].textContent, 'A canvas is bound in this chat.');
  assert.deepEqual(fields(v), ['Canvas', 'Landing', 'Folder', 'acme']);
  assert.deepEqual(meta(v), ['Folder path', '/w/acme', 'Canvas path', '/w/acme/design/Landing']);
  assert.ok(!texts(v.els.result).some((t) => /managed-workspace-write/.test(t)), 'no permission profile');
  assert.equal(v.els['ask-open'].hidden, false);
  v.els['ask-open'].onclick();
  await flush();
  const text = lastMessage(v);
  assert.match(text, USABLE);
  assert.match(text, /exactly the "workspace" value below and its bound project is exactly the "project_path" value/);
  assert.deepEqual(selection(text), { workspace: '/w/acme', project: 'Landing', project_path: '/w/acme/design/Landing' });
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  assert.equal(v.els['ask-open'].hidden, true);
  assert.equal(v.els['result-state'].textContent, 'No canvas is bound in this chat yet.');
});

test('a refused or retired status shows plain recovery and no folder', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'retired', workspace_usable: false, workspace: '/w/old', workspace_problem: 'Codex workspace changed', bound_project: null }));
  assert.equal(v.els['result-state'].className, 'bad');
  assert.match(v.els['result-state'].textContent, /^This chat can't use a project folder\./);
  assert.deepEqual(fields(v), []);
  assert.equal(v.els['result-detail'].textContent, 'Codex workspace changed');
  assert.ok(!texts(v.els.result).includes('/w/old'));
});

test('a failed step reports only that step; it never claims earlier work was undone (F5)', async () => {
  const v = await initialized();
  v.from(result('The browser editor is not built in.\nmore', true));
  const text = v.els['result-state'].textContent;
  assert.doesNotMatch(text, /Nothing was changed|nothing changed|rolled back|undone/i);
  assert.match(text, /last step didn't work\. Anything it finished before that may already have happened; check the chat/);
  assert.equal(v.els['result-detail'].textContent, 'The browser editor is not built in.');
});

test('an open_editor result names its own canvas; the token stays hidden; the ack is not a load', async () => {
  for (const bad of ['https://127.0.0.1:1/#token=' + TOKEN, 'http://127.0.0.1:1/x#token=' + TOKEN,
    'http://localhost:1/#token=' + TOKEN, 'http://127.0.0.1:1/#token=short', URL_OK + '&x=1', 'javascript:alert(1)']) {
    const v = await initialized();
    v.from(result({ url: bad }));
    assert.equal(v.els.canvas.hidden, true, bad);
    assert.match(v.els['result-state'].textContent, /won't be opened/);
  }
  const v = await initialized();
  v.from(result(OPENED('Beta')));
  assert.equal(v.els['result-title'].textContent, 'Beta');
  assert.deepEqual(fields(v), ['Canvas', 'Beta (1 page)', 'Folder', 'acme']);
  assert.deepEqual(meta(v), ['Project', 'design/Beta', 'Folder path', '/w/acme', 'Address', 'local, port 41234']);
  assert.ok(!JSON.stringify(meta(v)).includes(TOKEN) && !JSON.stringify(meta(v)).includes('#token'), 'no link or token in Details');
  assert.ok(!JSON.stringify(texts(v.els.result)).includes(TOKEN));
  v.els.canvas.onclick();
  assert.ok(v.els.canvas.disabled);
  await flush();
  assert.deepEqual(v.last('ui/open-link').params, { url: URL_OK });
  await ack(v, 'ui/open-link');
  assert.ok(!v.els.canvas.disabled);
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/, 'positive current reply is shown');
  v.els['ask-open'].onclick();
  await flush();
  assert.ok(!lastMessage(v).includes(TOKEN), 'the agent gets a fresh link');
  onlyAllowed(v);
});

// ---- late replies belong to the view that started them (F1) ----

test('a pending open of Alpha neither blocks Beta nor reports under it (F1)', async () => {
  const v = await initialized();
  v.from(result(OPENED('Alpha')));
  v.els.canvas.onclick();
  await flush();
  const alpha = v.last('ui/open-link');
  v.from(result(OPENED('Beta')));
  assert.equal(v.els.canvas.disabled, false, 'Beta\'s button is usable');
  v.from({ id: alpha.id, error: { code: -32000, message: 'x' } });
  await flush();
  assert.equal(v.els['canvas-state'].textContent, '', 'Alpha\'s failure is not shown under Beta');
  assert.equal(v.els['result-title'].textContent, 'Beta');
});

test('late open replies after cancellation or Back change nothing (F1)', async () => {
  const v = await initialized();
  v.from(result(OPENED('Alpha')));
  v.els.canvas.onclick();
  await flush();
  v.from({ method: 'ui/notifications/tool-cancelled', params: { reason: 'user' } });
  await ack(v, 'ui/open-link');
  assert.equal(v.els['canvas-state'].textContent, '');
  assert.equal(v.els.live.textContent, 'Cancelled');
  v.from(result(OPENED('Alpha')));
  v.els.canvas.onclick();
  await flush();
  v.els.back.onclick();
  await ack(v, 'ui/open-link');
  assert.equal(v.els.live.textContent, 'Projects');
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.count('ui/open-link'), 2, 'no link after Back');
});

test('a late request ack does not replace a delivered refusal, and busy ends with the view (F1)', async () => {
  const v = await initialized();
  row(v, 'Field notes').open.onclick();
  await flush();
  v.from(result('refused', true));
  const announced = v.els.live.textContent;
  assert.ok(!v.els['menu-button'].disabled, 'the new view is not busy');
  await ack(v);
  assert.equal(v.els.live.textContent, announced);
  assert.equal(v.els.notice.textContent, '');
});

test('a late inventory reply after a result renders quietly and announces nothing (F1)', async () => {
  const v = await initialized({ inventory: null });
  const call = v.last('tools/call');
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  const announced = v.els.live.textContent;
  v.from({ id: call.id, result: toolText(LISTED) });
  await flush();
  assert.equal(v.els.live.textContent, announced);
  v.els.back.onclick();
  assert.equal(rows(v).length, 4, 'the list is there when you go back');
});

test('a delivered home result revokes an earlier editor link and its fallback (F4)', async () => {
  const v = await initialized();
  v.from(result(OPENED('Alpha')));
  v.from(result({ surface: 'home', version: '0.1.0' }));
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.result.hidden, true);
  v.els.canvas.onclick();
  v.els['ask-open'].onclick();
  await flush();
  assert.equal(v.count('ui/open-link'), 0);
  assert.equal(v.count('ui/message'), 0);
});

test('messages from other windows are ignored', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { hostCapabilities: { message: { text: {} } } } }, { postMessage() {} });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  v.from(result({ url: URL_OK }), { postMessage() {} });
  assert.equal(v.els.canvas.hidden, true);
});

// ---- R2: intent and control ownership ----

const submitNew = async (v) => { v.els['new-form'].onsubmit({ preventDefault() {} }); await flush(); };
const options = (v) => v.els.where.children.map((o) => o.textContent);

test('a refresh while New canvas is open never changes what a shown destination sends (R2-F1, P14)', async () => {
  const OTHER = { ...BETA, roots: [{ path: '/w/other' }] };
  for (const [kind, next, nextOptions] of [
    ['reorder', [NOTES, BETA, ACME], ["This chat's project", 'Field notes', 'Acme site · beta', 'Acme site · acme · acme', 'Acme site · acme · acme-docs']],
    ['removal', [ACME], ["This chat's project", 'Acme site · acme', 'Acme site · acme-docs']],
    ['root replacement', [OTHER, ACME, NOTES], ["This chat's project", 'Acme site · other', 'Acme site · acme · acme', 'Acme site · acme · acme-docs', 'Field notes']],
  ]) {
    const v = await initialized();
    v.els['m-refresh'].onclick();
    await flush();
    const pending = v.last('tools/call');
    v.els['m-new'].onclick();
    v.els.where.value = '2';
    v.els.where.onchange();
    v.els.name.value = 'Draft';
    v.els.name.oninput();
    const shown = v.els.where.children.find((o) => o.value === '2').textContent;
    assert.equal(shown, 'Acme site · beta', kind);
    v.from({ id: pending.id, result: toolText({ ...LISTED, projects: next }) });
    await flush();
    assert.equal(v.els['new-form'].hidden, false, 'the open form stays as it was');
    assert.equal(v.els.where.children.find((o) => o.value === '2').textContent, shown);
    await submitNew(v);
    const text = lastMessage(v);
    assert.match(text, /in my Codex project "Acme site", in the folder given below/);
    assert.deepEqual(selection(text), { codex_project_id: 'prj_fixture_beta', source: 'codex-app-server', host: 'this computer', folder: '/w/beta' }, kind);
    assert.match(text, USABLE);
    assert.match(text, EXACT_FOLDER, 'a listed root is still only checked, never authority');
    await ack(v);
    v.els['m-new'].onclick();
    assert.deepEqual(options(v), nextOptions, 'the next form uses the refreshed list');
  }
});

test('a late inventory reply leaves the visible canvas rows to their own request (R2-F2, P15)', async () => {
  for (const [kind, reply, state] of [
    ['acknowledged', { result: {} }, /^Asked the agent in this chat to open Alpha\./],
    ['refused', { error: { code: -32601, message: 'no' } }, /^Codex didn't accept the request \(code -32601\)\. Try again\.$/],
  ]) {
    const v = await initialized({ inventory: null });
    const inventory = v.last('tools/call');
    v.from(result(CANVASES));
    const list = rows(v, 'result-list');
    list[0].open.onclick();
    await flush();
    assert.ok(list.every((r) => r.open.disabled) && v.els['menu-button'].disabled, kind);
    v.from({ id: inventory.id, result: toolText(LISTED) });
    await flush();
    assert.equal(v.els.result.hidden, false, 'the canvases stay in view');
    assert.ok(rows(v).every((r) => r.open.disabled), 'the hidden home\'s new rows wait for the same request');
    assert.ok(list.every((r) => r.open.disabled), 'the visible rows are still owned by the pending request');
    assert.ok(list.every((r) => !r.more.disabled), 'details only show, so they stay usable');
    await ack(v, 'ui/message', reply);
    assert.match(v.els.notice.textContent, state);
    assert.ok(list.every((r) => !r.open.disabled), `${kind}: the visible canvas rows are usable again`);
    assert.ok(!v.els['menu-button'].disabled);
    assert.ok(rows(v).every((r) => !r.open.disabled));
    assert.equal(v.count('ui/message'), 1, 'nothing was resent');
  }
});

test('copy text goes when its name or destination changes, or the next choice cannot proceed (R2-F3, P11-P13)', async () => {
  const EMPTY = { ...NOTES, id: 'prj_fixture_empty', name: 'Empty', roots: [] };
  const v = await initialized({ caps: {}, inventory: { ...LISTED, projects: [ACME, BETA, NOTES, EMPTY] } });
  v.els['m-new'].onclick();
  v.els.name.value = 'Draft A';
  v.els.name.oninput();
  v.els.where.value = '2';
  v.els.where.onchange();
  await submitNew(v);
  assert.equal(v.els.prompt.hidden, false);
  assert.match(v.els.prompt.textContent, /named "Draft A"/);
  assert.equal(selection(v.els.prompt.textContent).folder, '/w/beta');
  v.els.where.value = '3';
  v.els.where.onchange();
  assert.equal(v.els.prompt.hidden, true, 'a new destination drops the old text (P11)');
  assert.equal(v.els.prompt.textContent, '');
  assert.match(v.els.notice.textContent, /can't send requests from apps/);
  await submitNew(v);
  assert.equal(selection(v.els.prompt.textContent).folder, '/w/notes', 'the next deliberate Create makes fresh text');
  v.els.name.value = 'Draft B';
  v.els.name.oninput();
  assert.equal(v.els.prompt.hidden, true, 'an edited name drops the old text (P12)');
  await submitNew(v);
  assert.match(v.els.prompt.textContent, /named "Draft B"/);
  v.els.back.onclick();
  row(v, 'Field notes').open.onclick();
  await flush();
  assert.equal(selection(v.els.prompt.textContent).folder, '/w/notes');
  row(v, 'Empty').open.onclick();
  assert.equal(v.els.prompt.hidden, true, 'a project that cannot proceed drops the old text (P13)');
  assert.equal(v.els.notice.textContent, 'Empty has no folder in Codex.');
  assert.equal(v.count('ui/message'), 0, 'copy mode never sends');
});

test('destinations with equal folder names show the shortest distinguishing path (R2-F4, P16)', async () => {
  const SITE = { ...BETA, id: 'prj_fixture_site', name: 'Site', roots: [{ path: '/work/current/site' }, { path: '/archive/site' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [SITE] } });
  assert.deepEqual(rows(v).map((r) => [r.name, r.hint]), [["This chat's project", ''], ['Site', '2 folders']], 'the list stays quiet');
  v.els['m-new'].onclick();
  assert.deepEqual(v.els.where.children.slice(1).map((o) => [o.textContent, o.title]),
    [['Site · current/site', '/work/current/site'], ['Site · archive/site', '/archive/site']]);
  v.els.where.value = '1';
  v.els.name.value = 'Draft';
  await submitNew(v);
  assert.deepEqual(selection(lastMessage(v)), { codex_project_id: 'prj_fixture_site', source: 'codex-app-server', host: 'this computer', folder: '/archive/site' });
  await ack(v);
  v.els['cancel-new'].onclick();
  row(v, 'Site', '2 folders').open.onclick();
  assert.deepEqual(rows(v, 'pick-list').map((r) => [r.name, r.hint]),
    [['current/site', '/work/current/site'], ['archive/site', '/archive/site']], 'the folder question matches');
});

test('results keep exact paths and the port under Details, cleared for each new result (R2-F5, P17)', async () => {
  const v = await initialized();
  const inView = () => texts(v.els['result-fields']).concat(texts(v.els['result-state']), texts(v.els['result-list'])).join('\n');
  for (const r of [{ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: '/w/acme/design/Alpha' }, CANVASES, OPENED('Alpha')]) {
    v.from(result(r));
    assert.doesNotMatch(inView(), /\/w\/|41234/, 'no full folder or port in the normal view');
    assert.equal(v.els['result-details'].hidden, false, 'Details is there to check them');
    assert.equal(v.els['result-details'].open, false, 'and starts closed');
    assert.ok(meta(v).includes('/w/acme'));
    v.els['result-details'].open = true;
  }
  assert.ok(!JSON.stringify(meta(v)).includes(TOKEN));
  v.from(result('The browser editor is not built in.', true));
  assert.deepEqual(meta(v), [], 'no earlier result\'s metadata');
  assert.equal(v.els['result-meta'].hidden, true);
  assert.equal(v.els['result-detail'].hidden, false);
  assert.equal(v.els['result-details'].open, false);
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  assert.equal(v.els['result-detail'].textContent, '', 'no earlier refusal reason');
  assert.equal(v.els['result-detail'].hidden, true);
  assert.deepEqual(meta(v), ['Folder path', '/w/acme']);
  v.from({ method: 'ui/notifications/tool-cancelled', params: {} });
  assert.equal(meta(v), null, 'nothing to show, so no Details');
});

// ---- static ----

test('the view calls only the inventory tool, stores nothing and embeds its one font locally', () => {
  assert.equal(script.split("'tools/call'").length, 2, 'one tools/call site');
  for (const banned of ['resources/read', 'localStorage', 'sessionStorage', 'indexedDB', 'document.cookie', 'fetch(', 'XMLHttpRequest', 'innerHTML',
    'openpaperdesign_status\'', 'bind_project\'', 'create_project\'', 'open_editor\'', '@import']) {
    assert.ok(!html.includes(banned), banned);
  }
  const style = html.slice(0, html.indexOf('</style>'));
  assert.deepEqual(style.match(/url\([^)]*\)/g), ['url("data:font/woff2;base64,__OPD_WORDMARK_FONT__")'], 'the only url() is the served font slot');
  assert.ok(!/https?:\/\/(?!127\.0\.0\.1)/.test(style), 'no external stylesheet or font');
  assert.match(style, /\.wordmark \{ font: 400 16px\/1\.2 "Geist Pixel", monospace;/);
  assert.match(html, /<footer><span class="wordmark">onlinesourdough<\/span><\/footer>/);
  assert.match(html, /aria-label="Back to projects"/);
});
