import {test} from 'node:test';
import assert from 'node:assert/strict';
import {detectPlatform,selectAsset,allowedDownload,appDownloadCount,publishedManifest,downloadChoice} from '../../site/downloads.js';
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
  assert.equal(appDownloadCount([{assets:[asset('source.tar.gz',88)]}]),null);
  assert.equal(appDownloadCount([{assets:[asset('OpenPaperDesign-0.1.0-windows-x64.zip',0)]}]),0);
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
  assert.equal(allowedDownload('/downloads/app.zip'),false);
  assert.equal(allowedDownload('/downloads/app.zip',true),true);
});

test('only a complete published three-platform release with checksum assets supplies downloads',()=>{
 const tag='v0.1.0-desktop.1';const base=`https://github.com/arcitai/openpaperdesign/releases/download/${tag}/`;
 const assets=['macos-universal.zip','windows-x64.zip','linux-x64.tar.gz'].flatMap(target=>{
  const name=`OpenPaperDesign-0.1.0-${target}`;
  return [{name,size:500,browser_download_url:base+name},{name:name+'.sha256',size:100,browser_download_url:base+name+'.sha256'}];
 });
 const release={tag_name:tag,assets};
 assert.equal(publishedManifest([release]).platforms.macos.architecture,'Intel + Apple Silicon');
 assert.match(downloadChoice(publishedManifest([release]),'macos').meta,/Apple notarization not verified/);
 assert.match(publishedManifest([release]).notice,/notarization is not verified/);
 assert.equal(publishedManifest([{...release,draft:true}]),null);
 assert.equal(publishedManifest([{...release,tag_name:'candidate-abc'}]),null);
 assert.equal(publishedManifest([{...release,assets:assets.slice(0,-1)}]),null);
 assert.equal(publishedManifest([{...release,assets:assets.map((a,i)=>i===0?{...a,browser_download_url:'https://evil.test/app.zip'}:a)}]),null);
 assert.equal(appDownloadCount([{assets:[{name:'OpenPaperDesign-0.1.0-macos-universal.zip',download_count:4}]}]),4);
});

test('the primary action directly downloads the validated file for the detected desktop OS',()=>{
 const manifest={platforms:Object.fromEntries(['macos','windows','linux'].map(os=>[os,{url:`/downloads/${os}.zip`,architecture:os==='macos'?'Intel + Apple Silicon':'x64',status:'Preview · not notarized'}]))};
 for(const os of ['macos','windows','linux']){
  const choice=downloadChoice(manifest,os,true);
  assert.equal(choice.href,`/downloads/${os}.zip`);
  assert.ok(choice.asset);
  assert.match(choice.label,/^Download for /);
 }
 assert.match(downloadChoice(manifest,'macos',true).meta,/Not Apple notarized/);
 const unknown={platforms:{macos:{...manifest.platforms.macos,status:'Preview'}}};
 assert.match(downloadChoice(unknown,'macos',true).meta,/Apple notarization not verified/);
 unknown.platforms.macos.notarized=true;
 assert.doesNotMatch(downloadChoice(unknown,'macos',true).meta,/notarization not verified/);
 for(const os of ['mobile','unknown']){
  assert.equal(downloadChoice(manifest,os,true).href,'#download');
  assert.equal(downloadChoice(manifest,os,true).asset,null);
 }
 assert.equal(downloadChoice(manifest,'macos',false).asset,null,'local files never become public downloads');
 assert.equal(downloadChoice({platforms:{}},'windows').href,'#download');
});
test('unsupported desktop hardware goes to platform choices rather than an x64 installer',()=>{
 for(const [platform,ua] of [['Linux aarch64','Firefox'],['Linux armv7l','Firefox'],['Win32','Windows NT 10.0; ARM64'],['Linux x86_64','Mozilla/5.0 (X11; CrOS x86_64 1)']])assert.equal(detectPlatform(platform,ua),'unknown');
 assert.equal(detectPlatform('MacIntel','Mozilla Safari',0),'macos','the universal Mac app needs no CPU guess');
});
