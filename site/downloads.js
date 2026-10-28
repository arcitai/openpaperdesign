export function detectPlatform(platform='',ua='',touch=0){
  if(/iPhone|iPad|iPod|Android/i.test(ua)||(/Mac/i.test(platform)&&touch>1))return 'mobile';
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
export function appDownloadCount(releases){
  if(!Array.isArray(releases))throw new Error('Invalid release list');
  let count=0,matched=0;
  for(const release of releases){
    if(release.draft)continue;
    for(const asset of release.assets||[]){
      if(!/^OpenPaperDesign-[0-9][A-Za-z0-9.-]*-(?:macos-(?:arm64|x64)|windows-x64|linux-x64)\.(?:zip|dmg|tar\.gz)$/.test(asset.name))continue;
      if(!Number.isSafeInteger(asset.download_count)||asset.download_count<0)throw new Error('Invalid download count');
      count+=asset.download_count;
      matched++;
      if(!Number.isSafeInteger(count))throw new Error('Download count overflow');
    }
  }
  return matched?count:null;
}
