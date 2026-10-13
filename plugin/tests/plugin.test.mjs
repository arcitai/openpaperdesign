// The plugin's manifests, MCP entry, skills and the repository marketplace (no network, no
// dependencies). Run: node --test plugin/tests/plugin.test.mjs
//
// Source-only checks: they read the files in this repository. Whether Codex discovers and
// installs the plugin, and starts `openpaperdesign-mcp` (command on PATH, `cwd` handling,
// openai/codex#22842), is only proven by installing it in Codex.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, statSync, readdirSync, realpathSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const plugin = join(repo, 'plugin/openpaperdesign');
const json = (p) => JSON.parse(readFileSync(p, 'utf8'));
/** A plugin-relative path: must start with ./ and stay inside `root`. */
const inside = (root, p) => {
  assert.ok(typeof p === 'string' && p.startsWith('./'), `path ${p} must start with ./`);
  const abs = resolve(root, p);
  assert.ok(abs === root || abs.startsWith(`${root}/`), `${p} leaves ${root}`);
  return abs;
};

/** Every string value starting with ./ anywhere in `v`. */
const dotPaths = (v) =>
  typeof v === 'string'
    ? v.startsWith('./') ? [v] : []
    : v && typeof v === 'object'
      ? Object.values(v).flatMap(dotPaths)
      : [];

test('plugin.json has the required fields and its paths exist', () => {
  const m = json(join(plugin, 'plugin.json'));
  assert.equal(m.$schema, 'https://agent-plugins.org/schemas/1.0.0/plugin.schema.json');
  assert.equal(m.name, 'openpaperdesign');
  assert.match(m.name, /^[a-z0-9]+(-[a-z0-9]+)*$/, 'kebab-case');
  assert.match(m.version, /^\d+\.\d+\.\d+$/);
  assert.equal(m.license, 'MIT OR Apache-2.0');
  assert.equal(m.repository, 'https://github.com/onlinesourdough/openpaperdesign');
  assert.ok(m.description && m.author && m.author.name);
  // Discovery is fixed (the skills/ folder and the root mcp.json): the portable manifest
  // must not declare components itself.
  for (const k of ['skills', 'mcpServers', 'apps', 'interface']) {
    assert.ok(!(k in m), `no top-level ${k} in the portable plugin.json`);
  }
  assert.ok(statSync(join(plugin, 'skills')).isDirectory());
  const ext = m.extensions['com.openai'];
  assert.ok(ext.interface.displayName && ext.interface.shortDescription);
  assert.ok(
    [...ext.interface.shortDescription].length <= 30,
    `shortDescription is ${[...ext.interface.shortDescription].length} characters (max 30)`,
  );
  assert.ok(existsSync(join(inside(plugin, ext.onboardingSkill), 'SKILL.md')));
  assert.ok(!/paper\.design/.test(m.name), 'no Paper branding');
});

test('every ./ path in the manifests exists inside its root', () => {
  const files = [
    [plugin, join(plugin, 'plugin.json')],
    [plugin, join(plugin, 'mcp.json')],
    [repo, join(repo, '.agents/plugins/marketplace.json')],
  ];
  let n = 0;
  for (const [root, f] of files) {
    for (const p of dotPaths(json(f))) {
      assert.ok(existsSync(inside(root, p)), `${f}: ${p} is missing`);
      n += 1;
    }
  }
  assert.ok(n >= 2, 'the onboarding skill and the marketplace path were checked');
});

test('one manifest: no .codex-plugin overlay to contradict plugin.json', () => {
  assert.ok(!existsSync(join(plugin, '.codex-plugin')));
});

test('mcp.json starts the stdio launcher from PATH', () => {
  assert.ok(statSync(join(plugin, 'mcp.json')).isFile());
  const m = json(join(plugin, 'mcp.json'));
  assert.deepEqual(Object.keys(m.mcpServers), ['openpaperdesign']);
  assert.equal(m.$schema, 'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json');
  const s = m.mcpServers.openpaperdesign;
  assert.equal(s.type, 'stdio', 'an explicit stdio server (no url)');
  assert.ok(!('url' in s));
  assert.equal(s.command, 'openpaperdesign-mcp');
  assert.ok(!s.command.includes('/'), 'a command on PATH, never a bundled relative path');
  assert.deepEqual(s.args, ['--codex-context']);
  // Codex 0.162.1 starts plugin servers in the plugin folder. Workspace authority
  // comes only from the experimental metadata on actual model tool calls.
  assert.ok(!('cwd' in s));
  assert.ok(!JSON.stringify(m).includes('token'), 'no token in config');
});

test('every skill has matching frontmatter', () => {
  const skills = join(plugin, 'skills');
  const names = readdirSync(skills).sort();
  assert.deepEqual(names, ['design', 'openpaperdesign', 'review-design', 'setup']);
  for (const n of names) {
    const s = readFileSync(join(skills, n, 'SKILL.md'), 'utf8');
    const fm = s.match(/^---\n([\s\S]*?)\n---\n/);
    assert.ok(fm, `${n}: frontmatter`);
    assert.match(fm[1], new RegExp(`^name: ${n}$`, 'm'));
    assert.match(fm[1], /^description: .{20,}$/m);
    assert.ok(Buffer.byteLength(s) < 8192, `${n}: short`);
    assert.ok(!/OpenPencil workbench|AIOS:/.test(s), `${n}: no AIOS/OpenPencil coupling`);
  }
});

test('the repository marketplace points at the plugin', () => {
  const m = json(join(repo, '.agents/plugins/marketplace.json'));
  assert.equal(m.name, 'openpaperdesign-local');
  assert.ok(m.interface.displayName);
  assert.equal(m.plugins.length, 1);
  const p = m.plugins[0];
  assert.equal(p.name, 'openpaperdesign');
  assert.equal(p.source.source, 'local');
  const dir = inside(repo, p.source.path);
  assert.equal(dir, plugin);
  // Also after resolving links: the path really is this folder, inside the repository.
  const real = realpathSync(dir);
  assert.equal(real, realpathSync(plugin));
  assert.ok(real.startsWith(`${realpathSync(repo)}/`));
  assert.equal(json(join(dir, 'plugin.json')).name, p.name);
  assert.ok(p.policy.installation && p.policy.authentication && p.category);
});

test('notices and licenses are present', () => {
  const notice = readFileSync(join(plugin, 'NOTICE.md'), 'utf8');
  for (const s of ['a14f2857d8e229e7d3ce2c9beb15b876094e6d85', 'MIT', 'paper-rs', 'OFL']) {
    assert.ok(notice.includes(s), s);
  }
  for (const n of ['design', 'review-design']) {
    assert.match(readFileSync(join(plugin, 'skills', n, 'LICENSE'), 'utf8'), /MIT License/);
  }
});
