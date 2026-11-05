import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
const read=name=>readFileSync(new URL(`../../site/${name}`,import.meta.url),'utf8');
const html=read('index.html'),script=read('site.js'),manifest=JSON.parse(read('releases.json'));
const text=html.replace(/<script[\s\S]*?<\/script>/g,' ').replace(/<[^>]+>/g,' ').replace(/&amp;/g,'&').replace(/\s+/g,' ');
const GALLERY=['/assets/product-plugin-design.png','/assets/product-plugin-diagram.png','/assets/product-plugin-home.png'];

test('the active page offers no standalone download, counter or release-API call',()=>{
  assert.doesNotMatch(script,/downloads\.js|fetch\(|api\.github\.com|XMLHttpRequest|sendBeacon/);
  assert.doesNotMatch(html,/download-count|desktop-download|platform-downloads|id="download"|href="#download"|releases\/download/);
  assert.doesNotMatch(html,/api\.github\.com|releases\.json|releases\.preview\.json/);
  assert.doesNotMatch(text,/desktop app for|Check releases|Choose your version|Download for|Intel \+ Apple Silicon|\bwindows\b.*\bx64\b/i);
  assert.deepEqual(manifest.platforms,{});
  assert.equal(manifest.downloads,null);
  assert.match(manifest.notice,/retired/);
});

test('install section states the real requirements and boundaries',()=>{
  for(const phrase of [/Node\.js 22 or later/,/outside the plugin cache|never inside the plugin cache/,/manual snapshot/,/not live sync/,/optional/i,/earlier standalone app are preserved/,/different format/,/not converted automatically/,/not a standalone desktop app/,/not offered yet/])assert.match(text,phrase);
  assert.match(html,/id="install"/);
  assert.equal([...html.matchAll(/href="#install"/g)].length,3);
});

test('install commands match the successor README when it is checked out beside this repository',t=>{
  const readme=new URL('../../../openpaperdesign/README.md',import.meta.url);
  const expected=['codex plugin marketplace add gustavonline/openpaperdesign','codex plugin add openpaperdesign@openpaperdesign','/plugin marketplace add gustavonline/openpaperdesign','/plugin install openpaperdesign@openpaperdesign','/openpaperdesign:home'];
  for(const command of expected)assert.ok(html.includes(command),command);
  // GitHub's owner/repo checkout layout can make this sibling path alias the legacy checkout itself.
  const manifest=new URL('./package.json',readme);
  if(!existsSync(readme)||!existsSync(manifest))return t.skip('verified sibling successor checkout not present');
  const identity=JSON.parse(readFileSync(manifest,'utf8'));
  if(identity.repository?.url!=='https://github.com/gustavonline/openpaperdesign.git')return t.skip('sibling is not the lightweight successor');
  const source=readFileSync(readme,'utf8');
  for(const command of expected)assert.ok(source.includes(command),`successor README no longer contains: ${command}`);
});

test('project links point at the successor and never imply a public plugin archive',()=>{
  const links=[...html.matchAll(/href="(https:\/\/github\.com\/[^"]+)"/g)].map(m=>m[1]);
  assert.ok(links.includes('https://github.com/gustavonline/openpaperdesign#install'));
  assert.ok(links.includes('https://github.com/gustavonline/openpaperdesign/releases'));
  assert.ok(!links.some(u=>u.startsWith('https://github.com/arcitai/openpaperdesign/releases')||u.includes('/blob/328623852a')));
  assert.ok(links.includes('https://github.com/arcitai/openpaperdesign'),'old repository stays reachable as history');
  assert.doesNotMatch(html,/\.zip"|\.tar\.gz"|\.dmg"/);
});

test('the gallery uses exactly the three browser/plugin development previews',()=>{
  const used=new Set([...html.matchAll(/\/assets\/(product-[A-Za-z0-9-]+\.png)/g)].map(m=>`/assets/${m[1]}`));
  assert.deepEqual([...used].sort(),GALLERY);
  const tabs=[...html.matchAll(/data-preview="([a-z]+)"/g)].map(m=>m[1]);
  assert.deepEqual(tabs,['design','diagram','home']);
  assert.deepEqual(tabs.map(k=>`/assets/product-plugin-${k}.png`).sort(),GALLERY);
  for(const key of tabs)assert.match(script,new RegExp(`${key}:\\['`));
  assert.match(script,/product-plugin-\$\{key\}\.png/);
  assert.match(text,/browser\/plugin development previews/);
  assert.match(text,/not captures of the earlier native app and have not been release-qualified/);
  const alts=[...html.matchAll(/<img[^>]*src="\/assets\/product-[^>]*>/g)].map(m=>m[0]);
  assert.equal(alts.length,4);
  for(const img of alts)assert.match(img,/alt="[^"]*[Dd]evelopment preview\.?"/);
});

test('unsupported feature claims stay off the page',()=>{
  assert.doesNotMatch(text,/Excalidraw|draw\.io|\bSVG\b|\bPDF\b|sticky|whiteboard|Windows|Linux/i);
  assert.doesNotMatch(text,/(?<!not )(?:automatically|automatic) (?:converted|migrated|migration)/i);
});

test('theme, navigation and family links of the established layout remain',()=>{
  for(const id of ['theme-toggle','menu-toggle','menu-close','mobile-navigation','preview-open','product-screenshot','preview-caption'])assert.match(html,new RegExp(`id="${id}"`));
  for(const href of ['https://arcitai.com/','https://onlinesourdough.com/','https://onlinesourdough.com/fermentary/','https://gustavonline.com/'])assert.ok(html.includes(`href="${href}"`),href);
  assert.match(html,/class="family-links"/);
  assert.match(script,/openpaperdesign-site-theme/);
  assert.match(html,/Independent of Paper \(paper\.design\)/);
});

test('the transition note gives every legacy issue exactly one outcome',t=>{
  const doc=readFileSync(new URL('../../docs/openpaperdesign/lightweight-transition.md',import.meta.url),'utf8');
  const rows=[...doc.matchAll(/^\| (\d+) \|.*\| (Source-supported|Superseded|Gap) \|/gm)].map(m=>[Number(m[1]),m[2]]);
  assert.equal(rows.length,33);
  assert.equal(new Set(rows.map(r=>r[0])).size,33);
  const inventory=new URL('../../../legacy-open-issues.json',import.meta.url);
  if(existsSync(inventory))assert.deepEqual(rows.map(r=>r[0]).sort((a,b)=>a-b),JSON.parse(readFileSync(inventory,'utf8')).map(i=>i.number).sort((a,b)=>a-b));
  else t.diagnostic('legacy-open-issues.json not present; checked row count only');
  assert.match(doc,/\*not\* native acceptance/);
  assert.match(doc,/closes nothing/);
});
