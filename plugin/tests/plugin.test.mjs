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
import { createHash } from 'node:crypto';

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
  assert.equal(m.repository, 'https://github.com/arcitai/openpaperdesign');
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

test('interface icons are contained, square, static SVGs of at least 48 px', () => {
  // plugin-creator 0.1.26 packaging reference (read by the operator): logo and composerIcon
  // (dark variants optional) are contained relative paths to square PNG/JPEG/WebP/SVG files
  // of at least 48x48 and at most 5 MiB. Whether Codex shows them is only proven installed.
  // The listing and composer icons are the owner-selected mark, which
  // scripts/render-brand-assets.cjs copies from assets/brand/: a square 64-unit viewBox
  // without width/height, so their size is the viewBox. The launcher's icon.svg and
  // icon-dark.svg keep 48x48 on a 24-unit viewBox.
  const svgAt = (rel, f) => {
    const abs = inside(plugin, rel);
    assert.match(rel, /^\.\/assets\/[a-z-]+\.svg$/, f);
    assert.ok(statSync(abs).size <= 5 * 1024 * 1024, `${f}: at most 5 MiB`);
    const svg = readFileSync(abs, 'utf8');
    assert.match(svg, /^<svg xmlns="http:\/\/www\.w3\.org\/2000\/svg" /, `${f}: one root svg`);
    // The root element's own size: a square viewBox, and a square, unitless width/height if
    // it has one (a unit or percentage would be an intrinsic size this check can't compare).
    const root = svg.slice(0, svg.indexOf('>') + 1);
    const attr = (k) => (root.match(new RegExp(` ${k}="([^"]*)"`)) || [])[1];
    const box = root.match(/ viewBox="0 0 ([0-9.]+) ([0-9.]+)"/);
    assert.ok(box && box[1] === box[2], `${f}: square viewBox`);
    const [w, h] = [attr('width'), attr('height')];
    assert.ok(w === h && (w === undefined || /^[0-9.]+$/.test(w)) && Number(w ?? box[1]) >= 48,
      `${f}: square, >= 48`);
    // Static and self-contained: nothing that runs, styles, loads or embeds.
    for (const banned of ['<script', 'foreignObject', 'href', 'url(', '<style', 'style=', '<text',
      '<image', '@import', 'on', '<!']) {
      if (banned === 'on') assert.doesNotMatch(svg, /\son[a-z]+=/i, `${f}: no event handler`);
      else assert.ok(!svg.includes(banned), `${f}: no ${banned}`);
    }
    assert.deepEqual(svg.match(/https?:\/\/[^"]*/g), ['http://www.w3.org/2000/svg'], `${f}: no external reference`);
    return { svg, box: Number(box[1]), width: w && Number(w) };
  };
  const ui = json(join(plugin, 'plugin.json')).extensions['com.openai'].interface;
  const fields = ['logo', 'logoDark', 'composerIcon', 'composerIconDark'];
  const icons = Object.fromEntries(fields.map((f) => [f, svgAt(ui[f], f)]));
  const paths = new Set(fields.map((f) => ui[f]));
  assert.equal(paths.size, 4, 'distinct light/dark logo and composer icon');
  const brand = (n) => readFileSync(join(repo, 'assets/brand', n), 'utf8');
  // One brand source each: the listing logo and the composer icon are the same mark.
  for (const [f, n] of [['logo', 'openpaperdesign.svg'], ['composerIcon', 'openpaperdesign.svg'],
    ['logoDark', 'openpaperdesign-dark.svg'], ['composerIconDark', 'openpaperdesign-dark.svg']]) {
    assert.equal(icons[f].svg, brand(n), `${f}: assets/brand/${n}`);
  }
  // The launcher embeds these two (launcher.rs ICON_LIGHT, ICON_DARK). They still carry the
  // issue #8 mark (48x48 on a 24-unit viewBox), not the selected one: an open follow-up.
  for (const n of ['icon.svg', 'icon-dark.svg']) {
    const i = svgAt(`./assets/${n}`, n);
    assert.ok(i.box === 24 && i.width === 48, `${n}: 48x48 on a 24-unit viewBox`);
  }
  // Sandboxed sessions can leave empty dot placeholders (see AGENTS.md); they aren't assets.
  const assets = readdirSync(join(plugin, 'assets')).filter((n) => !n.startsWith('.')).sort();
  assert.deepEqual(assets, ['composer-icon-dark.svg', 'composer-icon.svg', 'icon-dark.svg',
    'icon.svg', 'logo-dark.svg', 'logo.svg']);
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
  // The packaged server is the app library, in the plugin's host-configured data folder:
  // Codex 0.162.1's Agent Plugin MCP loader expands the literal ${PLUGIN_DATA} placeholder in
  // stdio arguments (no shell). No working directory, environment override or tool argument
  // names the folder. Workspace projects use a separately started --codex-context server.
  assert.deepEqual(s.args, ['--app-library', '${PLUGIN_DATA}']);
  assert.ok(!('cwd' in s));
  assert.ok(!('env' in s), 'no environment override');
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

test('the home view font is the unmodified Geist Pixel Square with its license and notice', () => {
  const assets = join(repo, 'crates/openpaperdesign-serve/assets');
  const sha = (p) => createHash('sha256').update(readFileSync(join(assets, p))).digest('hex');
  assert.equal(sha('geist-pixel-square.woff2'), '04f9cf917a824370c77fecbd743c99d0b3dfd2c6c906de959996ac6f7fb343b3');
  assert.equal(sha('geist-OFL.txt'), '942560b236adfa83745b2c64e5fc09ebaf91cb331751b1157eb92187e5d6e930');
  assert.deepEqual(readFileSync(join(assets, 'geist-pixel-square.woff2')).subarray(0, 4), Buffer.from('wOF2'));
  const license = readFileSync(join(assets, 'geist-OFL.txt'), 'utf8');
  assert.match(license, /Copyright 2024 The Geist Project Authors/);
  assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/);
  const notice = readFileSync(join(plugin, 'NOTICE.md'), 'utf8');
  assert.match(notice, /Geist Pixel Square/);
  assert.match(readFileSync(join(repo, 'THIRD_PARTY.md'), 'utf8'), /Geist Pixel Square/);
  // Served from the binary as a data URI: the view names no font service or font URL.
  const home = readFileSync(join(repo, 'crates/openpaperdesign-serve/src/home.html'), 'utf8');
  assert.ok(!/fonts\.(googleapis|gstatic)|vercel\.com|@import/.test(home));
  assert.match(home, /url\("data:font\/woff2;base64,__OPD_WORDMARK_FONT__"\)/);
});
