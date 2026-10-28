export function detectPlatform(platform='',ua='',touch=0){
  if(/iPhone|iPad|iPod|Android/i.test(ua)||(/Mac/i.test(platform)&&touch>1))return 'mobile';
  if(/CrOS/i.test(ua)||(/Linux|Win/i.test(platform)&&/aarch64|arm(?:64|v[5-9])/i.test(platform+' '+ua)))return 'unknown';
  if(/Win/i.test(platform))return 'windows';if(/Mac/i.test(platform))return 'macos';if(/Linux/i.test(platform))return 'linux';return 'unknown';
}
export function allowedDownload(url,local=false){
  if(typeof url!=='string')return false;
  if(local&&/^\/downloads\/[A-Za-z0-9_.-]+$/.test(url))return true;
  try{const u=new URL(url);return u.protocol==='https:'&&u.hostname==='github.com'&&u.pathname.startsWith('/arcitai/openpaperdesign/releases/download/')&&!u.username&&!u.password;}catch{return false;}
}
export function selectAsset(manifest,platform,local=false){
  if(!['macos','windows','linux'].includes(platform))return null;
  const asset=manifest?.platforms?.[platform];return asset&&allowedDownload(asset.url,local)&&typeof asset.architecture==='string'&&typeof asset.status==='string'?asset:null;
}
// A known, validated asset is the primary action itself, not an anchor to a
// second download chooser. Mobile, unsupported CPUs and missing builds stay safe.
export function downloadChoice(manifest,platform,local=false){
  const asset=selectAsset(manifest,platform,local);
  if(!asset)return {asset:null,label:platform==='mobile'?'Choose your desktop version ↓':'Choose your version ↓',href:'#download',meta:''};
  const name={macos:'Mac',windows:'Windows',linux:'Linux'}[platform];
  const caution=platform!=='macos'?'':/not(?: Apple)? notarized/i.test(asset.status)?' · Not Apple notarized':asset.notarized===true?'':' · Apple notarization not verified';
  return {asset,label:`Download for ${name} ↓`,href:asset.url,meta:`Preview · ${asset.architecture}${caution}`};
}
export function appDownloadCount(releases){
  if(!Array.isArray(releases))throw new Error('Invalid release list');
  let count=0,matched=0;
  for(const release of releases){
    if(release.draft)continue;
    for(const asset of release.assets||[]){
      if(!/^OpenPaperDesign-[0-9][A-Za-z0-9.-]*-(?:macos-(?:arm64|x64|universal)|windows-x64|linux-x64)\.(?:zip|dmg|tar\.gz)$/.test(asset.name))continue;
      if(!Number.isSafeInteger(asset.download_count)||asset.download_count<0)throw new Error('Invalid download count');
      count+=asset.download_count;
      matched++;
      if(!Number.isSafeInteger(count))throw new Error('Download count overflow');
    }
  }
  return matched?count:null;
}
// GitHub already supplies the published asset metadata. A draft, source-only release,
// candidate branch or incomplete platform set must never become an install button.
export function publishedManifest(releases){
  if(!Array.isArray(releases))return null;
  for(const release of releases){
    if(release.draft||!/^v[0-9][A-Za-z0-9.-]*$/.test(release.tag_name||''))continue;
    const assets=release.assets||[], platforms={};let version=null,invalid=false;
    for(const [platform,target,extension] of [['macos','macos-universal','zip'],['windows','windows-x64','zip'],['linux','linux-x64','tar.gz']]){
      const candidates=assets.filter(a=>typeof a.name==='string'&&a.name.startsWith('OpenPaperDesign-')&&a.name.endsWith(`-${target}.${extension}`));
      if(candidates.length!==1){invalid=true;break;}
      const asset=candidates[0],current=asset.name.slice(16,-(`-${target}.${extension}`.length));
      if(!/^[0-9][A-Za-z0-9.-]*$/.test(current)||(version&&current!==version)||!(release.tag_name===`v${current}`||release.tag_name.startsWith(`v${current}-`))||!Number.isSafeInteger(asset.size)||asset.size<=0||!allowedDownload(asset.browser_download_url)||!assets.some(a=>a.name===asset.name+'.sha256'&&a.size>0&&allowedDownload(a.browser_download_url))){invalid=true;break;}
      version=current;
      platforms[platform]={url:asset.browser_download_url,architecture:platform==='macos'?'Intel + Apple Silicon':'x64',status:platform==='macos'?'Desktop preview · Apple notarization not verified':'Desktop preview · see release notes for test status'};
    }
    if(!invalid)return {version,platforms,notice:'Desktop preview. Apple notarization is not verified by GitHub asset metadata. Check the release notes for signing, installation and test status, and verify the archive checksum.'};
  }
  return null;
}
