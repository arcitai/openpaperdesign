import {test} from 'node:test';
import assert from 'node:assert/strict';
import {detectPlatform,selectAsset,allowedDownload,appDownloadCount} from '../../site/downloads.js';
test('mobile never receives a desktop installer, including iPad desktop mode',()=>{
  assert.equal(detectPlatform('MacIntel','iPhone',1),'mobile');
  assert.equal(detectPlatform('MacIntel','Mozilla Safari',5),'mobile');
  assert.equal(detectPlatform('Linux armv8','Android',5),'mobile');
  assert.equal(selectAsset({platforms:{macos:{url:'/downloads/app.zip',architecture:'arm64',status:'preview'}}},'mobile'),null);
});
test('download totals count app archives across releases, not sources, checksums or drafts',()=>{
  const asset=(name,download_count)=>({name,download_count});
  assert.equal(appDownloadCount([{assets:[asset('OpenPaperDesign-0.1.0-macos-arm64.zip',3),asset('OpenPaperDesign-0.1.0-macos-arm64.zip.sha256',90),asset('source.tar.gz',88)]},{assets:[asset('OpenPaperDesign-0.1.1-linux-x64.tar.gz',5)]},{draft:true,assets:[asset('OpenPaperDesign-0.1.0-windows-x64.zip',50)]}]),8);
  assert.throws(()=>appDownloadCount([{assets:[asset('OpenPaperDesign-0.1.0-macos-arm64.zip',-1)]}]));
  assert.throws(()=>appDownloadCount({error:'rate limit'}));
});
test('desktop OS detection has an honest unknown fallback',()=>{
  assert.equal(detectPlatform('MacIntel','Safari',0),'macos');
  assert.equal(detectPlatform('Win32'),'windows');assert.equal(detectPlatform('Linux x86_64'),'linux');assert.equal(detectPlatform('Other'),'unknown');
});
test('only reviewed release or local-preview assets become download actions',()=>{
  assert.equal(allowedDownload('https://github.com/arcitai/openpaperdesign/releases/download/v1/app.zip'),true);
  for(const u of ['javascript:alert(1)','https://github.com/other/repo/releases/download/v1/a','https://github.com.evil.test/arcitai/openpaperdesign/releases/download/v1/a','/downloads/../secret','https://evil.test/app.zip'])assert.equal(allowedDownload(u),false);
  assert.equal(selectAsset({platforms:{windows:{url:'https://evil.test/app.zip',architecture:'x64',status:'built'}}},'windows'),null);
  assert.equal(selectAsset({},'linux'),null);
});
