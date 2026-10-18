// The home view (src/home.html) against a synthetic MCP Apps host: a fake parent window and a
// minimal fake DOM. Protocol and state behaviour only; this is not a real Codex host or
// browser, and it proves nothing about rendering or real accessibility.
// Run: node --test crates/openpaperdesign-serve/tests/home.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
// The editor's own status rule, which the assembled module puts in scope with the panel.
import { opdQuietStatus, opdStatusDisplay, opdStatusKind } from '../../openpaperdesign-web/static/editor.js';
// The real save tick and session, for the save-failure cases (R25-F2).
import { discardStep, saveStep } from '../../openpaperdesign-web/static/page.js';
import { Session } from '../../openpaperdesign-web/static/session.js';

const html = readFileSync(new URL('../src/home.html', import.meta.url), 'utf8');
// The view's one inline module. Its first line is the editor slot (`opdCreatePanel = null`,
// filled in by an app-library server); the rest is a plain script, so it runs here as one, in
// a fresh context. Tests that need the in-panel editor put a recording fake in the slot; the
// real module (renderer glue, session, page, editor, panel) is checked in
// crates/openpaperdesign-web/tests/panel.test.mjs.
const MODULE_OPEN = '<script type="module">';
const PANEL_SLOT = 'const opdCreatePanel = null; // __OPD_PANEL_MODULE__';
const script = html.slice(html.lastIndexOf(MODULE_OPEN) + MODULE_OPEN.length, html.lastIndexOf('</script>'));
assert.ok(script.startsWith('\n' + PANEL_SLOT + '\n'), 'the editor slot is the module\'s first line');
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setImmediate(r)); };
const TOKEN = 'ab'.repeat(32);
const URL_OK = `http://127.0.0.1:41234/#token=${TOKEN}`;
// The view asks for no display mode: the editor fills whatever surface Codex gives it.
const ALLOWED = new Set(['ui/initialize', 'ui/notifications/initialized', 'ui/open-link', 'tools/call']);
const INVENTORY = 'openpaperdesign_projects';
// Every server tool the view may call: the inventory, the host folder picker and the four
// direct actions. The server admits the actions only with the call's own workspace context.
const TOOLS = new Set([INVENTORY, 'openpaperdesign_choose_folder', 'openpaperdesign_ui_canvases', 'openpaperdesign_ui_create',
  'openpaperdesign_ui_open', 'openpaperdesign_ui_editor']);
// What the view may call when an app-library server serves it: only the library's own actions.
const LIB_LIST = 'openpaperdesign_app_canvases';
const LIB_TOOLS = new Set([LIB_LIST, 'openpaperdesign_app_create', 'openpaperdesign_app_open', 'openpaperdesign_app_editor',
  'openpaperdesign_app_close', 'openpaperdesign_app_import', 'openpaperdesign_app_export', 'openpaperdesign_app_rename',
  // The kept appearance (a view preference; no canvas).
  'openpaperdesign_app_appearance']);

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

const IDS = ['back', 'title', 'menu-button', 'menu-icon', 'menu-label', 'menu', 'm-new', 'm-current', 'm-folder', 'm-import', 'm-refresh',
  'live', 'notice', 'import-file', 'home', 'h-projects', 'lib-bar', 'lib-context', 'search', 'layout-list', 'layout-grid', 'new-file',
  'inventory', 'projects', 'no-match', 'rename-form', 'rename-title', 'rename-name', 'rename-save', 'rename-cancel',
  'pick', 'pick-title', 'pick-list', 'new-form', 'new-title', 'name', 'where-field', 'where',
  'create', 'cancel-new', 'folder-help', 'folder-title', 'folder-refresh', 'folder-done', 'result', 'result-title',
  'result-state', 'result-fields', 'result-list', 'result-actions', 'canvas', 'reopen', 'force', 'close', 'export', 'canvas-state',
  'result-details', 'result-meta', 'result-detail', 'app', 'editor', 'ed-title', 'ed-status', 'ed-renderer', 'ed-stage', 'ed-canvas',
  'ed-conflict', 'ed-alert', 'ed-alert-text', 'ed-leave', 'ed-home', 'about'];
// What the editor view had before it filled the view, and no longer has (no Close, Export, Full
// screen or browser button, no notes line, no Details).
const GONE = ['ed-close', 'ed-export', 'ed-full', 'ed-browser', 'ed-note', 'ed-details', 'ed-meta', 'edit-here'];
// The view as an app-library server serves it (its one mode slot filled in).
const libScript = script.replace("'__OPD_MODE__'", "'library'");
// ...with an in-panel editor in its editor slot: the test's recording fake (`fakePanel`).
const panelScript = libScript.replace(PANEL_SLOT, 'const opdCreatePanel = __opdTestPanel;');

// `clock: true` gives the view a live, deterministic clock (`tick(ms)` runs the timers set for
// that delay; `runAllTimers()` runs every timer set so far). Without it timers never fire.
// Adapted from the R11 reviewer's timer-active probe harness.
// `media`: a stand-in for the browser's `(prefers-color-scheme: dark)` query (`{ matches }`;
// `media.change(dark)` reports a new OS appearance). Without it the view has no OS signal.
function osScheme(matches) {
  const media = { matches, listeners: [] };
  media.addEventListener = (type, fn) => { if (type === 'change') media.listeners.push(fn); };
  media.change = (dark) => { media.matches = dark; for (const fn of media.listeners) fn({ matches: dark }); };
  return media;
}
function mount({ library = false, panel = null, clock = false, media = null } = {}) {
  const timers = [];
  const setTimeout = clock ? (fn, ms) => { timers.push({ fn, ms }); return timers.length; } : () => 0;
  const tick = (ms) => {
    const due = timers.filter((t) => t.ms === ms);
    for (const t of due) timers.splice(timers.indexOf(t), 1);
    for (const t of due) t.fn();
  };
  const runAllTimers = () => { for (const t of timers.splice(0)) t.fn(); };
  const document = { activeElement: null };
  const els = Object.fromEntries(IDS.map((id) => [id, el('div', document)]));
  for (const id of ['back', 'menu', 'pick', 'new-form', 'rename-form', 'folder-help', 'result', 'canvas', 'reopen', 'force', 'close', 'export',
    'm-import', 'import-file', 'result-details', 'result-meta', 'result-detail', 'editor', 'ed-renderer', 'ed-alert', 'ed-leave', 'ed-home',
    'lib-bar', 'no-match']) {
    els[id].hidden = true;
  }
  const props = {};
  const rootAttrs = {};
  const documentElement = { style: { setProperty: (k, v) => { props[k] = v; }, removeProperty: (k) => { delete props[k]; } },
    setAttribute: (k, v) => { rootAttrs[k] = String(v); } };
  const sent = [];
  let listener = null;
  // Copy into this realm so deep equality is not confused by the VM context's prototypes.
  const parent = { postMessage: (m) => sent.push(JSON.parse(JSON.stringify(m))) };
  const forbidden = () => { throw new Error('storage or network access'); };
  const window = { parent, addEventListener: (n, fn) => { if (n === 'message') listener = fn; } };
  if (media) window.matchMedia = (q) => { assert.equal(q, '(prefers-color-scheme: dark)'); return media; };
  for (const k of ['localStorage', 'sessionStorage', 'indexedDB', 'fetch', 'XMLHttpRequest']) {
    Object.defineProperty(window, k, { get: forbidden });
  }
  Object.assign(document, {
    getElementById: (id) => els[id], createElement: (tag) => el(tag, document),
    createElementNS: (_ns, tag) => el(tag, document), documentElement,
  });
  Object.defineProperty(document, 'cookie', { get: forbidden, set: forbidden });
  const source = panel ? panelScript : library ? libScript : script;
  vm.runInNewContext(source, { window, document, setTimeout, JSON, String, RegExp, Array, Map, Promise, Object, Number, Math,
    __opdTestPanel: panel, opdStatusDisplay: panel ? opdStatusDisplay : undefined });
  const from = (m, source = parent) => listener({ source, data: { jsonrpc: '2.0', ...m } });
  const last = (method) => [...sent].reverse().find((m) => m.method === method);
  const count = (method) => sent.filter((m) => m.method === method).length;
  return { els, sent, from, last, count, parent, document, props, rootAttrs, timers, tick, runAllTimers };
}

const toolText = (obj) => ({ content: [{ type: 'text', text: JSON.stringify(obj) }] });

/// Initialize, then answer the inventory call (if any) with `inventory`.
async function initialized({ caps = {}, context = {}, inventory = LISTED } = {}) {
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
const onlyAllowed = (v) => {
  assert.ok(methods(v).every((m) => ALLOWED.has(m)), methods(v).join());
  for (const m of v.sent.filter((m) => m.method === 'tools/call')) assert.ok(TOOLS.has(m.params.name), m.params.name);
  assert.equal(v.count('ui/message'), 0, 'no message to the agent, ever');
};
// The view's direct actions (every tools/call except the inventory), in order.
const actions = (v) => v.sent.filter((m) => m.method === 'tools/call' && m.params.name !== INVENTORY).map((m) => m.params);
const lastAction = (v) => [...v.sent].reverse().find((m) => m.method === 'tools/call' && m.params.name !== INVENTORY);
// Answer a direct action: a result object, a refusal (isError text) or a JSON-RPC error.
async function answer(v, reply, call = lastAction(v)) {
  if (reply.error) v.from({ id: call.id, error: reply.error });
  else if (reply.refused !== undefined) v.from({ id: call.id, result: { isError: true, content: [{ type: 'text', text: reply.refused }] } });
  else v.from({ id: call.id, result: toolText(reply) });
  await flush();
}
async function ack(v, method = 'ui/open-link', reply = { result: {} }) {
  v.from({ id: v.last(method).id, ...reply });
  await flush();
}
const result = (obj, isError = false) => ({ method: 'ui/notifications/tool-result', params: { content: [{ type: 'text', text: typeof obj === 'string' ? obj : JSON.stringify(obj) }], isError } });
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
const NO_CONTEXT = "This view's request carries no Codex workspace context, so OpenPaperDesign can't read or change project files from here. Nothing was read or changed. (Missing Codex workspace context.)";

// ---- connection, inventory, rows ----

test('initialize claims only display modes; the menu waits for Codex; then the inventory is read', async () => {
  const v = mount();
  assert.equal(v.sent[0].method, 'ui/initialize');
  assert.deepEqual(v.sent[0].params.appCapabilities, { availableDisplayModes: ['inline', 'fullscreen'] });
  assert.ok(v.els['menu-button'].disabled, 'no action before Codex answers');
  v.from({ id: v.sent[0].id, result: { hostCapabilities: {} } });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize', 'ui/notifications/initialized', 'tools/call']);
  assert.deepEqual(v.last('tools/call').params, { name: INVENTORY, arguments: {} });
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
  assert.deepEqual(actions(v), [], 'listing runs no action');
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
  assert.deepEqual(actions(v), [], 'details run nothing');
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

// ---- direct actions: exact tool, arguments, order and the result shown ----

const CANVASES = {
  workspace: '/w/acme', design_present: true, complete: true,
  canvases: [
    { project: 'design/Alpha', path: '/w/acme/design/Alpha', bound: false, canvas: { id: 'c-alpha', name: 'Alpha', pages: 2, modified: 1 } },
    { project: 'design/Beta', path: '/w/acme/design/Beta', bound: true, canvas: { id: 'c-beta', name: 'Beta', pages: 1, modified: 2 } },
  ],
  skipped: [{ project: 'design/Escape', problem: 'a symbolic link (not followed)' }],
};
const EDITOR = (name, action, extra = {}) => ({ url: URL_OK, workspace: '/w/acme', project: 'design/' + name, project_path: '/w/acme/design/' + name, canvas: { id: 'c-' + name, name, pages: 1 }, ...(action ? { action } : {}), ...extra });
const fields = (v) => v.els['result-fields'].children.map((c) => c.textContent);
// What Details holds for a result, or null while Details is hidden.
const meta = (v) => (v.els['result-details'].hidden ? null : v.els['result-meta'].children.map((c) => c.textContent));

test('a one-folder project opens directly: one canvases action for exactly its folder, then its canvases', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'beta').open.onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_canvases', arguments: { root: '/w/beta' } }]);
  assert.equal(v.els.notice.textContent, 'Opening Acme site · beta…');
  assert.ok(rows(v).every((r) => r.open.disabled) && v.els['menu-button'].disabled, 'busy while it runs');
  row(v, 'Field notes').open.onclick();
  await flush();
  assert.equal(actions(v).length, 1, 'nothing runs twice');
  await answer(v, { ...CANVASES, workspace: '/w/beta' });
  assert.equal(v.els['result-title'].textContent, 'Canvases');
  assert.equal(v.document.activeElement, v.els['result-title']);
  assert.deepEqual(fields(v), ['Folder', 'beta']);
  assert.deepEqual(rows(v, 'result-list').map((r) => [r.name, r.hint]), [['Alpha', '2 pages'], ['Beta', '1 page · current']]);
  assert.equal(v.els['result-list'].children[2].textContent, "Skipped 1 folder that isn't a usable canvas.");
  onlyAllowed(v);
});

test('several folders are asked, never guessed; the chosen folder is the action\'s exact root', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'acme').open.onclick();
  await flush();
  assert.deepEqual(actions(v), [], 'ambiguity is asked first');
  assert.equal(v.els.pick.hidden, false);
  assert.equal(v.els['pick-title'].textContent, 'Which folder of Acme site · acme?');
  assert.equal(v.document.activeElement, v.els['pick-title']);
  const picks = rows(v, 'pick-list');
  assert.deepEqual(picks.map((p) => [p.name, p.hint]), [['acme', '/w/acme'], ['acme-docs', '/w/acme-docs']]);
  picks[1].open.onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_canvases', arguments: { root: '/w/acme-docs' } }]);
});

test('Escape or Back leaves the folder choice without running anything', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'acme').open.onclick();
  v.els.pick.onkeydown({ key: 'Escape', preventDefault() {} });
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.document.activeElement, v.els.title);
  row(v, 'Acme site', 'acme').open.onclick();
  v.els.back.onclick();
  assert.equal(v.els.home.hidden, false);
  assert.deepEqual(actions(v), []);
});

test('this chat\'s project asks for its own canvases with no folder argument', async () => {
  const v = await initialized();
  row(v, "This chat's project").open.onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_canvases', arguments: {} }]);
});

test('a project with no folder says so and runs nothing', async () => {
  const EMPTY = { ...NOTES, id: 'prj_fixture_empty', name: 'Empty', roots: [] };
  const v = await initialized({ inventory: { ...LISTED, projects: [EMPTY] } });
  row(v, 'Empty').open.onclick();
  assert.equal(v.els.notice.textContent, 'Empty has no folder in Codex.');
  assert.deepEqual(actions(v), []);
});

test('opening a listed canvas binds it directly, shows its returned identity and asks Codex to open its exact link', async () => {
  const v = await initialized();
  row(v, 'Field notes').open.onclick();
  await flush();
  await answer(v, CANVASES);
  rows(v, 'result-list')[0].open.onclick();
  await flush();
  assert.deepEqual(actions(v).at(-1), { name: 'openpaperdesign_ui_open', arguments: { root: '/w/acme', project: 'design/Alpha' } });
  await answer(v, EDITOR('Alpha', 'opened'));
  assert.equal(v.els['result-title'].textContent, 'Alpha');
  assert.equal(v.els['result-state'].textContent, 'Opening the canvas…');
  assert.deepEqual(fields(v), ['Canvas', 'Alpha (1 page)', 'Folder', 'acme']);
  assert.deepEqual(meta(v), ['Project', 'design/Alpha', 'Folder path', '/w/acme', 'Address', 'local, port 41234']);
  assert.ok(!JSON.stringify(texts(v.els.result)).includes(TOKEN), 'the token is never shown');
  assert.equal(v.count('ui/open-link'), 1, 'opened right away');
  assert.deepEqual(v.last('ui/open-link').params, { url: URL_OK });
  await ack(v);
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/, 'an acknowledgement is not a load');
  assert.equal(v.els.canvas.hidden, false, 'and can be opened again');
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.count('ui/open-link'), 2);
  onlyAllowed(v);
});

test('New canvas creates directly in the exact chosen folder, then opens the created canvas', async () => {
  const lone = await initialized({ inventory: { ...LISTED, projects: [] } });
  lone.els['m-new'].onclick();
  assert.equal(lone.els['where-field'].hidden, true, 'no destination choice without projects');
  const v = await initialized();
  v.els['menu-button'].onclick();
  v.els['m-new'].onclick();
  assert.equal(v.els['new-form'].hidden, false);
  assert.equal(v.document.activeElement, v.els.name);
  assert.deepEqual(v.els.where.children.map((o) => [o.value, o.textContent]),
    [['', "This chat's project"], ['0', 'Acme site · acme · acme'], ['1', 'Acme site · acme · acme-docs'], ['2', 'Acme site · beta'], ['3', 'Field notes']]);
  v.els.name.value = '';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  assert.equal(v.els.notice.textContent, 'Name the canvas first.');
  assert.deepEqual(actions(v), [], 'no name: nothing runs');
  v.els.name.value = '  Landing "page"\u0007 `x`  ';
  v.els.where.value = '1';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_create', arguments: { name: 'Landing page x', root: '/w/acme-docs' } }]);
  assert.equal(v.els.notice.textContent, 'Creating "Landing page x" in Acme site · acme · acme-docs…');
  await answer(v, EDITOR('Landing page x', 'created', { workspace: '/w/acme-docs', project: 'design/Landing page x' }));
  assert.equal(v.els['result-title'].textContent, 'Landing page x');
  assert.equal(v.els['result-state'].textContent, 'Created. Opening the canvas…');
  assert.deepEqual(fields(v), ['Canvas', 'Landing page x (1 page)', 'Folder', 'acme-docs']);
  assert.deepEqual(v.last('ui/open-link').params, { url: URL_OK });
  // This chat's project: no root.
  const w = await initialized();
  w.els['m-new'].onclick();
  w.els.name.value = 'Solo';
  w.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  assert.deepEqual(actions(w), [{ name: 'openpaperdesign_ui_create', arguments: { name: 'Solo' } }]);
  onlyAllowed(v);
});

test('Open current canvas reopens directly; a delivered status offers the same for its own folder', async () => {
  const v = await initialized();
  v.els['m-current'].onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_editor', arguments: {} }]);
  await answer(v, EDITOR('Beta', 'reopened'));
  assert.equal(v.els['result-title'].textContent, 'Beta');
  assert.equal(v.count('ui/open-link'), 1);
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', workspace_permission_mode: 'managed-workspace-write', bound_project: '/w/acme/design/Landing' }));
  assert.equal(v.els['result-state'].textContent, 'A canvas is bound in this chat.');
  assert.deepEqual(fields(v), ['Canvas', 'Landing', 'Folder', 'acme']);
  assert.ok(!texts(v.els.result).some((t) => /managed-workspace-write/.test(t)), 'no permission profile');
  assert.equal(v.els.reopen.hidden, false);
  v.els.reopen.onclick();
  await flush();
  assert.deepEqual(actions(v).at(-1), { name: 'openpaperdesign_ui_editor', arguments: { root: '/w/acme' } });
  v.from(result({ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: null }));
  assert.equal(v.els.reopen.hidden, true);
  assert.equal(v.els['result-state'].textContent, 'No canvas is bound in this chat yet.');
});

// ---- refusals, failures and late replies never look like success ----

test('a refusal without workspace context is plain, keeps the reason under Details and opens nothing', async () => {
  const v = await initialized();
  row(v, 'Acme site', 'beta').open.onclick();
  await flush();
  await answer(v, { refused: NO_CONTEXT });
  assert.equal(v.els['result-title'].textContent, "Can't open project files here");
  assert.equal(v.els['result-state'].className, 'bad');
  assert.match(v.els['result-state'].textContent, /nothing was opened or changed\. OpenPaperDesign can't open project files from here yet\.$/);
  assert.doesNotMatch(v.els['result-state'].textContent, /chat|agent|paste/i, 'no unverified recovery route is promised');
  assert.equal(v.els['result-detail'].textContent, NO_CONTEXT);
  assert.equal(v.els['result-details'].hidden, false);
  assert.equal(v.els.canvas.hidden, true);
  assert.equal(v.count('ui/open-link'), 0);
  assert.ok(!v.els['menu-button'].disabled, 'usable again');
  onlyAllowed(v);
});

test('a switch that needs confirmation asks first, then repeats the same action with force', async () => {
  const v = await initialized();
  v.els['m-new'].onclick();
  v.els.name.value = 'Second';
  v.els.where.value = '2';
  v.els['new-form'].onsubmit({ preventDefault() {} });
  await flush();
  await answer(v, { refused: 'The browser editor for the bound project was opened (open_editor). … then call again with force: true.' });
  assert.equal(v.els['result-title'].textContent, 'Switch canvases?');
  assert.equal(v.els.force.hidden, false);
  assert.equal(v.count('ui/open-link'), 0);
  v.els.force.onclick();
  await flush();
  assert.deepEqual(actions(v), [
    { name: 'openpaperdesign_ui_create', arguments: { name: 'Second', root: '/w/beta' } },
    { name: 'openpaperdesign_ui_create', arguments: { name: 'Second', root: '/w/beta', force: true } },
  ]);
  await answer(v, EDITOR('Second', 'created'));
  assert.equal(v.els['result-title'].textContent, 'Second');
  assert.equal(v.els.force.hidden, true);
});

test('errors, unreadable answers and invalid links never show success', async () => {
  for (const [reply, title, state] of [
    [{ error: { code: -32601, message: 'no' } }, 'Not done', /didn't run the request \(code -32601\)/],
    [{ refused: 'design/Landing already exists; bind it with bind_project' }, 'Not done', /already exists/],
  ]) {
    const v = await initialized();
    v.els['m-current'].onclick();
    await flush();
    await answer(v, reply);
    assert.equal(v.els['result-title'].textContent, title);
    assert.match(v.els['result-state'].textContent, state);
    assert.equal(v.els.canvas.hidden, true);
  }
  // An answer that isn't JSON, or isn't the expected shape.
  for (const text of ['not json', '"a string"']) {
    const v = await initialized();
    v.els['m-current'].onclick();
    await flush();
    v.from({ id: lastAction(v).id, result: { content: [{ type: 'text', text }] } });
    await flush();
    assert.equal(v.els['result-title'].textContent, 'Not done', text);
    assert.equal(v.els.canvas.hidden, true);
  }
  const shape = await initialized();
  row(shape, 'Field notes').open.onclick();
  await flush();
  await answer(shape, { workspace: '/w/notes' });
  assert.equal(shape.els['result-title'].textContent, 'Not done', 'canvases without a list');
  // An editor answer whose link isn't the exact loopback form.
  for (const bad of ['https://127.0.0.1:1/#token=' + TOKEN, 'http://127.0.0.1:1/x#token=' + TOKEN,
    'http://localhost:1/#token=' + TOKEN, 'http://127.0.0.1:1/#token=short', URL_OK + '&x=1', 'javascript:alert(1)']) {
    const v = await initialized();
    v.els['m-current'].onclick();
    await flush();
    await answer(v, { ...EDITOR('Alpha', 'reopened'), url: bad });
    assert.equal(v.els.canvas.hidden, true, bad);
    assert.match(v.els['result-state'].textContent, /won't be opened/);
    assert.equal(v.count('ui/open-link'), 0, bad);
  }
});

test('a late or reordered action reply never changes a newer view', async () => {
  // Back before the reply.
  const v = await initialized();
  row(v, 'Field notes').open.onclick();
  await flush();
  const call = lastAction(v);
  v.els.back.onclick();
  await answer(v, CANVASES, call);
  assert.equal(v.els.home.hidden, false, 'still home');
  assert.equal(v.els.result.hidden, true);
  assert.equal(v.els.live.textContent, 'Projects');
  // A delivered result before the reply.
  const w = await initialized();
  w.els['m-current'].onclick();
  await flush();
  const pending = lastAction(w);
  w.from(result(CANVASES));
  assert.ok(!w.els['menu-button'].disabled, 'the new view is not busy');
  await answer(w, EDITOR('Alpha', 'reopened'), pending);
  assert.equal(w.els['result-title'].textContent, 'Canvases', 'the late editor is not shown');
  assert.equal(w.count('ui/open-link'), 0, 'and its link is not opened');
});

test('a pending open of Alpha neither blocks Beta nor reports under it', async () => {
  const v = await initialized();
  v.from(result(EDITOR('Alpha')));
  v.els.canvas.onclick();
  await flush();
  const alpha = v.last('ui/open-link');
  v.from(result(EDITOR('Beta')));
  assert.equal(v.els.canvas.disabled, false, 'Beta\'s button is usable');
  v.from({ id: alpha.id, error: { code: -32000, message: 'x' } });
  await flush();
  assert.equal(v.els['canvas-state'].textContent, '', 'Alpha\'s failure is not shown under Beta');
  assert.equal(v.els['result-title'].textContent, 'Beta');
});

test('late open replies after cancellation or Back change nothing', async () => {
  const v = await initialized();
  v.from(result(EDITOR('Alpha')));
  v.els.canvas.onclick();
  await flush();
  v.from({ method: 'ui/notifications/tool-cancelled', params: { reason: 'user' } });
  await ack(v);
  assert.equal(v.els['canvas-state'].textContent, '');
  assert.equal(v.els.live.textContent, 'Cancelled');
  v.from(result(EDITOR('Alpha')));
  v.els.canvas.onclick();
  await flush();
  v.els.back.onclick();
  await ack(v);
  assert.equal(v.els.live.textContent, 'Projects');
  v.els.canvas.onclick();
  await flush();
  assert.equal(v.count('ui/open-link'), 2, 'no link after Back');
});

test('a late inventory reply after a result renders quietly and announces nothing', async () => {
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

test('a delivered home result revokes an earlier editor link', async () => {
  const v = await initialized();
  v.from(result(EDITOR('Alpha')));
  v.from(result({ surface: 'home', version: '0.1.0' }));
  assert.equal(v.els.home.hidden, false);
  v.els.canvas.onclick();
  v.els.reopen.onclick();
  await flush();
  assert.equal(v.count('ui/open-link'), 0);
  assert.deepEqual(actions(v), []);
});

test('a delivered editor result is shown but not opened until the person asks', async () => {
  const v = await initialized();
  v.from(result(EDITOR('Beta')));
  assert.equal(v.els['result-title'].textContent, 'Beta');
  assert.equal(v.count('ui/open-link'), 0);
  v.els.canvas.onclick();
  assert.ok(v.els.canvas.disabled);
  await flush();
  assert.deepEqual(v.last('ui/open-link').params, { url: URL_OK });
});

// ---- the folder picker ----

test('Open a folder uses the host picker; a chosen folder is only the next action\'s exact root', async () => {
  const v = await initialized();
  v.els['m-folder'].onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_choose_folder', arguments: {} }]);
  await answer(v, { status: 'selected', folder: { path: '/w/picked', name: 'picked' }, access: 'none' });
  assert.deepEqual(actions(v).at(-1), { name: 'openpaperdesign_ui_canvases', arguments: { root: '/w/picked' } });
  assert.equal(v.els.notice.textContent, 'Opening picked…');
  await answer(v, { refused: NO_CONTEXT });
  assert.equal(v.els['result-title'].textContent, "Can't open project files here", 'a pick alone grants nothing');
  onlyAllowed(v);
});

test('picker answers other than a folder run nothing further', async () => {
  for (const [reply, want] of [
    [{ status: 'declined' }, 'No folder was chosen.'],
    [{ status: 'cancelled' }, 'No folder was chosen.'],
    [{ status: 'timeout' }, 'The folder picker closed without a choice.'],
    [{ status: 'opaque' }, "Codex returned a folder OpenPaperDesign can't use here. Nothing was opened."],
    [{ status: 'unavailable', code: -32601 }, "Codex couldn't show its folder picker (code -32601)."],
    [{ status: 'selected', folder: { name: 'no path' } }, 'No folder was chosen.'],
  ]) {
    const v = await initialized();
    v.els['m-folder'].onclick();
    await flush();
    await answer(v, reply);
    assert.equal(v.els.notice.textContent, want, JSON.stringify(reply));
    assert.equal(actions(v).length, 1, JSON.stringify(reply));
  }
  const v = await initialized();
  v.els['m-folder'].onclick();
  await flush();
  await answer(v, { status: 'unsupported', reason: 'x' });
  assert.equal(v.els['folder-help'].hidden, false, 'without a picker: the add-in-Codex hint');
  assert.equal(v.document.activeElement, v.els['folder-title']);
  v.els['folder-refresh'].onclick();
  await flush();
  assert.equal(v.els.home.hidden, false);
  assert.deepEqual(v.last('tools/call').params, { name: INVENTORY, arguments: {} });
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
  assert.deepEqual(actions(v), [], 'the menu itself runs nothing');
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
  // The colours were the dark theme's: they never paint the light one (no stale host palette);
  // the font is kept across the partial update.
  assert.deepEqual(v.props, { '--font-sans': 'Inter, system-ui' }, 'partial update keeps the earlier font, not dark colours');
  const before = { ...v.props };
  v.from({ method: 'ui/notifications/host-context-changed', params: { styles: { css: { fonts: '@import url(https://fonts.invalid/x.css);' } } } });
  assert.deepEqual(v.props, before, 'font-loading CSS is never applied');
  v.from({ method: 'ui/notifications/host-context-changed', params: { theme: 'dark' } }, { postMessage() {} });
  assert.equal(v.rootAttrs['data-theme'], 'light', 'another window is ignored');
  assert.equal(v.count('tools/call'), 1, 'host context grants nothing and triggers no call');
});

// ---- delivered model results ----

test('a delivered canvases result lists canvases; bound means current; opening one is the direct action', async () => {
  const v = await initialized();
  v.from(result(CANVASES));
  assert.equal(v.els['result-title'].textContent, 'Canvases');
  assert.deepEqual(fields(v), ['Folder', 'acme'], 'concise identity in view');
  assert.deepEqual(meta(v), ['Folder path', '/w/acme'], 'the exact folder under Details');
  assert.ok(!texts(v.els.result).some((t) => /open now/.test(t)), 'binding is not a loaded editor');
  rows(v, 'result-list')[1].open.onclick();
  await flush();
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_open', arguments: { root: '/w/acme', project: 'design/Beta' } }]);
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

test('a refused or retired status shows plain recovery and no folder', async () => {
  const v = await initialized();
  v.from(result({ workspace_state: 'retired', workspace_usable: false, workspace: '/w/old', workspace_problem: 'Codex workspace changed', bound_project: null }));
  assert.equal(v.els['result-state'].className, 'bad');
  assert.match(v.els['result-state'].textContent, /^This chat can't use a project folder\./);
  assert.deepEqual(fields(v), []);
  assert.equal(v.els['result-detail'].textContent, 'Codex workspace changed');
  assert.ok(!texts(v.els.result).includes('/w/old'));
});

test('a failed step reports only that step; it never claims earlier work was undone', async () => {
  const v = await initialized();
  v.from(result('The browser editor is not built in.\nmore', true));
  const text = v.els['result-state'].textContent;
  assert.doesNotMatch(text, /Nothing was changed|nothing changed|rolled back|undone/i);
  assert.match(text, /last step didn't work\. Anything it finished before that may already have happened; check the chat/);
  assert.equal(v.els['result-detail'].textContent, 'The browser editor is not built in.');
});

test('messages from other windows are ignored', async () => {
  const v = mount();
  v.from({ id: v.sent[0].id, result: { hostCapabilities: {} } }, { postMessage() {} });
  await flush();
  assert.deepEqual(methods(v), ['ui/initialize']);
  v.from(result({ url: URL_OK }), { postMessage() {} });
  assert.equal(v.els.canvas.hidden, true);
});

// ---- R2 protections, kept ----

const submitNew = async (v) => { v.els['new-form'].onsubmit({ preventDefault() {} }); await flush(); };
const options = (v) => v.els.where.children.map((o) => o.textContent);

test('a refresh while New canvas is open never changes what a shown destination sends (R2-F1)', async () => {
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
    v.els.name.value = 'Draft';
    const shown = v.els.where.children.find((o) => o.value === '2').textContent;
    assert.equal(shown, 'Acme site · beta', kind);
    v.from({ id: pending.id, result: toolText({ ...LISTED, projects: next }) });
    await flush();
    assert.equal(v.els['new-form'].hidden, false, 'the open form stays as it was');
    await submitNew(v);
    assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_create', arguments: { name: 'Draft', root: '/w/beta' } }], kind);
    await answer(v, EDITOR('Draft', 'created'));
    v.els['m-new'].onclick();
    assert.deepEqual(options(v), nextOptions, 'the next form uses the refreshed list');
  }
});

test('a late inventory reply leaves the visible canvas rows to their own action (R2-F2)', async () => {
  for (const [kind, reply, title] of [
    ['succeeded', EDITOR('Alpha', 'opened'), 'Alpha'],
    ['refused', { refused: NO_CONTEXT }, "Can't open project files here"],
    ['failed', { error: { code: -32601, message: 'no' } }, 'Not done'],
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
    assert.ok(rows(v).every((r) => r.open.disabled), 'the hidden home\'s new rows wait for the same action');
    assert.ok(list.every((r) => r.open.disabled), 'the visible rows are still owned by the pending action');
    assert.ok(list.every((r) => !r.more.disabled), 'details only show, so they stay usable');
    await answer(v, reply);
    assert.equal(v.els['result-title'].textContent, title, kind);
    assert.ok(!v.els['menu-button'].disabled, kind);
    assert.ok(rows(v).every((r) => !r.open.disabled), kind);
    assert.equal(actions(v).length, 1, 'nothing was resent');
  }
});

test('destinations with equal folder names show the shortest distinguishing path (R2-F4)', async () => {
  const SITE = { ...BETA, id: 'prj_fixture_site', name: 'Site', roots: [{ path: '/work/current/site' }, { path: '/archive/site' }] };
  const v = await initialized({ inventory: { ...LISTED, projects: [SITE] } });
  assert.deepEqual(rows(v).map((r) => [r.name, r.hint]), [["This chat's project", ''], ['Site', '2 folders']], 'the list stays quiet');
  v.els['m-new'].onclick();
  assert.deepEqual(v.els.where.children.slice(1).map((o) => [o.textContent, o.title]),
    [['Site · current/site', '/work/current/site'], ['Site · archive/site', '/archive/site']]);
  v.els.where.value = '1';
  v.els.name.value = 'Draft';
  await submitNew(v);
  assert.deepEqual(actions(v), [{ name: 'openpaperdesign_ui_create', arguments: { name: 'Draft', root: '/archive/site' } }]);
  v.els.back.onclick();
  row(v, 'Site', '2 folders').open.onclick();
  assert.deepEqual(rows(v, 'pick-list').map((r) => [r.name, r.hint]),
    [['current/site', '/work/current/site'], ['archive/site', '/archive/site']], 'the folder question matches');
});

test('results keep exact paths and the port under Details, cleared for each new result (R2-F5)', async () => {
  const v = await initialized();
  const inView = () => texts(v.els['result-fields']).concat(texts(v.els['result-state']), texts(v.els['result-list'])).join('\n');
  for (const r of [{ workspace_state: 'pinned', workspace_usable: true, workspace: '/w/acme', bound_project: '/w/acme/design/Alpha' }, CANVASES, EDITOR('Alpha')]) {
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

test('the view sends no chat message, calls only its own tools, stores nothing and embeds its one font locally', () => {
  assert.ok(!script.includes('ui/message'), 'no message to the agent');
  assert.ok(!/id="prompt"|Paste this/.test(html), 'no paste prompt');
  // Tool names at call sites: act('…', …) and the inventory's tools/call.
  const named = new Set([...script.matchAll(/(?:act\(|name: )'(openpaperdesign_[a-z_]+)'/g)].map((m) => m[1]));
  assert.deepEqual([...named].sort(), [...TOOLS, ...LIB_TOOLS].sort(), 'exactly these server tools');
  // The appearance call: one call site, sending only the validated preference.
  assert.deepEqual([...script.matchAll(/name: 'openpaperdesign_app_appearance', arguments: \{ ([^}]*) \}/g)].map((m) => m[1].trim()),
    ['appearance'], 'one call site, sending only the preference');
  for (const banned of ['resources/read', 'localStorage', 'sessionStorage', 'indexedDB', 'document.cookie', 'fetch(', 'XMLHttpRequest', 'innerHTML',
    'openpaperdesign_status\'', 'bind_project\'', 'create_project\'', 'open_editor\'', '@import']) {
    assert.ok(!html.includes(banned), banned);
  }
  const style = html.slice(0, html.indexOf('</style>'));
  assert.deepEqual(style.match(/url\([^)]*\)/g), ['url("data:font/woff2;base64,__OPD_WORDMARK_FONT__")'], 'the only url() is the served font slot');
  assert.ok(!/https?:\/\/(?!127\.0\.0\.1)/.test(style), 'no external stylesheet or font');
  assert.match(style, /\.wordmark \{ font: 400 16px\/1\.2 "Geist Pixel", monospace;/);
  // The wordmark, and the notices behind an unobtrusive About.
  assert.match(html, /<footer><span class="wordmark">onlinesourdough<\/span><details id="about" class="about"><summary>About<\/summary>\s*<p>OpenPaperDesign: a fork of paper-rs \(MIT OR Apache-2\.0\)\. Inter font: SIL OFL 1\.1\. Not affiliated with Paper\.<\/p><\/details><\/footer>/);
  assert.match(html, /aria-label="Back to projects"/);
  assert.equal(html.split('__OPD_MODE__').length, 2, 'exactly one mode slot');
  // The editor fills the view: no fixed-height stage, frame, header, footer or button row.
  assert.ok(!style.includes('calc(100vh - 210px)'), 'no boxed stage');
  assert.match(style, /\.ed-stage \{ position: fixed; inset: 0; overflow: hidden; \}/);
  assert.match(style, /main\[data-view="editor"\] > \.notice, main\[data-view="editor"\] > footer \{ display: none; \}/);
  for (const id of GONE) assert.ok(!html.includes(`id="${id}"`), id);
  for (const text of ['Edit in this view', 'Open in browser', 'Full screen', 'Close canvas</button>']) {
    assert.ok(!html.slice(html.indexOf('<section id="editor"')).includes(text), text);
  }
  assert.ok(!script.includes('ui/request-display-mode'), 'no display mode request');
});

// ---- served by an app-library server (--app-library) ----

const ID1 = '0123456789abcdef';
const ID2 = 'fedcba9876543210';
// Saved times as the server sends them (milliseconds), local to this computer's clock.
const MAR_4_2025 = new Date(2025, 2, 4, 12).getTime();
const LIBRARY_LIST = { storage: 'app-library', complete: true, skipped: [], canvases: [
  { id: ID1, name: 'Landing', pages: 2, modified: MAR_4_2025, open: false },
  { id: ID2, name: 'Notes', pages: 1, modified: MAR_4_2025 - 86400000, open: true },
  { id: '../escape', name: 'Not an id', pages: 1 }] };
// Each editor result names its binding; the view sends it back with close, export and polls.
const B1 = `${ID1}.1`;
const B2 = `${ID2}.2`;
const LIB_EDITOR = (action, id = ID1, extra = {}) => ({ action, storage: 'app-library', url: URL_OK, browser_editor: 'built in',
  binding: id === ID1 ? B1 : B2, canvas: { id, name: id === ID1 ? 'Landing' : 'Notes', pages: 1 },
  location: `/data/library/v1/canvases/${id}`, editor_loaded: false, browser_saves: 0, ...extra });
const SWITCH_REFUSAL = (shown) => 'The browser editor for the bound project was opened ... then call again with force: true.'
  + ` (open canvas: ${shown})`;

// A library view as a build without the in-panel editor module serves it: library canvases go
// to their one loopback web editor link (the fallback). `{ panel: true }`: a complete build,
// with the in-panel editor (a recording stand-in, `fakePanel`, as `v.fake`), where New and Open
// mount it right here instead.
async function libraryView(list = LIBRARY_LIST, { panel = false, clock = false } = {}) {
  const fake = panel ? fakePanel() : null;
  const v = mount({ library: true, panel: fake, clock });
  v.fake = fake;
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: {} } });
  await flush();
  const call = v.last('tools/call');
  assert.deepEqual(call.params, { name: LIB_LIST, arguments: {} }, 'the library list, not the Codex inventory');
  v.from(list.error ? { id: call.id, error: list.error } : { id: call.id, result: toolText(list) });
  await flush();
  return v;
}
const libOnly = (v) => {
  assert.ok(methods(v).every((m) => ALLOWED.has(m)), methods(v).join());
  for (const m of v.sent.filter((m) => m.method === 'tools/call')) assert.ok(LIB_TOOLS.has(m.params.name), m.params.name);
  assert.equal(v.count('ui/message'), 0, 'no message to the agent, ever');
};
// The latest library action (every tools/call except the list).
const libAction = (v) => [...v.sent].reverse().find((m) => m.method === 'tools/call' && m.params.name !== LIB_LIST);
const libCalls = (v, name) => v.sent.filter((m) => m.method === 'tools/call' && m.params.name === name).length;

test('library (no module): the app\'s files are listed; Open hands the canvas to its one web editor link, with no message', async () => {
  assert.notEqual(libScript, script, 'the mode slot is filled');
  const v = await libraryView();
  assert.deepEqual(rows(v).map((r) => [r.name, r.hint]), [['Landing', '2 pages'], ['Notes', '1 page · open here']],
    'server ids only; no "This chat\'s project" row');
  assert.equal(v.els['m-folder'].hidden, true, 'no folder picker');
  assert.equal(v.els['m-import'].hidden, false);
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1 } },
    'the primary action asks for nothing but the canvas');
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  assert.equal(v.count('ui/open-link'), 1, 'exactly one opening request');
  assert.equal(v.last('ui/open-link').params.url, URL_OK, 'the result\'s own same-binding loopback link');
  assert.equal(v.els['result-state'].textContent, 'Opening the editor…');
  assert.ok(!v.els.close.hidden && !v.els.export.hidden && !v.els.canvas.hidden);
  const shown = ['result-fields', 'result-meta', 'result-state', 'notice', 'live'].flatMap((id) => texts(v.els[id])).join('\n');
  assert.ok(shown.includes(ID1) && !shown.includes(TOKEN), 'the canvas id under Details, never the token');
  await ack(v);
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/, 'an acknowledgement is not a load');
  assert.equal(libCalls(v, 'openpaperdesign_app_editor'), 0, 'no load is claimed without the editor\'s own report');
  libOnly(v);
});

test('library: an answer for another canvas, a link that isn\'t loopback, or a reply after Back opens nothing', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened', ID2), libAction(v));
  assert.equal(v.count('ui/open-link'), 0);
  assert.match(v.els['result-state'].textContent, /another canvas/);
  v.els.back.onclick();
  await flush();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened', ID1, { url: 'https://example.com/#token=' + TOKEN }), libAction(v));
  assert.equal(v.count('ui/open-link'), 0);
  assert.match(v.els['result-state'].textContent, /isn't a local OpenPaperDesign link/);
  v.els.back.onclick();
  await flush();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  const pending = libAction(v);
  v.els.back.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), pending);
  assert.equal(v.count('ui/open-link'), 0, 'a late reply belongs to the view it started in');
  // A delivered result is shown, never opened by itself.
  v.from(result(LIB_EDITOR('opened')));
  assert.equal(v.count('ui/open-link'), 0);
  libOnly(v);
});

test('library (no module): New file creates at once, with no form; an unanswered create is never retried', async () => {
  const v = await libraryView();
  assert.equal(v.els['m-new'].hidden, true, 'no named-form entry in the library menu: New file is the button');
  v.els['new-file'].onclick();
  await flush();
  assert.equal(v.els['new-form'].hidden, true, 'no form first');
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_create', arguments: { name: 'Untitled' } });
  await answer(v, LIB_EDITOR('created'), libAction(v));
  assert.equal(v.els['result-state'].textContent, 'Created and saved in your library. Opening the editor…');
  assert.equal(v.count('ui/open-link'), 1);
  assert.equal(v.last('ui/open-link').params.url, URL_OK);
  const w = await libraryView();
  w.els['new-file'].onclick();
  await flush();
  await answer(w, { error: { code: -32001, message: 'lost' } }, libAction(w));
  assert.equal(w.els['result-title'].textContent, 'Not sure it was created');
  assert.match(w.els['result-state'].textContent, /check your files before trying again/);
  assert.equal(libCalls(w, 'openpaperdesign_app_create'), 1, 'not retried');
  assert.equal(w.last('tools/call').params.name, LIB_LIST, 'the list is read again instead');
  libOnly(v);
  libOnly(w);
});

test('library: Close names this view\'s binding, asks the person first when the server does, and claims no save', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  v.els.close.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } },
    'close names the binding this view showed');
  await answer(v, { refused: 'The editor of this canvas was opened. ... then close again with force: true.' }, libAction(v));
  assert.equal(v.els['result-title'].textContent, 'Close the canvas?');
  assert.equal(v.els.force.textContent, 'Close anyway');
  v.els.force.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1, force: true } },
    'the confirmation is for the same binding');
  await answer(v, { action: 'closed', canvas: { id: ID1 } }, libAction(v));
  assert.equal(v.els['result-title'].textContent, 'Canvas closed');
  assert.match(v.els['result-state'].textContent, /not saved by closing/);
  assert.equal(v.last('tools/call').params.name, LIB_LIST);
  libOnly(v);
});

test('library: export and import move only bytes, and say plainly what this view could not do', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  v.els.export.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_export', arguments: { binding: B1 } });
  await answer(v, { action: 'exported', binding: B1, canvas: { id: ID1, name: 'Landing' }, content: 'canvas-only',
    file_name: 'Landing.opdcanvas.json', bytes: 38, payload: '{"format":"openpaperdesign-canvas"}' }, libAction(v));
  assert.match(v.els.notice.textContent, /can't download files here, so no file was saved/, 'no claimed download');
  v.els['m-import'].onclick();
  assert.match(v.els.notice.textContent, /can't open files here\. Nothing was imported/);
  let picked = 0;
  v.els['import-file'].click = () => { picked += 1; };
  v.els['m-import'].onclick();
  assert.equal(picked, 1);
  v.els['import-file'].files = [{ size: 33 * 1024 * 1024, name: 'big.json', text: async () => 'x' }];
  await v.els['import-file'].onchange();
  assert.match(v.els.notice.textContent, /larger than 32 MB/);
  v.els['import-file'].files = [{ size: 9, name: '/Users/someone/secret.json', text: async () => 'FILE-TEXT' }];
  await v.els['import-file'].onchange();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_import', arguments: { payload: 'FILE-TEXT' } },
    'only the text, never the file name or a path');
  await answer(v, LIB_EDITOR('imported', ID2), libAction(v));
  assert.equal(v.els['result-state'].textContent, 'Imported as a new canvas in your library. Opening the editor…');
  assert.equal(v.count('ui/open-link'), 2, 'one opening request per opened canvas');
  libOnly(v);
});

test('library: without a built editor nothing is opened, and the canvas is still reported saved', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened', ID1, { browser_editor: 'not built' }), libAction(v));
  assert.equal(v.count('ui/open-link'), 0);
  assert.equal(v.els.canvas.hidden, true);
  assert.match(v.els['canvas-state'].textContent, /built without the browser editor/);
});

test('library: a switch confirmation names exactly the binding the refusal showed', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1 } }, 'no force at first');
  await answer(v, { refused: SWITCH_REFUSAL(B2) }, libAction(v));
  assert.equal(v.els['result-title'].textContent, 'Switch canvases?');
  assert.equal(v.els.force.hidden, false);
  v.els.force.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1, force: true, replace: B2 } });
  // The server finds another view changed the open canvas meanwhile: refused, shown as such.
  await answer(v, { refused: 'The canvas this view shows isn\'t the one open in this OpenPaperDesign window any more: another view changed it. Nothing was done, and the open canvas is unchanged.' }, libAction(v));
  assert.equal(v.count('ui/open-link'), 0);
  assert.match(v.els['result-state'].textContent, /another view changed it/);
  // New file the same way.
  v.els.back.onclick();
  await flush();
  v.els['new-file'].onclick();
  await flush();
  await answer(v, { refused: SWITCH_REFUSAL(B1) }, libAction(v));
  v.els.force.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_create', arguments: { name: 'Untitled', force: true, replace: B1 } });
  libOnly(v);
});

test('library: an export answer for another binding is never offered as a download', async () => {
  const v = await libraryView();
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  v.els.export.onclick();
  await flush();
  await answer(v, { action: 'exported', binding: B2, canvas: { id: ID2, name: 'Notes' }, content: 'canvas-only',
    file_name: 'Notes.opdcanvas.json', bytes: 38, payload: '{"format":"openpaperdesign-canvas"}' }, libAction(v));
  assert.match(v.els['result-state'].textContent, /another canvas than the one shown here, so nothing was downloaded/);
  assert.doesNotMatch(v.els.notice.textContent, /download/);
  libOnly(v);
});

test('library: a file read that outlives its view or its selection sends nothing and says nothing', async () => {
  const v = await libraryView();
  const deferred = () => {
    let resolve;
    let reject;
    const p = new Promise((res, rej) => { resolve = res; reject = rej; });
    return { p, resolve, reject };
  };
  const choose = (read) => {
    v.els['import-file'].files = [{ size: 9, name: 'canvas.json', text: () => read }];
    return v.els['import-file'].onchange();
  };
  const imports = () => libCalls(v, 'openpaperdesign_app_import');
  // Back while the read is pending: the late text is dropped.
  const a = deferred();
  const readA = choose(a.p);
  v.els.back.onclick();
  await flush();
  a.resolve('LATE-TEXT');
  await readA;
  await flush();
  assert.equal(imports(), 0, 'no import after Back');
  // A late read error after Back leaves the newer view's notice alone.
  const b = deferred();
  const readB = choose(b.p);
  v.els.back.onclick();
  await flush();
  b.reject(new Error('the file went away'));
  await readB;
  await flush();
  assert.doesNotMatch(v.els.notice.textContent, /couldn't read/);
  assert.equal(imports(), 0);
  // A newer selection in the same view wins; the older read, finishing later, sends nothing.
  const c = deferred();
  const readC = choose(c.p);
  await choose(Promise.resolve('NEW-TEXT'));
  await flush();
  assert.equal(imports(), 1);
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_import', arguments: { payload: 'NEW-TEXT' } });
  c.resolve('OLD-TEXT');
  await readC;
  await flush();
  assert.equal(imports(), 1, 'the superseded read sends nothing');
  // A read error for the current selection is still reported.
  const w = await libraryView();
  w.els['import-file'].files = [{ size: 9, name: 'canvas.json', text: () => Promise.reject(new Error('unreadable')) }];
  await w.els['import-file'].onchange();
  assert.match(w.els.notice.textContent, /couldn't read that file\. Nothing was imported/);
  libOnly(v);
  libOnly(w);
});

test('the workspace view offers no library import', async () => {
  const v = await initialized();
  assert.equal(v.els['m-import'].hidden, true);
  assert.equal(v.els['m-folder'].hidden, false);
  assert.ok(v.sent.every((m) => m.method !== 'tools/call' || !LIB_TOOLS.has(m.params.name)));
  assert.equal(v.els['lib-bar'].hidden, true, 'no file library bar in the workspace view');
  assert.equal(v.els['m-new'].hidden, false, 'its New menu keeps New canvas…');
  assert.equal(v.els['menu-icon'].getAttribute('hidden'), null);
});

// ---- the library home: files, search, layout, a file's menu ----

// A file row's parts: [name, hint, last edit] and its menu (Open, Rename…, Export…).
const fileParts = (r) => r.open.children[1].children.map((c) => c.textContent);
const fileMenuOf = (r) => r.li.children[1];

test('library home: the app\'s own context is said plainly, with no project; New file, search and layout are there', async () => {
  const v = await libraryView();
  assert.equal(v.els['lib-bar'].hidden, false);
  assert.equal(v.els['h-projects'].textContent, 'Files');
  // The markup's line, unchanged by the script: the library's files belong to no Codex project.
  assert.match(html, /id="lib-context" class="lib-context">OpenPaperDesign library: files this app keeps itself, not part of a Codex project\.</);
  assert.equal(v.els['menu-label'].textContent, 'More');
  assert.equal(v.els['menu-icon'].getAttribute('hidden'), '');
  assert.deepEqual(['m-new', 'm-current', 'm-folder', 'm-import', 'm-refresh'].filter((id) => !v.els[id].hidden), ['m-current', 'm-import', 'm-refresh']);
  assert.equal(v.els['m-current'].textContent, 'Open current file');
  // No project grouping or folder: no project words or paths in the list.
  const listed = texts(v.els.projects).join('\n');
  assert.doesNotMatch(listed, /project|\/data\/|Folder/i);
  assert.equal(v.els.projects.className, 'files');
  assert.equal(v.els['new-file'].disabled, false);
  libOnly(v);
});

test('library home: each file shows its name, pages and last edit; old ones by date', async () => {
  const now = Date.now();
  const v = await libraryView({ ...LIBRARY_LIST, canvases: [
    { id: ID1, name: 'Fresh', pages: 1, modified: now - 10 * 1000 },
    { id: ID2, name: 'Earlier', pages: 3, modified: now - 5 * 60 * 1000 },
    { id: 'aaaaaaaaaaaaaaaa', name: 'Morning', pages: 1, modified: now - 3 * 3600 * 1000 },
    { id: 'bbbbbbbbbbbbbbbb', name: 'Yesterday', pages: 1, modified: now - 26 * 3600 * 1000 },
    { id: 'cccccccccccccccc', name: 'Old', pages: 1, modified: MAR_4_2025 },
    { id: 'dddddddddddddddd', name: 'Unknown', pages: 1 },
  ] });
  assert.deepEqual(rows(v).map(fileParts), [
    ['Fresh', '1 page', 'Edited just now'],
    ['Earlier', '3 pages', 'Edited 5 min ago'],
    ['Morning', '1 page', 'Edited 3 h ago'],
    ['Yesterday', '1 page', 'Edited 1 day ago'],
    ['Old', '1 page', 'Edited 2025-03-04'],
    ['Unknown', '1 page'],
  ]);
  for (const r of rows(v)) assert.equal(r.open.children[0].tag, 'svg', 'a generic file icon');
});

test('library home: search narrows by name, says when nothing matches, and Escape clears it', async () => {
  const v = await libraryView();
  const shown = () => rows(v).filter((r) => !r.li.hidden).map((r) => r.name);
  v.els.search.value = 'LAND';
  v.els.search.oninput();
  assert.deepEqual(shown(), ['Landing']);
  assert.equal(v.els['no-match'].hidden, true);
  v.els.search.value = 'zzz';
  v.els.search.oninput();
  assert.deepEqual(shown(), []);
  assert.equal(v.els['no-match'].hidden, false);
  let prevented = false;
  v.els.search.onkeydown({ key: 'Escape', preventDefault() { prevented = true; } });
  assert.ok(prevented);
  assert.equal(v.els.search.value, '');
  assert.deepEqual(shown(), ['Landing', 'Notes']);
  assert.equal(v.els['no-match'].hidden, true);
  // A refreshed list keeps the search.
  v.els.search.value = 'not';
  v.els.search.oninput();
  v.els['m-refresh'].onclick();
  await flush();
  await answer(v, LIBRARY_LIST, v.last('tools/call'));
  assert.deepEqual(shown(), ['Notes']);
  assert.deepEqual(actions(v).filter((a) => a.name !== LIB_LIST), [], 'searching runs nothing');
});

test('library home: list and grid are a pressed toggle; nothing is stored', async () => {
  const v = await libraryView();
  assert.equal(v.els['layout-list'].getAttribute('aria-pressed'), 'true');
  v.els['layout-grid'].onclick();
  assert.equal(v.els.projects.className, 'files grid');
  assert.equal(v.els['layout-grid'].getAttribute('aria-pressed'), 'true');
  assert.equal(v.els['layout-list'].getAttribute('aria-pressed'), 'false');
  // The grid shows the same files and keeps working.
  assert.deepEqual(rows(v).map((r) => r.name), ['Landing', 'Notes']);
  v.els['layout-list'].onclick();
  assert.equal(v.els.projects.className, 'files');
  libOnly(v);
});

test('library home: a file\'s menu opens on its labelled button, moves with arrows and closes on Escape with focus back', async () => {
  const v = await libraryView();
  // Everything the view sent before the menu was touched (its initialize and the list read).
  const baseline = v.sent.length;
  const r = row(v, 'Landing', '2 pages');
  const menu = fileMenuOf(r);
  assert.equal(r.more.getAttribute('aria-label'), 'More for Landing');
  assert.equal(r.more.getAttribute('aria-haspopup'), 'menu');
  assert.equal(r.more.getAttribute('aria-controls'), menu.id);
  assert.equal(menu.getAttribute('role'), 'menu');
  assert.equal(menu.hidden, true);
  r.more.onclick();
  assert.equal(menu.hidden, false);
  assert.equal(r.more.getAttribute('aria-expanded'), 'true');
  const items = menu.children;
  assert.deepEqual(items.map((b) => [b.textContent, b.getAttribute('role')]),
    [['Open', 'menuitem'], ['Rename…', 'menuitem'], ['Export…', 'menuitem']], 'no "Open in new chat"');
  assert.equal(v.document.activeElement, items[0]);
  const key = (k) => menu.onkeydown({ key: k, preventDefault() {} });
  key('ArrowDown');
  assert.equal(v.document.activeElement, items[1]);
  key('End');
  assert.equal(v.document.activeElement, items[2]);
  key('ArrowDown');
  assert.equal(v.document.activeElement, items[0], 'wraps around');
  key('Escape');
  assert.equal(menu.hidden, true);
  assert.equal(r.more.getAttribute('aria-expanded'), 'false');
  assert.equal(v.document.activeElement, r.more);
  // One menu at a time.
  r.more.onclick();
  const other = row(v, 'Notes', '1 page · open here');
  other.more.onclick();
  assert.equal(menu.hidden, true);
  assert.equal(fileMenuOf(other).hidden, false);
  assert.deepEqual(v.sent.slice(baseline), [], 'the menu itself sends nothing: no action, no list read');
  assert.deepEqual(actions(v), [{ name: LIB_LIST, arguments: {} }], 'only the initial list read');
});

test('library home: the menu\'s Open is the row\'s Open', async () => {
  const v = await libraryView();
  const r = row(v, 'Landing', '2 pages');
  r.more.onclick();
  fileMenuOf(r).children[0].onclick();
  await flush();
  assert.equal(fileMenuOf(r).hidden, true);
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1 } });
  libOnly(v);
});

test('library home: Rename changes the name only, by id; the list is read again', async () => {
  const v = await libraryView();
  const baseline = v.sent.length; // initialize and the list read
  const r = row(v, 'Landing', '2 pages');
  r.more.onclick();
  fileMenuOf(r).children[1].onclick();
  assert.equal(v.els['rename-form'].hidden, false);
  assert.equal(v.els['rename-title'].textContent, 'Rename "Landing"');
  assert.equal(v.els['rename-name'].value, 'Landing');
  assert.equal(v.document.activeElement, v.els['rename-name']);
  assert.deepEqual(v.sent.slice(baseline), [], 'opening the form sends nothing');
  assert.equal(libCalls(v, 'openpaperdesign_app_rename'), 0, 'no rename before it is confirmed');
  v.els['rename-name'].value = '  Launch "plan"\u0007 ';
  v.els['rename-form'].onsubmit({ preventDefault() {} });
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_rename', arguments: { id: ID1, name: 'Launch plan' } });
  assert.ok(v.els['rename-save'].disabled, 'busy while it runs');
  await answer(v, { action: 'renamed', canvas: { id: ID1, name: 'Launch plan' } }, libAction(v));
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els.notice.textContent, 'Renamed to "Launch plan".');
  assert.equal(v.last('tools/call').params.name, LIB_LIST, 'the list shows the name it has now');
  libOnly(v);
});

test('library home: Rename refused, unconfirmed, emptied, unchanged or cancelled never shows a rename', async () => {
  const start = async () => {
    const v = await libraryView();
    const r = row(v, 'Landing', '2 pages');
    r.more.onclick();
    fileMenuOf(r).children[1].onclick();
    return v;
  };
  const renames = (v) => libCalls(v, 'openpaperdesign_app_rename');
  // Refused (open in another window): said plainly, nothing changed.
  const busy = await start();
  busy.els['rename-name'].value = 'Other';
  busy.els['rename-form'].onsubmit({ preventDefault() {} });
  await flush();
  await answer(busy, { refused: 'This canvas is open in another OpenPaperDesign window (another Codex chat or panel). Close it there first. Nothing was changed here.' }, libAction(busy));
  assert.equal(busy.els['result-title'].textContent, 'Not done');
  assert.match(busy.els['result-state'].textContent, /open in another OpenPaperDesign window/);
  // Unconfirmed: maybe renamed, never retried; the list is read.
  const lost = await start();
  lost.els['rename-name'].value = 'Other';
  lost.els['rename-form'].onsubmit({ preventDefault() {} });
  await flush();
  await answer(lost, { error: { code: -32001, message: 'lost' } }, libAction(lost));
  assert.equal(lost.els['result-title'].textContent, 'Not sure it was renamed');
  assert.equal(renames(lost), 1);
  assert.equal(lost.last('tools/call').params.name, LIB_LIST);
  // An answer for another file isn't a confirmation either.
  const other = await start();
  other.els['rename-name'].value = 'Other';
  other.els['rename-form'].onsubmit({ preventDefault() {} });
  await flush();
  await answer(other, { action: 'renamed', canvas: { id: ID2, name: 'Other' } }, libAction(other));
  assert.equal(other.els['result-title'].textContent, 'Not sure it was renamed');
  // Empty, unchanged, Cancel and Escape: nothing runs.
  const v = await start();
  v.els['rename-name'].value = '   ';
  v.els['rename-form'].onsubmit({ preventDefault() {} });
  assert.equal(v.els.notice.textContent, 'Name the file first.');
  v.els['rename-name'].value = 'Landing';
  v.els['rename-form'].onsubmit({ preventDefault() {} });
  assert.equal(v.els.home.hidden, false, 'the same name: back home');
  const w = await start();
  w.els['rename-cancel'].onclick();
  assert.equal(w.els.home.hidden, false);
  const x = await start();
  x.els['rename-form'].onkeydown({ key: 'Escape', preventDefault() {} });
  assert.equal(x.els.home.hidden, false);
  for (const view of [v, w, x]) assert.equal(renames(view), 0);
  for (const view of [busy, lost, other, v, w, x]) libOnly(view);
});

test('library home: Export by id offers only that file\'s saved bytes and says what this view could not do', async () => {
  const v = await libraryView();
  const r = row(v, 'Landing', '2 pages');
  r.more.onclick();
  fileMenuOf(r).children[2].onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_export', arguments: { id: ID1 } },
    'by the listed id, with nothing opened');
  await answer(v, { action: 'exported', binding: null, canvas: { id: ID1, name: 'Landing' }, content: 'canvas-only',
    file_name: 'Landing.opdcanvas.json', bytes: 38, payload: '{"format":"openpaperdesign-canvas"}' }, libAction(v));
  assert.equal(v.els.home.hidden, false, 'the list stays');
  assert.match(v.els.notice.textContent, /can't download files here, so no file was saved/);
  assert.equal(libCalls(v, 'openpaperdesign_app_open'), 0);
  // Another file's bytes are never offered.
  const w = await libraryView();
  const s = row(w, 'Landing', '2 pages');
  s.more.onclick();
  fileMenuOf(s).children[2].onclick();
  await flush();
  await answer(w, { action: 'exported', binding: null, canvas: { id: ID2, name: 'Notes' }, content: 'canvas-only',
    file_name: 'Notes.opdcanvas.json', bytes: 38, payload: '{}' }, libAction(w));
  assert.match(w.els['result-state'].textContent, /another canvas than the one shown here, so nothing was downloaded/);
  libOnly(v);
  libOnly(w);
});

// ---- the in-panel editor (an app-library view whose build has the editor module) ----
// The editor itself (renderer transfer, page transport, Session, mount) is panel.js, tested
// against the real module in crates/openpaperdesign-web/tests/panel.test.mjs. Here a recording
// stand-in with the factory's shape takes its place, to check what the view does around it.

// The in-panel editor's own tools (panel.js calls them through the view's rpc): the canvas
// tools it uses, and the per-page ones the server still offers older views.
const PANEL_TOOLS = new Set([...LIB_TOOLS, 'openpaperdesign_app_editor_asset', 'openpaperdesign_app_canvas',
  'openpaperdesign_app_canvas_revision', 'openpaperdesign_app_canvas_save', 'openpaperdesign_app_page',
  'openpaperdesign_app_page_revision', 'openpaperdesign_app_page_save', 'openpaperdesign_app_ready', 'openpaperdesign_app_page_release']);
const panelOnly = (v) => {
  assert.ok(methods(v).every((m) => ALLOWED.has(m)), methods(v).join());
  for (const m of v.sent.filter((m) => m.method === 'tools/call')) assert.ok(PANEL_TOOLS.has(m.params.name), m.params.name);
  assert.equal(v.count('ui/message'), 0, 'no message to the agent, ever');
};

function fakePanel() {
  const log = [];
  const handles = [];
  const factory = (opts) => {
    factory.rpc = opts.rpc;
    log.push('create');
    return {
      open(o) {
        let settle;
        const h = {
          o, nextDrain: 'clean', drainGate: null,
          started: new Promise((resolve, reject) => { settle = { resolve, reject }; }),
          drain: async () => { log.push('drain'); if (h.drainGate) await h.drainGate; return h.nextDrain; },
          resume: () => { log.push('resume'); },
          dispose: async () => { log.push('dispose'); },
          // A kept preference the view learned late (panel.js `setAppearance`), recorded.
          kept: [],
          setAppearance: (a) => { h.kept.push(a); },
        };
        h.started.catch(() => {});
        h.finish = () => settle.resolve('mounted');
        h.fail = (e) => settle.reject(e);
        handles.push(h);
        log.push('open ' + o.binding);
        return h;
      },
    };
  };
  factory.log = log;
  factory.handles = handles;
  return factory;
}

async function panelView(context = {}) {
  const fake = fakePanel();
  const v = mount({ library: true, panel: fake });
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: context } });
  await flush();
  v.from({ id: v.last('tools/call').id, result: toolText(LIBRARY_LIST) });
  await flush();
  return { v, fake };
}
// A library editor result for the view's own editor: no link.
const PANEL_EDITOR = (action, id = ID1, extra = {}) => {
  const e = LIB_EDITOR(action, id, { panel: true, panel_editor: 'built in', open_in: 'this view', ...extra });
  delete e.url;
  return e;
};
// Answer the list the view reads again on its way home.
async function relist(v) {
  const call = v.last('tools/call');
  assert.equal(call.params.name, LIB_LIST);
  v.from({ id: call.id, result: toolText(LIBRARY_LIST) });
  await flush();
}
// Open a file from the list: with the module, straight into the editor here.
async function openInPanel(v) {
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1, panel: true } });
  await answer(v, PANEL_EDITOR('opened'), libAction(v));
}
// The close Home sends once the editor was let go: exactly its binding, not forced.
const CLOSED = { action: 'closed', canvas: { id: ID1 } };
async function closedHome(v, reply = CLOSED) {
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } });
  await answer(v, reply, libAction(v));
}

// ---- the resolved appearance decides Home's colours; a host palette only joins it when it
// was sent for that appearance (theme-host-palette-correction/observed.json) ----

const LIGHT_HOST = { theme: 'light', styles: { variables: { '--color-background-primary': '#ffffff',
  '--color-background-secondary': '#f5f5f5', '--color-text-primary': '#202020', '--color-text-secondary': '#606060',
  '--font-sans': 'Inter, system-ui', '--border-radius-md': '6px' } } };
const DARK_HOST = { theme: 'dark', styles: { variables: { '--color-background-primary': '#101010',
  '--color-text-primary': 'rgb(240 240 240)', '--font-sans': 'Inter, system-ui' } } };
const SHAPE = { '--font-sans': 'Inter, system-ui', '--border-radius-md': '6px' };
const colourKeys = (props) => Object.keys(props).filter((k) => k.startsWith('--color-'));
const hostSays = (v, params) => v.from({ method: 'ui/notifications/host-context-changed', params });

// A library view with the editor module whose list reports `kept` as the appearance.
async function appearanceView(kept, context, media = null) {
  const fake = fakePanel();
  const v = mount({ library: true, panel: fake, media });
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: context } });
  await flush();
  v.from({ id: v.last('tools/call').id, result: toolText({ ...LIBRARY_LIST, appearance: kept }) });
  await flush();
  return { v, fake };
}

test('explicit Dark keeps Home dark against a light host palette (the observed defect); Light lets that palette join; stale colours go', async () => {
  const { v, fake } = await appearanceView('dark', {});
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  hostSays(v, LIGHT_HOST);
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'still dark');
  assert.deepEqual(colourKeys(v.props), [], 'no light host colour paints the dark view');
  assert.deepEqual(v.props, SHAPE, 'the host\'s font and radius still apply');
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onAppearance('light');
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.equal(v.props['--color-background-primary'], '#ffffff', 'Light agrees with the host: its palette joins');
  assert.equal(v.props['--color-text-secondary'], '#606060');
  h.o.onAppearance('dark');
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  assert.deepEqual(v.props, SHAPE, 'back to Dark: the light colours are removed again');
  assert.equal(libCalls(v, 'openpaperdesign_app_appearance'), 2, 'only the two menu choices were kept');
  panelOnly(v);
});

test('explicit Light keeps Home light against a dark host palette', async () => {
  const { v } = await appearanceView('light', DARK_HOST);
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.deepEqual(colourKeys(v.props), [], 'the dark palette shown before the kept Light arrived is gone');
  assert.equal(v.props['--font-sans'], 'Inter, system-ui');
  hostSays(v, { theme: 'dark' });
  assert.equal(v.rootAttrs['data-theme'], 'light', 'a host theme change doesn\'t override an explicit mode');
  assert.deepEqual(colourKeys(v.props), []);
  libOnly(v);
});

test('System follows the OS first, live, whatever the host says; the host palette joins only when it agrees, and never leaves stale colours', async () => {
  const media = osScheme(true);
  const { v } = await appearanceView('system', LIGHT_HOST, media);
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'the OS, not the host\'s light');
  assert.deepEqual(colourKeys(v.props), [], 'the light palette doesn\'t paint the dark view');
  media.change(false);
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.equal(v.props['--color-background-primary'], '#ffffff', 'OS and host agree: the host palette joins');
  hostSays(v, { theme: 'dark' });
  assert.equal(v.rootAttrs['data-theme'], 'light', 'the host\'s theme doesn\'t override the OS');
  assert.equal(v.props['--color-background-primary'], '#ffffff', 'the light palette still paints the light view');
  hostSays(v, DARK_HOST);
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.deepEqual(colourKeys(v.props), [], 'the host\'s dark palette never paints the light view');
  media.change(true);
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  assert.equal(v.props['--color-background-primary'], '#101010', 'OS and host agree again: dark palette');
  assert.equal(v.props['--color-background-secondary'], undefined, 'no light colour is left over from the earlier palette');
  assert.equal(v.props['--border-radius-md'], '6px', 'the earlier radius is kept (non-colour, merged)');
  assert.equal(libCalls(v, 'openpaperdesign_app_appearance'), 0, 'an OS or host change keeps nothing');
  libOnly(v);
});

// ---- R30: freshness of the preference, the no-signal fallback, and whose refusal it is ----
// Consolidated from the R30 review's probes (theme-r30-corrections/home-review-probes.mjs),
// with their reproductions and assertions, plus the controls the findings ask for.

const APPEARANCE_TOOL = 'openpaperdesign_app_appearance';
const UNKEPT = 'Appearance applied here, but it couldn\'t be kept for next time.';

test('R30-F1: an older list never replaces a newer explicit choice, and the list itself still shows', async () => {
  const fake = fakePanel();
  const v = mount({ library: true, panel: fake, media: osScheme(false) });
  v.from({ id: v.sent[0].id, result: { hostCapabilities: {}, hostContext: { theme: 'light' } } });
  await flush();
  const initial = v.last('tools/call');
  assert.equal(initial.params.name, LIB_LIST);
  v.els['new-file'].onclick();
  await flush();
  const create = libAction(v);
  assert.equal(create.params.name, 'openpaperdesign_app_create');
  await answer(v, PANEL_EDITOR('created'), create);
  const h = fake.handles[0];
  h.o.onStatus('Ready');
  h.o.onAppearance('dark');
  await flush();
  const preference = libAction(v);
  assert.deepEqual(preference.params, { name: APPEARANCE_TOOL, arguments: { appearance: 'dark' } });
  await answer(v, { appearance: 'dark' }, preference);
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  // The read that began before the choice comes back with the older System.
  await answer(v, { ...LIBRARY_LIST, appearance: 'system' }, initial);
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'an older list cannot override a newer explicit preference');
  assert.deepEqual(h.kept, [], 'nor is the editor told to take it');
  assert.deepEqual(rows(v).map((r) => r.name), ['Landing', 'Notes'], 'the list itself is still shown');
  panelOnly(v);
});

test('R30-F1: a kept preference that arrives only after New mounted reaches that editor, as no new choice', async () => {
  const fake = fakePanel();
  const v = mount({ library: true, panel: fake, media: osScheme(false) });
  v.from({ id: v.sent[0].id, result: { hostCapabilities: {}, hostContext: { theme: 'light' } } });
  await flush();
  const initial = v.last('tools/call');
  v.els['new-file'].onclick();
  await flush();
  await answer(v, PANEL_EDITOR('created'), libAction(v));
  const h = fake.handles[0];
  assert.equal(h.o.appearance, 'system', 'opened before the kept preference was known');
  await answer(v, { ...LIBRARY_LIST, appearance: 'dark' }, initial);
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  assert.deepEqual(h.kept, ['dark'], 'the mounted editor takes the kept Dark');
  assert.equal(libCalls(v, APPEARANCE_TOOL), 0, 'learning the kept preference keeps nothing');
  panelOnly(v);
});

test('R30-F1: a list read while a keep is under way, or after a refused choice, leaves this view\'s choice', async () => {
  const { v, fake } = await appearanceView('system', { theme: 'light' }, osScheme(false));
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onStatus('Ready');
  h.o.onAppearance('dark');
  await flush();
  const keep = libAction(v);
  // A refresh that begins and ends while the keep is unanswered.
  v.els['m-refresh'].onclick();
  await flush();
  await answer(v, { ...LIBRARY_LIST, appearance: 'system' }, v.last('tools/call'));
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'a keep under way owns the preference');
  // The keep is refused (current choice, current editor): said, and the choice still applies.
  await answer(v, { refused: 'The appearance couldn\'t be kept.' }, keep);
  assert.equal(v.els['ed-alert-text'].textContent, UNKEPT);
  assert.equal(v.els['ed-alert'].hidden, false);
  v.els['m-refresh'].onclick();
  await flush();
  await answer(v, { ...LIBRARY_LIST, appearance: 'system' }, v.last('tools/call'));
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'the refused choice stays this view\'s');
  assert.deepEqual(h.kept, []);
  // A later kept choice makes the library's list authoritative again.
  h.o.onAppearance('light');
  await flush();
  await answer(v, { appearance: 'light' }, libAction(v));
  v.els['m-refresh'].onclick();
  await flush();
  await answer(v, { ...LIBRARY_LIST, appearance: 'dark' }, v.last('tools/call'));
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'a fresh read after a kept choice applies (changed elsewhere)');
  assert.deepEqual(h.kept, ['dark']);
  panelOnly(v);
});

test('R30-F2: System without any OS or host signal is Light, also after an explicit Dark and when a fallback signal goes', async () => {
  const { v, fake } = await appearanceView('dark', {});
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  await openInPanel(v);
  fake.handles[0].o.onAppearance('system');
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'light', 'System without a signal resolves Light, as the editor\'s fallback does');
  // The host's theme stands in (no browser signal here), then goes away again.
  hostSays(v, { theme: 'dark' });
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  hostSays(v, { theme: null });
  assert.equal(v.rootAttrs['data-theme'], 'light', 'the fallback signal gone: Light again, not the last Dark');
  // An explicit mode still wins without any signal.
  fake.handles[0].o.onAppearance('dark');
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'dark');
  panelOnly(v);
});

test('R30-F3: an older refusal after a newer kept choice says nothing', async () => {
  const { v, fake } = await appearanceView('system', { theme: 'light' });
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onStatus('Ready');
  h.o.onAppearance('dark');
  await flush();
  const old = libAction(v);
  h.o.onAppearance('light');
  await flush();
  const latest = libAction(v);
  assert.notEqual(old.id, latest.id);
  await answer(v, { appearance: 'light' }, latest);
  assert.equal(v.rootAttrs['data-theme'], 'light');
  assert.equal(v.els['ed-alert'].hidden, true);
  await answer(v, { refused: 'The older Dark preference could not be kept' }, old);
  assert.equal(v.els['ed-alert'].hidden, true, 'a later successful preference supersedes this older refusal');
  assert.equal(v.rootAttrs['data-theme'], 'light');
  panelOnly(v);
});

test('R30-F3: a refusal for a replaced editor says nothing on its replacement; a current one is said, and latched problems stay', async () => {
  const { v, fake } = await appearanceView('system', { theme: 'light' });
  await openInPanel(v);
  const old = fake.handles[0];
  old.o.onStatus('Ready');
  old.o.onAppearance('dark');
  await flush();
  const pending = libAction(v);
  old.o.onHome();
  await flush();
  await closedHome(v);
  await relist(v);
  await openInPanel(v);
  const current = fake.handles[1];
  current.o.onStatus('Ready');
  assert.equal(v.els['ed-alert'].hidden, true);
  await answer(v, { refused: 'Preference could not be persisted' }, pending);
  assert.equal(v.els['ed-alert'].hidden, true, 'the old editor owns this failure');
  // Control: the shown editor's own refusal is said.
  current.o.onAppearance('light');
  await flush();
  await answer(v, { refused: 'Preference could not be persisted' }, libAction(v));
  assert.equal(v.els['ed-alert'].hidden, false);
  assert.equal(v.els['ed-alert-text'].textContent, UNKEPT);
  // Control: a current save failure or terminal error stays shown through a current refusal.
  for (const [message, fatal] of [['Save failed; retrying', false], ['The renderer stopped: device lost', true]]) {
    const w = await appearanceView('system', { theme: 'light' });
    await openInPanel(w.v);
    const e = w.fake.handles[0];
    e.o.onAppearance('dark');
    await flush();
    const keep = libAction(w.v);
    e.o.onStatus(message, fatal);
    await answer(w.v, { refused: 'Preference could not be persisted' }, keep);
    assert.equal(w.v.els['ed-alert-text'].textContent, message);
    assert.equal(w.v.els['ed-alert'].className, 'ed-alert bad');
    assert.equal(w.v.els['ed-alert'].hidden, false);
  }
  panelOnly(v);
});

test('panel: the editor gets the kept appearance; a menu choice shows at once and is kept by the library as a validated preference only', async () => {
  const fake = fakePanel();
  const v = mount({ library: true, panel: fake });
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: { theme: 'light' } } });
  await flush();
  v.from({ id: v.last('tools/call').id, result: toolText({ ...LIBRARY_LIST, appearance: 'dark' }) });
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'dark', 'the kept Dark wins over the host\'s light');
  await openInPanel(v);
  const h = fake.handles[0];
  assert.equal(h.o.appearance, 'dark', 'the editor starts in the kept preference');
  assert.equal(h.o.hostTheme(), 'light', 'the host\'s theme, the editor\'s OS fallback');
  // Menu ▸ Appearance ▸ Light in the editor: shown here at once, then kept.
  h.o.onAppearance('light');
  await flush();
  assert.equal(v.rootAttrs['data-theme'], 'light');
  const kept = libAction(v);
  assert.deepEqual(kept.params, { name: 'openpaperdesign_app_appearance', arguments: { appearance: 'light' } });
  await answer(v, { appearance: 'light', note: 'Kept.' }, kept);
  // Anything else goes as System, never as it came.
  h.o.onAppearance('sepia');
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_appearance', arguments: { appearance: 'system' } });
  // Not kept: said over the workspace; the choice still applies here.
  await answer(v, { refused: 'An appearance of system, light or dark is required. Nothing was changed.' }, libAction(v));
  assert.match(v.els['ed-alert-text'].textContent, /couldn't be kept/);
  assert.equal(v.rootAttrs['data-theme'], 'light', 'System with the host\'s light (no browser signal here)');
  panelOnly(v);
});

test('panel: Open mounts the editor right here on the result\'s binding: no result screen, no link, no message', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  assert.equal(v.els.editor.hidden, false);
  assert.equal(v.els.result.hidden, true, 'no result screen in between');
  assert.equal(v.els['menu-button'].hidden, true);
  assert.equal(v.els.app.getAttribute('data-view'), 'editor');
  assert.equal(v.count('ui/open-link'), 0, 'no link is asked for or opened');
  assert.equal(libCalls(v, 'openpaperdesign_app_editor'), 0, 'and no link-giving editor call');
  assert.deepEqual(fake.log, ['create', 'open ' + B1]);
  const h = fake.handles[0];
  assert.equal(h.o.binding, B1, 'the editor edits exactly the binding the result named');
  assert.equal(h.o.refs.canvas, v.els['ed-canvas']);
  assert.equal(h.o.refs.conflict, v.els['ed-conflict']);
  assert.equal(v.document.activeElement, v.els['ed-canvas'], 'the workspace has the keyboard');
  // Upstream's own workspace draws its tools in the canvas: no toolbar of the view's own.
  assert.equal(h.o.refs.tools, undefined, 'no parallel tool buttons');
  assert.equal(h.o.refs.actions, undefined);
  assert.ok(!/id="ed-(select|hand|rect|ellipse|undo|redo|delete)"/.test(html), 'and none in the markup');
  // The file's name is for assistive technology here; upstream's top bar shows it.
  assert.equal(v.els['ed-title'].textContent, 'Landing');
  assert.match(html, /<h2 id="ed-title" class="sr-only" tabindex="-1">/);
  assert.match(html, /<p id="ed-status" class="sr-only" role="status" aria-live="polite">/);
  // The factory got the view's own request function: its calls go to the host, as the view's.
  fake.rpc('tools/call', { name: 'openpaperdesign_app_canvas', arguments: { binding: B1 } });
  assert.deepEqual(v.last('tools/call').params, { name: 'openpaperdesign_app_canvas', arguments: { binding: B1 } });
  panelOnly(v);
});

test('panel: stable healthy states stay in the status live region; loading shows until Ready; failures, conflicts and notes show', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  const h = fake.handles[0];
  // On the way to Ready (the renderer's parts may take a while): a small, temporary indication.
  for (const s of ['Loading the editor…', 'Opening the canvas…', 'Starting renderer…']) {
    if (s !== 'Loading the editor…') h.o.onStatus(s);
    assert.equal(v.els['ed-status'].textContent, s);
    assert.equal(v.els['ed-alert'].hidden, false, `${s}: shown while loading`);
    assert.equal(v.els['ed-alert'].className, 'ed-alert loading');
    assert.equal(v.els['ed-alert-text'].textContent, s);
    assert.equal(v.els['ed-leave'].hidden, true, 'no controls with it');
    assert.equal(v.els['ed-home'].hidden, true);
  }
  for (const s of ['Ready', 'Saving…', 'Saved', 'Updated with changes made elsewhere']) {
    h.o.onStatus(s);
    assert.equal(v.els['ed-status'].textContent, s, 'observable');
    assert.equal(v.els['ed-alert'].hidden, true, `${s}: no visible chrome`);
  }
  h.o.onStatus('Downloaded "Landing.png".');
  assert.equal(v.els['ed-alert'].hidden, false, 'a note is shown');
  assert.equal(v.els['ed-alert'].className, 'ed-alert');
  h.o.onStatus('Ready');
  assert.equal(v.els['ed-alert'].hidden, true, 'a note goes once healthy again');
  // A terminal error (the mount's fatal status) stays: nothing healthy hides it.
  h.o.onStatus('The renderer stopped: device lost', true);
  assert.equal(v.els['ed-alert'].hidden, false);
  assert.equal(v.els['ed-alert'].className, 'ed-alert bad');
  assert.equal(v.els['ed-alert-text'].textContent, 'The renderer stopped: device lost');
  assert.equal(v.els['ed-status'].className, 'sr-only bad');
  h.o.onStatus('Ready');
  assert.equal(v.els['ed-status'].textContent, 'Ready', 'the live region has the status');
  assert.equal(v.els['ed-alert'].hidden, false, 'the error is still shown');
  assert.equal(v.els['ed-alert-text'].textContent, 'The renderer stopped: device lost');
  // The same rule as the browser page's (editor.js).
  assert.deepEqual(['Ready', 'Saved', 'Saving…', 'Updated with changes made elsewhere', 'Starting renderer…',
    'Loading the editor…', 'Opening the canvas…', 'Downloaded "x.svg".', 'Save failed; retrying',
    'Save failed (server unreachable); retrying', 'Not saved: conflict'].map((s) => opdStatusKind(s, false)),
  ['quiet', 'quiet', 'quiet', 'quiet', 'loading', 'loading', 'loading', 'note', 'failure', 'failure', 'conflict']);
  assert.equal(opdStatusKind('Ready', true), 'error', 'an error is never quiet');
  assert.equal(opdStatusKind('Loading the editor…', true), 'error', 'nor loading');
  assert.equal(opdQuietStatus('Ready', false), true);
  assert.equal(opdQuietStatus('Loading the editor…', false), false, 'loading is not quiet');
});

// ---- R25-F2: an autosave failure stays shown through its retry, with the real save tick and
// session. The mount forwards save statuses as text only (opdMountEditor's `ui.status`), so the
// stand-in `ui` does the same. Adapted from the R25 review's probe ("an unacknowledged retry
// must not hide the last save failure"), with its fixture and assertion kept.

/**
 * A ready session with one unsaved change, saving through `put`, and its tick's `ui`. For the
 * conflict banner's reload (`discardStep` → `Session.discard`): the server's canvas as
 * `fetchPage` returns it, the editor's `reload` (recorded in `reloaded`), and the banner's
 * `conflict`/`hideConflict` (recorded in `ui.banner`).
 */
function realSave(onStatus, put) {
  const ui = {
    banner: false,
    status: (text) => onStatus(text),
    conflict: () => { ui.banner = true; },
    hideConflict: () => { ui.banner = false; },
  };
  const disk = { text: '{"pages":[]}', revision: '"disk"' };
  const session = new Session({ put, fetchPage: async () => disk, log: () => {} });
  const reloaded = [];
  session.ed = {
    dirty: () => true, save_snapshot: () => '{"pages":[]}', save_failed: () => {}, save_succeeded: () => {},
    reload: (text) => { reloaded.push(text); },
  };
  session.state.ready = true;
  return { session, ui, reloaded, disk };
}

test('R25-F2: an unacknowledged retry must not hide the last save failure; the acknowledged save resolves it', async () => {
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  await openInPanel(v);
  const h = v.fake.handles[0];
  h.o.onStatus('Ready');
  assert.equal(v.els['ed-alert'].hidden, true, 'healthy: nothing shown');
  const { session, ui } = realSave(h.o.onStatus, async () => ({ ok: false, status: 500, revision: null }));
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert'].hidden, false, 'a failed save is initially shown');
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed; retrying');
  assert.equal(v.els['ed-alert'].className, 'ed-alert bad');
  let finish;
  session.deps.put = () => new Promise((resolve) => { finish = resolve; });
  const retry = saveStep(session, ui);
  const hiddenWhileUnconfirmed = v.els['ed-alert'].hidden;
  assert.equal(v.els['ed-status'].textContent, 'Saving…', 'the live region has the retry');
  assert.equal(hiddenWhileUnconfirmed, false, 'last save failure stays visible until a successful save acknowledgement');
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed; retrying');
  v.runAllTimers();
  await flush();
  assert.equal(v.els['ed-alert'].hidden, false, 'no timer hides it');
  finish({ ok: true, status: 200, revision: '"next"' });
  await retry;
  assert.equal(v.els['ed-status'].textContent, 'Saved');
  assert.equal(v.els['ed-alert'].hidden, true, 'the acknowledged save resolves it');
  h.o.onStatus('Saving…');
  assert.equal(v.els['ed-alert'].hidden, true, 'healthy again: Saving… shows nothing');
});

test('R25-F2: persistent, unreachable and conflict failures stay shown and distinct; a new editor starts clear', async () => {
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  await openInPanel(v);
  const h = v.fake.handles[0];
  const { session, ui, reloaded, disk } = realSave(h.o.onStatus, async () => ({ ok: false, status: 500, revision: null }));
  await saveStep(session, ui);
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed; retrying', 'still failing');
  session.deps.put = async () => { throw new Error('offline'); };
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed (server unreachable); retrying');
  h.o.onStatus('Ready');
  h.o.onStatus('Updated with changes made elsewhere');
  assert.equal(v.els['ed-alert'].hidden, false, 'nothing healthy resolves an unsaved change');
  // A conflict: not retried (no "Saving…" follows), shown until the explicit reload.
  session.deps.put = async () => ({ ok: false, status: 409, revision: '"other"' });
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert-text'].textContent, 'Not saved: conflict');
  assert.equal(v.els['ed-alert'].className, 'ed-alert bad');
  assert.equal(session.state.conflict, true);
  assert.equal(ui.banner, true, 'the conflict banner is shown');
  await saveStep(session, ui);
  assert.equal(v.els['ed-status'].textContent, 'Not saved: conflict', 'no retry while in conflict');
  v.runAllTimers();
  await flush();
  assert.equal(v.els['ed-alert'].hidden, false);
  // The banner's reload (the real discard): the version on disk is loaded, the session leaves
  // the conflict, and the view says so.
  assert.equal(await discardStep(session, ui), 'discarded');
  assert.equal(session.state.conflict, false, 'the session is out of the conflict');
  assert.equal(session.state.revision, disk.revision, 'at the version on disk');
  assert.deepEqual(reloaded, [disk.text], 'the editor loaded it');
  assert.equal(ui.banner, false, 'the banner is gone');
  assert.equal(v.els['ed-status'].textContent, 'Reloaded the version on disk');
  h.o.onStatus('Ready');
  assert.equal(v.els['ed-alert'].hidden, true, 'resolved by the reload');
  // A failure, then Home (refused, the editor stays): its reason shows; once a save succeeds,
  // both go.
  session.deps.put = async () => ({ ok: false, status: 500, revision: null });
  await saveStep(session, ui);
  h.nextDrain = 'failed';
  v.els.back.onclick();
  await flush();
  assert.match(v.els['ed-alert-text'].textContent, /couldn't be saved yet/);
  assert.equal(v.els['ed-leave'].hidden, false);
  h.o.onStatus('Saving…');
  assert.match(v.els['ed-alert-text'].textContent, /couldn't be saved yet/, 'the reason, not an echo');
  session.deps.put = async () => ({ ok: true, status: 200, revision: '"ok"' });
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert'].hidden, true);
  assert.equal(v.els['ed-leave'].hidden, true);
  // A new editor has nothing outstanding.
  h.nextDrain = 'clean';
  v.els.back.onclick();
  await flush();
  await closedHome(v);
  await relist(v);
  await openInPanel(v);
  v.fake.handles[1].o.onStatus('Ready');
  assert.equal(v.els['ed-alert'].hidden, true);
});

test('panel: New file, Import, Open current file and the menu\'s Open all mount here, asking for the panel', async () => {
  const routes = [
    ['New file', (v) => { v.els['new-file'].onclick(); }, 'openpaperdesign_app_create', { name: 'Untitled', panel: true }, 'created'],
    ['Import', (v) => {
      v.els['import-file'].files = [{ size: 9, name: 'canvas.json', text: async () => 'FILE-TEXT' }];
      return v.els['import-file'].onchange();
    }, 'openpaperdesign_app_import', { payload: 'FILE-TEXT', panel: true }, 'imported'],
    ['Open current file', (v) => v.els['m-current'].onclick(), 'openpaperdesign_app_editor', { panel: true }, 'reopened'],
    ['the menu\'s Open', (v) => { const r = row(v, 'Notes', '1 page · open here'); r.more.onclick(); r.li.children[1].children[0].onclick(); },
      'openpaperdesign_app_open', { id: ID2, panel: true }, 'already-open'],
  ];
  for (const [route, start, tool, args, action] of routes) {
    const { v, fake } = await panelView();
    await start(v);
    await flush();
    assert.deepEqual(libAction(v).params, { name: tool, arguments: args }, route);
    await answer(v, PANEL_EDITOR(action, tool === 'openpaperdesign_app_open' ? ID2 : ID1), libAction(v));
    assert.equal(v.els.editor.hidden, false, route);
    assert.equal(fake.handles.length, 1, route);
    assert.equal(v.count('ui/open-link'), 0, `${route}: no link`);
    assert.match(v.els.live.textContent, /Loading the editor$/, route);
    panelOnly(v);
  }
  // A refusal opens nothing and mounts nothing.
  const { v, fake } = await panelView();
  v.els['new-file'].onclick();
  await flush();
  await answer(v, { refused: 'nope' }, libAction(v));
  assert.equal(fake.handles.length, 0);
  assert.equal(v.els['result-title'].textContent, 'Not done');
});

test('panel: Home (the workspace\'s All projects) saves what is left, lets the editor go, then closes exactly its binding', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onHome();
  await flush();
  assert.deepEqual(fake.log.slice(2), ['drain', 'dispose'], 'drained, then let go, before closing');
  await closedHome(v);
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.els['menu-button'].hidden, false);
  assert.equal(v.els.notice.textContent, '', 'a confirmed close needs no note');
  assert.equal(libCalls(v, 'openpaperdesign_app_close'), 1);
  await relist(v);
  // A late Home from the editor that was let go does nothing.
  h.o.onHome();
  await flush();
  assert.equal(libCalls(v, 'openpaperdesign_app_close'), 1);
  // The keyboard Back is the same way home; another file opens in a fresh editor.
  await openInPanel(v);
  v.els.back.onclick();
  await flush();
  await closedHome(v);
  assert.equal(fake.handles.length, 2);
  panelOnly(v);
});

test('panel: a second Home while the close is under way sends nothing more', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  fake.handles[0].o.onHome();
  await flush();
  v.els.back.onclick();
  fake.handles[0].o.onHome();
  await flush();
  assert.equal(libCalls(v, 'openpaperdesign_app_close'), 1);
  assert.equal(fake.log.filter((e) => e === 'dispose').length, 1);
  await closedHome(v);
  assert.equal(v.els.home.hidden, false);
});

test('panel: a refused, failed or unanswered close is said on the library; nothing claims it closed or forces it', async () => {
  for (const [reply, want] of [
    [{ refused: 'The editor of this canvas was opened (its browser tab, or the editor in another OpenPaperDesign view). Closing stops the server ... then close again with force: true.' },
      /^The file stays open in this window: The editor of this canvas was opened/],
    [{ error: { code: -32001, message: 'lost' } }, /^Codex didn't confirm closing the file, so it may still be open in this window\.$/],
  ]) {
    const { v, fake } = await panelView();
    await openInPanel(v);
    fake.handles[0].o.onHome();
    await flush();
    await closedHome(v, reply);
    assert.equal(v.els.home.hidden, false);
    assert.match(v.els.notice.textContent, want);
    assert.doesNotMatch(v.els.notice.textContent + v.els.live.textContent, /closed\./i);
    assert.equal(v.els.force.hidden, true, 'no force offered from Home');
    assert.equal(libCalls(v, 'openpaperdesign_app_close'), 1);
    panelOnly(v);
  }
  // Unanswered: the view's own timeout, with live timers.
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  await openInPanel(v);
  v.fake.handles[0].o.onHome();
  await flush();
  assert.equal(libAction(v).params.name, 'openpaperdesign_app_close');
  v.tick(60000);
  await flush();
  assert.equal(v.els.home.hidden, false);
  assert.match(v.els.notice.textContent, /didn't confirm closing/);
});

test('panel: a close answer that arrives after a newer result belongs to nothing', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  fake.handles[0].o.onHome();
  await flush();
  const close = libAction(v);
  // The editor is gone, so a model's result may now replace the view.
  v.from(result(LIB_EDITOR('opened', ID2)));
  assert.equal(v.els.result.hidden, false);
  await answer(v, CLOSED, close);
  assert.equal(v.els.result.hidden, false, 'the newer result stays');
  assert.equal(v.els.home.hidden, true);
});

test('panel: an editor with a change that isn\'t saved stays until the person leaves without saving', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  const h = fake.handles[0];
  for (const [reason, want] of [['conflict', /changed elsewhere/], ['busy', /edit is still under way/],
    ['failed', /couldn't be saved yet/], ['unreachable', /may not be saved/], ['error', /renderer stopped/]]) {
    h.nextDrain = reason;
    // From the workspace's own All projects, or the keyboard Back: the same leaving path.
    if (reason === 'busy') h.o.onHome(); else v.els.back.onclick();
    await flush();
    assert.equal(v.els.editor.hidden, false, reason);
    assert.match(v.els['ed-status'].textContent, want, reason);
    assert.equal(v.els['ed-status'].className, 'sr-only bad');
    assert.equal(v.els['ed-alert'].hidden, false, 'the reason is shown');
    assert.match(v.els['ed-alert-text'].textContent, want, reason);
    assert.equal(v.els['ed-leave'].hidden, false);
    // Editing goes on, and a passing healthy status doesn't hide the reason or the way out.
    h.o.onStatus('Saving…');
    assert.equal(v.els['ed-alert'].hidden, false, `${reason}: still shown`);
    assert.equal(v.els['ed-leave'].hidden, false);
  }
  assert.equal(libCalls(v, 'openpaperdesign_app_close'), 0, 'not closed while a change is unsaved');
  assert.ok(!fake.log.includes('dispose'), 'never let go silently');
  assert.equal(fake.log.filter((e) => e === 'resume').length, 5, 'editing goes on each time');
  // Saved after all: the reason goes, and the next Home leaves normally.
  h.o.onStatus('Saved');
  assert.equal(v.els['ed-alert'].hidden, true);
  assert.equal(v.els['ed-leave'].hidden, true);
  // Refused again, then the explicit discard: let go, then exactly this binding closed (not forced).
  h.nextDrain = 'failed';
  v.els.back.onclick();
  await flush();
  await v.els['ed-leave'].onclick();
  await flush();
  assert.equal(fake.log.at(-1), 'dispose');
  await closedHome(v);
  assert.equal(v.els.home.hidden, false);
  panelOnly(v);
});

test('panel: the host\'s teardown is answered only after the editor was drained and let go', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  const h = fake.handles[0];
  let open;
  h.drainGate = new Promise((r) => { open = r; });
  v.from({ id: 'td-1', method: 'ui/resource-teardown', params: {} });
  await flush();
  const replied = () => v.sent.some((m) => m.id === 'td-1' && m.result);
  assert.equal(replied(), false, 'still draining');
  open();
  await flush();
  assert.deepEqual(fake.log.slice(2), ['drain', 'dispose']);
  assert.equal(replied(), true);
  assert.equal(libCalls(v, 'openpaperdesign_app_close'), 0, 'the view going away is not Home: it closes nothing');
  // Without an editor it is answered at once.
  const w = await panelView();
  w.v.from({ id: 'td-2', method: 'ui/resource-teardown', params: {} });
  await flush();
  assert.ok(w.v.sent.some((m) => m.id === 'td-2' && m.result));
});

test('panel: a model result or cancellation never replaces an editor', async () => {
  const { v } = await panelView();
  await openInPanel(v);
  v.from(result(LIB_EDITOR('opened', ID2)));
  v.from({ method: 'ui/notifications/tool-cancelled', params: {} });
  await flush();
  assert.equal(v.els.editor.hidden, false);
  assert.equal(v.count('ui/open-link'), 0, 'nothing is opened from it either');
});

test('panel: a start failure is shown as it came, with a visible way back (no workspace, so no All projects)', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  fake.handles[0].fail(new Error('WebAssembly.instantiate(): Refused to compile or instantiate WebAssembly module'));
  await flush();
  assert.match(v.els['ed-status'].textContent, /^The editor couldn't start here: WebAssembly\.instantiate\(\): Refused to compile/);
  assert.equal(v.els['ed-status'].className, 'sr-only bad');
  assert.equal(v.els['ed-alert'].hidden, false);
  assert.equal(v.els['ed-home'].hidden, false);
  assert.match(html, /<button id="ed-home" class="btn" type="button" hidden>Back to files<\/button>/);
  v.els['ed-home'].onclick();
  await flush();
  await closedHome(v);
  assert.equal(v.els.home.hidden, false);
});

// The editor used to carry a browser button (its own link-opening attempts, R12-F1 and R13-F1),
// Close, Export, Full screen and Details. The one app has none of them: the workspace is the
// whole view, and the library is the way to close or export a file. So no link is ever asked
// for from the editor, nothing in it can mark a browser link as given, and the R12-F1/R13-F1
// sequences (late links and acknowledgements after leaving) have nothing left to send; their
// departure halves stay below, against the real panel module.
test('panel: the editor adds no chrome of its own and never asks for or opens a link', async () => {
  const { v, fake } = await panelView({ availableDisplayModes: ['inline', 'fullscreen'] });
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onStatus('Ready');
  h.o.onStatus('Saved');
  for (const id of GONE) assert.ok(!html.includes(`id="${id}"`), id);
  // While it is shown, only the workspace (and the keyboard Back) remains.
  assert.equal(v.els.app.getAttribute('data-view'), 'editor');
  assert.equal(v.els['ed-alert'].hidden, true, 'no persistent Ready or Saved');
  assert.equal(v.els['menu-button'].hidden, true);
  h.o.onHome();
  await flush();
  await closedHome(v);
  assert.equal(v.count('ui/open-link'), 0);
  assert.equal(v.count('ui/request-display-mode'), 0, 'no Full screen, even when offered');
  // Every editor call asked for the panel: none could give out a browser link.
  const editorCalls = v.sent.filter((m) => m.method === 'tools/call' && ['openpaperdesign_app_editor', 'openpaperdesign_app_open',
    'openpaperdesign_app_create', 'openpaperdesign_app_import'].includes(m.params.name));
  assert.ok(editorCalls.length > 0);
  assert.ok(editorCalls.every((m) => m.params.arguments.panel === true), JSON.stringify(editorCalls.map((m) => m.params)));
  panelOnly(v);
});

test('panel: an Open answered without the panel, or for another canvas or binding, mounts nothing and opens nothing', async () => {
  const cases = [
    ['a link result', LIB_EDITOR('opened')],
    ['another canvas', PANEL_EDITOR('opened', ID2)],
    ['another canvas\'s binding', PANEL_EDITOR('opened', ID1, { binding: B2 })],
    ['an invalid binding', PANEL_EDITOR('opened', ID1, { binding: 'invalid' })],
    ['no binding', PANEL_EDITOR('opened', ID1, { binding: undefined })],
    ['a link result labelled for the panel', { ...LIB_EDITOR('opened'), panel: true }],
  ];
  // Collected first, so each case fails on its own.
  const observations = [];
  for (const [what, reply] of cases) {
    const { v, fake } = await panelView();
    row(v, 'Landing', '2 pages').open.onclick();
    await flush();
    await answer(v, reply, libAction(v));
    observations.push({ what, mounts: fake.handles.length, links: v.count('ui/open-link'),
      enabled: ['canvas', 'close', 'export'].filter((id) => !v.els[id].hidden), editor: !v.els.editor.hidden });
    panelOnly(v);
  }
  // The last one is a panel result with a link in it: it mounts (the link is never used).
  assert.deepEqual(observations.filter((o) => o.links || o.enabled.length), []);
  assert.deepEqual(observations.filter((o) => o.mounts).map((o) => o.what), ['a link result labelled for the panel']);
});

test('library: without the in-panel module, New file and Open still hand off to the web editor', async () => {
  assert.ok(libScript.includes('const opdCreatePanel = null;'), 'the slot as a build without the module leaves it');
  const v = await libraryView(LIBRARY_LIST, { panel: false });
  assert.deepEqual(rows(v).map((r) => [r.name, r.hint]), [['Landing', '2 pages'], ['Notes', '1 page · open here']]);
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1 } }, 'no panel asked for');
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  assert.equal(v.count('ui/open-link'), 1);
  assert.equal(v.last('ui/open-link').params.url, URL_OK);
  assert.equal(libCalls(v, 'openpaperdesign_app_editor'), 0);
  v.els.back.onclick();
  v.els['new-file'].onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_create', arguments: { name: 'Untitled' } });
  await answer(v, LIB_EDITOR('created'), libAction(v));
  assert.equal(v.count('ui/open-link'), 2);
  libOnly(v);
});

// ---- R11: what the home says about a browser opening, and which results it opens ----
// These run with the view's timers live (`clock: true`). Their sequences are adapted from the
// R11 reviewer's probes (home-probes.mjs, home-binding-probes.mjs).

async function openedInBrowser() {
  const v = await libraryView(LIBRARY_LIST, { panel: false, clock: true });
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  assert.equal(v.count('ui/open-link'), 1);
  return v;
}
// Run every timer the view has set, again for any it sets meanwhile, and settle.
async function letTimersRun(v) {
  for (let i = 0; i < 5; i++) {
    v.runAllTimers();
    await flush();
  }
}
const readinessPolls = (v) => libCalls(v, 'openpaperdesign_app_editor');

test('R11-F1 control: the clock is live here: an unanswered opening request times out and says so', async () => {
  const v = await openedInBrowser();
  assert.equal(v.els['canvas-state'].textContent, 'Asking Codex to open the canvas…');
  v.tick(60000);
  await flush();
  assert.equal(v.els['canvas-state'].textContent, 'Codex didn\'t answer. Press Open canvas to try again.');
  assert.equal(v.els.canvas.disabled, false);
  libOnly(v);
});

test('R11-F1: an acknowledged opening is reported as a request only; nothing later claims it loaded', async () => {
  const v = await openedInBrowser();
  await ack(v);
  const said = v.els['canvas-state'].textContent;
  assert.match(said, /doesn't confirm it loaded/);
  await letTimersRun(v);
  assert.equal(readinessPolls(v), 0, 'no readiness poll: the editor itself shows Ready');
  assert.equal(v.els['canvas-state'].textContent, said);
  assert.doesNotMatch(v.els.live.textContent, /loaded this canvas/);
  libOnly(v);
});

test('R11-F1: a failed opening stays failed after the timers run', async () => {
  for (const reply of [{ result: { isError: true } }, { error: { code: -32603, message: 'not opened' } }]) {
    const v = await openedInBrowser();
    await ack(v, 'ui/open-link', reply);
    assert.match(v.els['canvas-state'].textContent, /^Codex didn't open the link/);
    await letTimersRun(v);
    assert.equal(readinessPolls(v), 0);
    assert.match(v.els['canvas-state'].textContent, /^Codex didn't open the link/, JSON.stringify(reply));
  }
});

test('R11-F1: a reply to an earlier opening attempt never replaces a later attempt\'s failure', async () => {
  const v = await openedInBrowser();
  const first = v.last('ui/open-link');
  v.els.canvas.onclick(); // a retry while the first request is unanswered
  await flush();
  assert.equal(v.count('ui/open-link'), 2);
  await ack(v, 'ui/open-link', { error: { code: -32603, message: 'not opened' } });
  const failed = v.els['canvas-state'].textContent;
  assert.match(failed, /^Codex didn't open the link \(code -32603\)/);
  v.from({ id: first.id, result: {} }); // the first request is acknowledged late
  await flush();
  await letTimersRun(v);
  assert.equal(v.els['canvas-state'].textContent, failed);
  assert.equal(readinessPolls(v), 0);
});

test('R11-F1: readiness the binding got elsewhere (an earlier tab, another view\'s panel) is never this opening\'s', async () => {
  const v = await libraryView(LIBRARY_LIST, { clock: true });
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  // The result carries the binding's shared readiness and saves from another editor.
  await answer(v, LIB_EDITOR('opened', ID1, { editor_loaded: true, browser_saves: 3 }), libAction(v));
  await ack(v);
  await letTimersRun(v);
  const shown = ['result-state', 'canvas-state', 'live', 'notice'].flatMap((id) => texts(v.els[id])).join('\n');
  assert.doesNotMatch(shown, /loaded this canvas|Saved from the editor/);
  assert.match(v.els['canvas-state'].textContent, /doesn't confirm it loaded/);
  assert.equal(readinessPolls(v), 0);
  libOnly(v);
});

test('R11-F2: a library result is opened only with a valid binding of its own canvas', async () => {
  for (const [what, extra] of [
    ['an omitted binding', { binding: undefined }],
    ['an invalid binding', { binding: 'invalid' }],
    ['another canvas\'s binding', { binding: B2 }],
    ['no canvas id', { canvas: { name: 'Landing', pages: 1 } }],
  ]) {
    const v = await libraryView(LIBRARY_LIST, { clock: true });
    row(v, 'Landing', '2 pages').open.onclick();
    await flush();
    await answer(v, LIB_EDITOR('opened', ID1, extra), libAction(v));
    assert.equal(v.count('ui/open-link'), 0, `${what}: no navigation`);
    assert.match(v.els['result-state'].textContent, /canvas and binding don't match/, what);
    for (const id of ['canvas', 'close', 'export']) assert.equal(v.els[id].hidden, true, `${what}: ${id} stays hidden`);
    // No action is left targeting anything.
    v.els.close.onclick();
    v.els.export.onclick();
    await letTimersRun(v);
    for (const t of ['openpaperdesign_app_close', 'openpaperdesign_app_export', 'openpaperdesign_app_editor']) {
      assert.equal(libCalls(v, t), 0, `${what}: ${t}`);
    }
    libOnly(v);
  }
  // The valid control: the same answer with its own binding opens once and enables its actions.
  const v = await libraryView(LIBRARY_LIST, { clock: true });
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, LIB_EDITOR('opened'), libAction(v));
  assert.equal(v.count('ui/open-link'), 1);
  for (const id of ['canvas', 'close', 'export']) assert.equal(v.els[id].hidden, false, id);
  v.els.close.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } });
});

test('R11-F2 (panel): the in-view editor mounts only on a valid binding of the canvas asked for, and Home closes that one', async () => {
  // Open names the canvas, not a binding, so a newer opening of the same canvas is its answer;
  // what is mounted and later closed is exactly the answer's binding.
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(v, PANEL_EDITOR('opened', ID1, { binding: `${ID1}.9` }), libAction(v));
  assert.equal(v.fake.handles.length, 1);
  assert.equal(v.fake.handles[0].o.binding, `${ID1}.9`);
  v.fake.handles[0].o.onHome();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: `${ID1}.9` } });
  // An invalid or foreign binding mounts nothing.
  for (const binding of ['invalid', B2, undefined]) {
    const w = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
    row(w, 'Landing', '2 pages').open.onclick();
    await flush();
    await answer(w, PANEL_EDITOR('opened', ID1, { binding }), libAction(w));
    await letTimersRun(w);
    assert.equal(w.fake.handles.length, 0, String(binding));
    assert.match(w.els['result-state'].textContent, /canvas and binding don't match/);
    assert.equal(libCalls(w, 'openpaperdesign_app_close'), 0);
  }
});

// ---- R12: library answers without their label ----
// Live timers again. The unlabelled answers and the exact-binding controls are adapted from the
// R12 reviewer's probes (r12-probes.mjs). Its browser-button sequences (R12-F1) went with the
// button: see "the editor adds no chrome of its own" above.

// What a library action did with its answer.
const outcome = (v) => ({
  navigations: v.count('ui/open-link'),
  mounts: v.fake ? v.fake.handles.length : 0,
  workspace: fields(v).includes('Folder'),
  enabled: ['canvas', 'close', 'export'].filter((id) => !v.els[id].hidden),
});
// The library actions that hand a canvas to an editor, each started in a fresh view. A start
// isn't awaited: its action waits for the answer the test then gives (a flush exposes the
// request, Import's file read included).
const LIB_ROUTES = [
  ['Open', (v) => { row(v, 'Landing', '2 pages').open.onclick(); }],
  ['New file', (v) => { v.els['new-file'].onclick(); }],
  ['Open current file', (v) => v.els['m-current'].onclick()],
  ['Import', (v) => {
    v.els['import-file'].files = [{ size: 9, name: 'canvas.json', text: async () => 'FILE-TEXT' }];
    return v.els['import-file'].onchange();
  }],
];

test('R12-F2: a known library action checks its answer whatever storage the answer claims', async () => {
  // Collected first, so each case fails on its own.
  // With the module (the panel answers) and without it (the link answers).
  const observations = [];
  for (const panel of [false, true]) {
    for (const storage of [undefined, 'workspace']) {
      for (const [route, start] of LIB_ROUTES) {
        for (const binding of [undefined, 'invalid', B2]) {
          const v = await libraryView(LIBRARY_LIST, { panel, clock: true });
          start(v);
          await flush();
          // With workspace fields too, so a workspace presentation would show.
          const reply = (panel ? PANEL_EDITOR : LIB_EDITOR)('opened', ID1, { storage, binding, workspace: '/w/acme', project: 'design/Landing' });
          await answer(v, reply, libAction(v));
          const o = outcome(v);
          observations.push({ panel, storage: String(storage), route, binding: String(binding), ...o,
            refused: /canvas and binding don't match/.test(v.els['result-state'].textContent) });
          if (panel) panelOnly(v); else libOnly(v);
        }
      }
    }
  }
  assert.equal(observations.length, 48);
  assert.deepEqual(observations.filter((o) => o.navigations || o.mounts || o.workspace || o.enabled.length || !o.refused), []);
  // Controls without the module: the same routes, unlabelled, with the answer's own binding.
  for (const [route, start] of LIB_ROUTES) {
    const v = await libraryView(LIBRARY_LIST, { clock: true });
    start(v);
    await flush();
    await answer(v, LIB_EDITOR('opened', ID1, { storage: undefined }), libAction(v));
    assert.equal(v.count('ui/open-link'), 1, route);
    assert.deepEqual(fields(v), ['Canvas', 'Landing (1 page)', 'Stored in', 'OpenPaperDesign library'], route);
    v.els.close.onclick();
    await flush();
    assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } }, route);
    libOnly(v);
  }
  // Controls with the module: mounted on the answer's own binding, nothing opened, and Home
  // closes exactly that binding.
  for (const [route, start] of LIB_ROUTES) {
    const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
    start(v);
    await flush();
    assert.equal(libAction(v).params.arguments.panel, true, route);
    await answer(v, PANEL_EDITOR('opened', ID1, { storage: undefined }), libAction(v));
    assert.deepEqual(outcome(v), { navigations: 0, mounts: 1, workspace: false, enabled: [] }, route);
    assert.equal(v.fake.handles[0].o.binding, B1, route);
    v.fake.handles[0].o.onHome();
    await flush();
    assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } }, route);
    panelOnly(v);
  }
  // A primary Open names no binding, so a newer opening of the same canvas is its answer...
  const g = await libraryView(LIBRARY_LIST, { clock: true });
  row(g, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(g, LIB_EDITOR('opened', ID1, { storage: undefined, binding: `${ID1}.9` }), libAction(g));
  assert.equal(g.count('ui/open-link'), 1);
  g.els.close.onclick();
  await flush();
  assert.deepEqual(libAction(g).params, { name: 'openpaperdesign_app_close', arguments: { binding: `${ID1}.9` } });
  // ...but never in a way it didn't ask for: a panel answer without the module, or a link
  // answer with it.
  const p = await libraryView(LIBRARY_LIST, { clock: true });
  row(p, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(p, PANEL_EDITOR('opened', ID1, { storage: undefined }), libAction(p));
  assert.deepEqual(outcome(p), { navigations: 0, mounts: 0, workspace: false, enabled: [] });
  assert.match(p.els['result-state'].textContent, /another way of opening this canvas/);
  libOnly(p);
  const q = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  row(q, 'Landing', '2 pages').open.onclick();
  await flush();
  await answer(q, LIB_EDITOR('opened', ID1, { storage: undefined }), libAction(q));
  assert.deepEqual(outcome(q), { navigations: 0, mounts: 0, workspace: false, enabled: [] });
  assert.match(q.els['result-state'].textContent, /another way of opening this canvas/);
  panelOnly(q);
});

test('R12-F2: a library view treats a delivered editor result as a library one; a workspace view keeps its own', async () => {
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  v.from(result(LIB_EDITOR('opened', ID1, { storage: undefined, binding: 'invalid', workspace: '/w/acme' })));
  assert.match(v.els['result-state'].textContent, /canvas and binding don't match/);
  assert.deepEqual(outcome(v), { navigations: 0, mounts: 0, workspace: false, enabled: [] });
  v.from(result(LIB_EDITOR('opened', ID1, { storage: undefined })));
  assert.deepEqual(fields(v), ['Canvas', 'Landing (1 page)', 'Stored in', 'OpenPaperDesign library']);
  assert.equal(v.count('ui/open-link'), 0, 'a delivered result is shown, not opened');
  assert.equal(v.els.canvas.hidden, false);
  libOnly(v);
  // The workspace view: its unlabelled results are workspace results, as before.
  const w = await initialized();
  w.from(result(EDITOR('Alpha')));
  assert.deepEqual(fields(w), ['Canvas', 'Alpha (1 page)', 'Folder', 'acme']);
  w.els['m-current'].onclick();
  await flush();
  await answer(w, EDITOR('Beta', 'reopened'));
  assert.equal(w.count('ui/open-link'), 1);
  assert.deepEqual(fields(w), ['Canvas', 'Beta (1 page)', 'Folder', 'acme']);
  onlyAllowed(w);
});

// ---- R13: leaving the in-panel editor, with the real panel module ----
// The exact crates/openpaperdesign-web/static/panel.js lifecycle and transport (read here, so
// this needs that file); only the engine, the editor mount and the platform check are stand-ins.
// Nothing renders. Adapted from the R13 reviewer's r13-real-panel-probe.mjs; its late browser
// link/acknowledgement halves went with the editor's browser button.
let realPanelModule = null;
async function realPanelView() {
  if (!realPanelModule) {
    const source = readFileSync(new URL('../../openpaperdesign-web/static/panel.js', import.meta.url), 'utf8');
    realPanelModule = import('data:text/javascript;base64,'
      + Buffer.from('const opdIsMac = () => false;\n' + source).toString('base64'));
  }
  const panel = await realPanelModule;
  const editor = { nextDrain: 'clean', disposed: false, handle: null, opts: null, mountOpts: null };
  const factory = ({ rpc }) => {
    const real = panel.opdCreatePanel({
      rpc, identity: 'r13-fixture', nav: {}, lib: {}, env: {},
      engine: { init: async () => {}, start: () => {} },
      mount: (m) => {
        editor.mountOpts = m;
        return { started: Promise.resolve(), drain: async () => editor.nextDrain,
          resume: () => {}, dispose: () => { editor.disposed = true; } };
      },
    });
    return { open: (o) => { editor.opts = o; editor.handle = real.open(o); return editor.handle; } };
  };
  const v = mount({ library: true, panel: factory, clock: true });
  v.from({ id: v.sent[0].id, result: { protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: {} } });
  await flush();
  await answer(v, LIBRARY_LIST, v.last('tools/call'));
  row(v, 'Landing', '2 pages').open.onclick();
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_open', arguments: { id: ID1, panel: true } });
  await answer(v, PANEL_EDITOR('opened'), libAction(v));
  const asset = v.last('tools/call');
  assert.equal(asset.params.name, 'openpaperdesign_app_editor_asset');
  await answer(v, { asset: 'openpaperdesign_web_bg.wasm', identity: 'r13-fixture', bytes: 1,
    parts: 1, part: 0, offset: 0, part_bytes: 1, data: 'AA==' }, asset);
  // The real panel's first read: the whole canvas, naming only the binding; answered as the
  // server's canvas tool answers it.
  const canvas = v.last('tools/call');
  assert.deepEqual(canvas.params, { name: 'openpaperdesign_app_canvas', arguments: { binding: B1 } });
  await answer(v, { binding: B1, canvas: { name: 'Landing' }, revision: '"c1"', active_page: 'p1',
    pages: [{ id: 'p1', name: 'Page 1', revision: '"r1"', text: '{}' }], mount: 7 }, canvas);
  assert.equal(await editor.handle.started, 'mounted');
  // What upstream's workspace opens: the file's name (its top-bar title) apart from its pages.
  const opened = JSON.parse(editor.mountOpts.firstPage.text);
  assert.deepEqual([opened.canvas, opened.active_page, opened.pages.map((p) => p.name)], [{ name: 'Landing' }, 'p1', ['Page 1']]);
  assert.equal(typeof editor.mountOpts.onHome, 'function', 'the way back reaches the workspace');
  return { v, editor };
}

test('R13: with the real panel, Home, the workspace\'s All projects, a discard or teardown releases the mount first; only Home and the discard then close exactly its binding', async () => {
  // Collected first, so each case fails on its own.
  const observations = [];
  for (const departure of ['back', 'all projects', 'discard', 'teardown']) {
    const { v, editor } = await realPanelView();
    let leaving = null;
    if (departure === 'back') leaving = v.els.back.onclick();
    else if (departure === 'all projects') editor.mountOpts.onHome(); // upstream's Home, as the real panel passes it on
    else if (departure === 'discard') {
      editor.nextDrain = 'failed';
      await v.els.back.onclick();
      assert.equal(v.els['ed-leave'].hidden, false, 'leaving without saving is offered');
      leaving = v.els['ed-leave'].onclick();
    } else v.from({ id: 'td-r13', method: 'ui/resource-teardown', params: {} });
    await flush();
    // The mount is gone; the server hasn't answered its release yet, and nothing closes before it.
    const release = v.last('tools/call');
    const o = { departure, released: JSON.stringify(release.params)
      === JSON.stringify({ name: 'openpaperdesign_app_page_release', arguments: { binding: B1, mount: 7 } }),
    disposed: editor.disposed, closesBeforeRelease: libCalls(v, 'openpaperdesign_app_close') };
    await answer(v, { binding: B1, released: true }, release);
    if (leaving) await leaving;
    await flush();
    const closes = v.sent.filter((m) => m.method === 'tools/call' && m.params.name === 'openpaperdesign_app_close');
    o.closes = closes.map((m) => JSON.stringify(m.params.arguments));
    if (closes.length) await answer(v, { action: 'closed', canvas: { id: ID1 } }, closes[0]);
    o.left = departure === 'teardown' ? v.sent.some((m) => m.id === 'td-r13' && m.result) : !v.els.home.hidden;
    o.links = v.count('ui/open-link');
    observations.push(o);
    panelOnly(v);
  }
  assert.deepEqual(observations.filter((o) => !o.released || !o.disposed || o.closesBeforeRelease || !o.left || o.links), []);
  assert.deepEqual(Object.fromEntries(observations.map((o) => [o.departure, o.closes])), {
    back: [JSON.stringify({ binding: B1 })],
    'all projects': [JSON.stringify({ binding: B1 })],
    discard: [JSON.stringify({ binding: B1 })],
    teardown: [],
  });
});

test('R13 control: with the real panel, a refused Home keeps the editor (no release, no close); a later clean Home goes', async () => {
  const { v, editor } = await realPanelView();
  editor.nextDrain = 'failed';
  await v.els.back.onclick();
  assert.equal(v.els.editor.hidden, false);
  assert.equal(editor.disposed, false, 'a refused Home lets nothing go');
  assert.ok(!v.sent.some((m) => m.method === 'tools/call' && ['openpaperdesign_app_page_release', 'openpaperdesign_app_close'].includes(m.params.name)));
  editor.nextDrain = 'clean';
  const back = v.els.back.onclick();
  await flush();
  const release = v.last('tools/call');
  assert.deepEqual(release.params, { name: 'openpaperdesign_app_page_release', arguments: { binding: B1, mount: 7 } });
  await answer(v, { binding: B1, released: true }, release);
  await back;
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } });
  await answer(v, { action: 'closed', canvas: { id: ID1 } }, libAction(v));
  assert.equal(v.els.home.hidden, false);
  assert.equal(v.count('ui/open-link'), 0);
  panelOnly(v);
});

// ---- R25-F1: a departure owns its view, editor and binding across the mount release ----
// The real panel module, with the release acknowledgement held back while something newer
// takes the view. The first three cases are the R25 review's probes (lifecycle-probes.mjs),
// with their fixtures and assertions; only `probe` is this file's `test`, and its CLOSED is
// this file's.

// The Close calls the view sent so far.
const closes = (v) => v.sent.filter((m) => m.method === 'tools/call' && m.params.name === 'openpaperdesign_app_close');

test('R25 control: a normal release acknowledgement closes the originating binding', async () => {
  const { v } = await realPanelView();
  const back = v.els.back.onclick();
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  await answer(v, { binding: B1, released: true }, release);
  await back;
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } });
  await answer(v, CLOSED, libAction(v));
  assert.equal(v.els.home.hidden, false);
});

test('R25: a delayed disposal must not navigate a newer result', async () => {
  const { v } = await realPanelView();
  const back = v.els.back.onclick();
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  // The server has processed release but its response is delayed. A newer model result is
  // now delivered, which the production view accepts because the editor was let go.
  v.from(result(LIB_EDITOR('opened', ID2)));
  assert.equal(v.els.result.hidden, false);
  await answer(v, { binding: B1, released: true }, release);
  await back;
  const close = libAction(v);
  if (close.params.name === 'openpaperdesign_app_close') await answer(v, { refused: 'The shown canvas is no longer current; nothing was closed.' }, close);
  assert.equal(v.els.result.hidden, false, 'the newer result must stay after the old release/close finishes');
  // Added: nothing at all was closed for the old departure, and the newer result is intact.
  assert.equal(closes(v).length, 0, 'no Close from the departure that lost the view');
  assert.equal(v.els['result-title'].textContent, 'Notes');
  panelOnly(v);
});

test('R25: a delayed disposal must not hide a newly mounted editor', async () => {
  const { v, editor } = await realPanelView();
  const back = v.els.back.onclick();
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  // A Home result arrives while the old release acknowledgement is delayed.
  v.from(result({ surface: 'home' }));
  await flush();
  assert.equal(v.els.home.hidden, false);
  await relist(v);
  v.els['new-file'].onclick();
  await flush();
  await answer(v, PANEL_EDITOR('created', ID2), libAction(v));
  const canvas = v.last('tools/call');
  assert.equal(canvas.params.name, 'openpaperdesign_app_canvas');
  await answer(v, { binding: B2, canvas: { name: 'New document' }, revision: '"new"', active_page: 'p2', pages: [{ id: 'p2', name: 'Page 1', text: '{}' }], mount: 8 }, canvas);
  const newHandle = editor.handle;
  assert.equal(await newHandle.started, 'mounted');
  assert.equal(v.els.editor.hidden, false);
  // Local work can now be unsaved in this new editor.
  editor.nextDrain = 'busy';
  await answer(v, { binding: B1, released: true }, release);
  await back;
  const close = libAction(v);
  if (close.params.name === 'openpaperdesign_app_close') await answer(v, { refused: 'The shown canvas is no longer current; nothing was closed.' }, close);
  assert.equal(v.els.editor.hidden, false, 'the new editor and its unfinished work must remain visible');
  assert.equal(newHandle.left, false);
  // Added: no Close for the old binding, the new editor still owns its binding and Home, and
  // its own Home (refused: its work is unfinished) keeps it.
  assert.equal(closes(v).length, 0);
  editor.mountOpts.onHome();
  await flush();
  assert.equal(v.els.editor.hidden, false, 'its own refused Home keeps it');
  assert.equal(newHandle.left, false);
  assert.match(v.els['ed-alert-text'].textContent, /edit is still under way/);
  // Finished, its own Home goes: its release, then exactly its binding is closed.
  editor.nextDrain = 'clean';
  editor.mountOpts.onHome();
  await flush();
  const own = v.last('tools/call');
  assert.deepEqual(own.params, { name: 'openpaperdesign_app_page_release', arguments: { binding: B2, mount: 8 } });
  await answer(v, { binding: B2, released: true }, own);
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B2 } });
  panelOnly(v);
});

test('R25: an explicit discard whose release is delayed past a newer result closes and navigates nothing', async () => {
  const { v, editor } = await realPanelView();
  editor.nextDrain = 'failed';
  await v.els.back.onclick();
  assert.equal(v.els['ed-leave'].hidden, false);
  const leaving = v.els['ed-leave'].onclick();
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  assert.equal(editor.disposed, true, 'the discarded editor is let go at once');
  v.from(result(LIB_EDITOR('opened', ID2)));
  await answer(v, { binding: B1, released: true }, release);
  await leaving;
  await flush();
  assert.equal(closes(v).length, 0);
  assert.equal(v.els.result.hidden, false, 'the newer result stays');
  assert.equal(v.els['result-title'].textContent, 'Notes');
  // Control: the same discard without anything newer closes exactly its binding.
  const w = await realPanelView();
  w.editor.nextDrain = 'failed';
  await w.v.els.back.onclick();
  const discard = w.v.els['ed-leave'].onclick();
  await flush();
  await answer(w.v, { binding: B1, released: true }, w.v.last('tools/call'));
  await discard;
  await flush();
  assert.deepEqual(libAction(w.v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B1 } });
  panelOnly(v);
  panelOnly(w.v);
});

test('R25: a teardown whose release is delayed past a newer editor lets only its own editor go', async () => {
  const { v, editor } = await realPanelView();
  const first = editor.handle;
  v.from({ id: 'td-r25', method: 'ui/resource-teardown', params: {} });
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  // Meanwhile Home and a new file (as in the case above).
  v.from(result({ surface: 'home' }));
  await flush();
  await relist(v);
  v.els['new-file'].onclick();
  await flush();
  await answer(v, PANEL_EDITOR('created', ID2), libAction(v));
  const canvas = v.last('tools/call');
  await answer(v, { binding: B2, canvas: { name: 'New document' }, revision: '"new"', active_page: 'p2', pages: [{ id: 'p2', name: 'Page 1', text: '{}' }], mount: 8 }, canvas);
  const second = editor.handle;
  assert.notEqual(second, first);
  await answer(v, { binding: B1, released: true }, release);
  await flush();
  assert.ok(v.sent.some((m) => m.id === 'td-r25' && m.result), 'the teardown is answered');
  assert.equal(first.left, true);
  assert.equal(second.left, false, 'the newer editor is untouched');
  assert.equal(v.els.editor.hidden, false);
  assert.equal(closes(v).length, 0, 'a teardown closes nothing');
  panelOnly(v);
});

// ---- R26: departure admission belongs to its editor; the reload resolves a refused leave ----
// The first three cases are the R26 review's probes (home-probes.mjs), with their fixtures and
// assertions; `mountReplacement` is theirs.

/** Home, then New file, mounting a second real editor (binding B2, mount 8). */
async function mountReplacement(v, editor) {
  v.from(result({ surface: 'home' }));
  await flush();
  await relist(v);
  v.els['new-file'].onclick();
  await flush();
  await answer(v, PANEL_EDITOR('created', ID2), libAction(v));
  const read = v.last('tools/call');
  assert.equal(read.params.name, 'openpaperdesign_app_canvas');
  await answer(v, { binding: B2, canvas: { name: 'New document' }, revision: '"new"', active_page: 'p2', pages: [{ id: 'p2', name: 'Page 1', text: '{}' }], mount: 8 }, read);
  assert.equal(await editor.handle.started, 'mounted');
  return editor.handle;
}

test('R26 control: teardown drain timeout and release permit the replacement editor to run its own Home', async () => {
  const { v, editor } = await realPanelView();
  const first = editor.handle;
  let finish;
  first.mounted.drain = () => new Promise((r) => { finish = r; });
  v.from({ id: 'td-timeout-control', method: 'ui/resource-teardown', params: {} });
  await flush();
  v.tick(8000);
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  const h = await mountReplacement(v, editor);
  editor.nextDrain = 'busy';
  await answer(v, { binding: B1, released: true }, release);
  await flush();
  const before = v.els['ed-alert-text'].textContent;
  editor.mountOpts.onHome();
  await flush();
  assert.equal(h.left, false);
  assert.notEqual(v.els['ed-alert-text'].textContent, before);
  assert.match(v.els['ed-alert-text'].textContent, /edit is still under way/);
  finish('stale');
  await flush();
});

test('R26: a Home drain superseded by timed-out teardown must not block the replacement editor Home', async () => {
  const { v, editor } = await realPanelView();
  const first = editor.handle;
  let finish;
  const pending = new Promise((r) => { finish = r; });
  // Real Session/saveStep/drain/dispose, with only the editor and delayed transport as stand-ins.
  const oldSession = new Session({ put: () => pending, log: () => {} });
  oldSession.ed = { dirty: () => true, save_snapshot: () => 'old work', save_succeeded: () => {}, save_failed: () => {}, can_reload: () => false, free: () => {} };
  oldSession.state.ready = true;
  oldSession.wasReady = true;
  const save = saveStep(oldSession, { status: editor.opts.onStatus, conflict: () => {} });
  first.mounted.drain = () => oldSession.drain();
  first.mounted.dispose = () => { oldSession.dispose(); editor.disposed = true; };
  const back = v.els.back.onclick();
  await flush();
  v.from({ id: 'td-timeout-after-back', method: 'ui/resource-teardown', params: {} });
  await flush();
  v.tick(8000);
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  const h = await mountReplacement(v, editor);
  editor.nextDrain = 'busy';
  await answer(v, { binding: B1, released: true }, release);
  await flush();
  editor.mountOpts.onHome();
  await flush();
  const blocked = v.els['ed-alert-text'].textContent;
  // Added: the old editor's save is still unanswered, its session disposed, the teardown
  // answered, and the replacement still shown and not let go.
  assert.equal(oldSession.disposed, true);
  assert.equal(oldSession.state.saving, true, 'the old save reply is still pending');
  assert.ok(v.sent.some((m) => m.id === 'td-timeout-after-back' && m.result), 'the teardown was answered');
  assert.equal(v.els.editor.hidden, false);
  assert.equal(h.left, false);
  // Settle only our original Session/save-answer stand-in; the old continuation closes nothing.
  finish({ ok: true, status: 200, revision: '"old-confirmed"' });
  await save;
  await back;
  await flush();
  editor.mountOpts.onHome();
  await flush();
  assert.match(v.els['ed-alert-text'].textContent, /edit is still under way/, 'control: same current Home works once the old drain finishes');
  assert.equal(closes(v).length, 0);
  assert.match(blocked, /edit is still under way/, 'the replacement own Home should not be blocked by the superseded departure');
  // Added: the old continuation left the replacement as it was, and its own Home still goes.
  assert.equal(h.left, false);
  editor.nextDrain = 'clean';
  editor.mountOpts.onHome();
  await flush();
  const own = v.last('tools/call');
  assert.deepEqual(own.params, { name: 'openpaperdesign_app_page_release', arguments: { binding: B2, mount: 8 } });
  await answer(v, { binding: B2, released: true }, own);
  await flush();
  assert.deepEqual(libAction(v).params, { name: 'openpaperdesign_app_close', arguments: { binding: B2 } }, 'unforced, exactly its own binding');
  panelOnly(v);
});

test('R26: an old drain resolving while the replacement\'s own departure is in flight clears only its own state', async () => {
  const { v, editor } = await realPanelView();
  const first = editor.handle;
  // One pending drain, whoever asks (Home, then the teardown).
  let finishOld;
  const oldDrain = new Promise((r) => { finishOld = r; });
  first.mounted.drain = () => oldDrain;
  const back = v.els.back.onclick();
  await flush();
  // The host's teardown times out the old drain and lets that editor go; a new file mounts.
  v.from({ id: 'td-r26-own', method: 'ui/resource-teardown', params: {} });
  await flush();
  v.tick(8000);
  await flush();
  const release = v.last('tools/call');
  assert.equal(release.params.name, 'openpaperdesign_app_page_release');
  const h = await mountReplacement(v, editor);
  await answer(v, { binding: B1, released: true }, release);
  await flush();
  // The replacement's own Home starts and waits on its own drain.
  let finishNew;
  const newDrain = new Promise((r) => { finishNew = r; });
  h.mounted.drain = () => newDrain;
  editor.mountOpts.onHome();
  await flush();
  // The old drain resolves now: it must neither end the replacement's departure nor admit a
  // second one for it.
  finishOld('clean');
  await back;
  await flush();
  editor.mountOpts.onHome();
  await flush();
  assert.equal(h.left, false, 'still draining: a second Home for the same editor waits');
  assert.equal(closes(v).length, 0);
  // Its own drain finishes clean: its release, then exactly its binding.
  finishNew('clean');
  await flush();
  const own = v.last('tools/call');
  assert.deepEqual(own.params, { name: 'openpaperdesign_app_page_release', arguments: { binding: B2, mount: 8 } });
  await answer(v, { binding: B2, released: true }, own);
  await flush();
  assert.deepEqual(closes(v).map((m) => m.params.arguments), [{ binding: B2 }]);
  panelOnly(v);
});

test('R26: a real pending Session conflict drain retains the editor, and successful reload resolves departure feedback', async () => {
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  await openInPanel(v);
  const h = v.fake.handles[0];
  const session = new Session({ put: async () => ({ ok: false, status: 409 }), fetchPage: async () => ({ text: 'disk', revision: '"disk"' }), log: () => {} });
  let pending = 'local edit';
  let loaded = null;
  let banner = false;
  session.ed = {
    dirty: () => pending !== undefined,
    save_snapshot: () => pending,
    save_failed: () => {},
    save_succeeded: () => { pending = undefined; },
    can_reload: () => pending === undefined,
    reload: (t) => { loaded = t; pending = undefined; },
  };
  session.state.ready = true;
  session.wasReady = true;
  h.drain = () => session.drain();
  h.resume = () => session.resume();
  const ui = { status: h.o.onStatus, conflict: () => { banner = true; }, hideConflict: () => { banner = false; } };
  await saveStep(session, ui);
  assert.equal(session.state.conflict, true);
  await v.els.back.onclick();
  assert.equal(v.els.editor.hidden, false);
  assert.equal(v.els['ed-leave'].hidden, false);
  assert.match(v.els['ed-alert-text'].textContent, /last change isn't saved/);
  assert.equal(await discardStep(session, ui), 'discarded');
  assert.equal(session.state.conflict, false);
  assert.equal(loaded, 'disk');
  assert.equal(banner, false);
  assert.equal(session.ed.can_reload(), true);
  assert.equal(session.closing, false);
  // Added: the reload is said, at once and without a Saved.
  assert.equal(v.els['ed-alert-text'].textContent, 'Reloaded the version on disk');
  assert.equal(v.els['ed-leave'].hidden, true, 'no Leave without saving for a clean editor');
  v.tick(6000);
  await flush();
  assert.equal(v.els['ed-leave'].hidden, true, 'successful actual reload resolves the need to leave without saving');
  assert.equal(v.els['ed-alert'].hidden, true, 'a resolved reload note may time out');
  // Added: and a later Home of the now clean editor goes.
  assert.equal(await session.drain(), 'clean');
  session.resume();
  h.nextDrain = 'clean';
  h.drain = async () => 'clean';
  v.els.back.onclick();
  await flush();
  await closedHome(v);
  assert.equal(v.els.home.hidden, false);
});

test('R26 control: a terminal error keeps its Leave without saving through an informational note', async () => {
  const { v, fake } = await panelView();
  await openInPanel(v);
  const h = fake.handles[0];
  h.o.onStatus('Renderer error: device lost', true);
  h.nextDrain = 'error';
  v.els.back.onclick();
  await flush();
  assert.equal(v.els['ed-leave'].hidden, false);
  assert.match(v.els['ed-alert-text'].textContent, /renderer stopped/);
  // A note (a refused copy) and a healthy status change nothing on screen.
  h.o.onStatus('The browser didn\'t allow copying here; use Ctrl/⌘+C on the canvas.');
  h.o.onStatus('Ready');
  assert.equal(v.els['ed-leave'].hidden, false, 'the discard stays offered');
  assert.equal(v.els['ed-alert'].hidden, false);
  assert.equal(v.els['ed-alert'].className, 'ed-alert bad');
  assert.equal(v.els['ed-status'].textContent, 'Ready', 'the live region has every status');
});

test('R26-F2 (embedded): an informational note during an unanswered retry keeps the save failure shown', async () => {
  const v = await libraryView(LIBRARY_LIST, { panel: true, clock: true });
  await openInPanel(v);
  const h = v.fake.handles[0];
  const { session, ui } = realSave(h.o.onStatus, async () => ({ ok: false, status: 500, revision: null }));
  await saveStep(session, ui);
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed; retrying');
  let finish;
  session.deps.put = () => new Promise((resolve) => { finish = resolve; });
  const retry = saveStep(session, ui);
  // What the mount says when the browser refuses a copy (editor.js flushClipboard).
  h.o.onStatus('The browser didn\'t allow copying here; use Ctrl/⌘+C on the canvas.');
  assert.match(v.els['ed-status'].textContent, /didn't allow copying/, 'said in the live region');
  assert.equal(v.els['ed-alert-text'].textContent, 'Save failed; retrying', 'the failure stays on screen');
  assert.equal(v.els['ed-alert'].className, 'ed-alert bad');
  v.runAllTimers();
  await flush();
  assert.equal(v.els['ed-alert'].hidden, false, 'no timer hides it');
  finish({ ok: true, status: 200, revision: '"next"' });
  await retry;
  assert.equal(v.els['ed-alert'].hidden, true, 'the acknowledged save resolves it');
});
