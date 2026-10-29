import { detectPlatform, selectAsset, downloadChoice, appDownloadCount, publishedManifest } from './downloads.js?v=351db479d439';
const root=document.documentElement;
const themeButton=document.querySelector('#theme-toggle');
const system=matchMedia('(prefers-color-scheme: dark)');
let preference;
try {preference=localStorage.getItem('openpaperdesign-site-theme');}catch{}
if(!['dark','light'].includes(preference))preference=null;
function applyTheme(){root.dataset.theme=preference||(system.matches?'dark':'light');const next=root.dataset.theme==='dark'?'light':'dark';themeButton.setAttribute('aria-label',`Switch to ${next} mode`);themeButton.title=`Switch to ${next} theme`;}
applyTheme();
themeButton.addEventListener('click',()=>{preference=root.dataset.theme==='dark'?'light':'dark';applyTheme();try{localStorage.setItem('openpaperdesign-site-theme',preference);}catch{}});
system.addEventListener('change',()=>{if(!preference)applyTheme();});
const dialog=document.querySelector('#mobile-navigation');
const menuButton=document.querySelector('#menu-toggle');
let previousOverflow='',navigationTarget=null;
menuButton.addEventListener('click',()=>{previousOverflow=document.body.style.overflow;dialog.showModal();document.body.style.overflow='hidden';menuButton.setAttribute('aria-expanded','true');});
function closeMenu(){if(dialog.open)dialog.close();}
document.querySelector('#menu-close').addEventListener('click',closeMenu);
dialog.addEventListener('close',()=>{document.body.style.overflow=previousOverflow;menuButton.setAttribute('aria-expanded','false');if(navigationTarget){navigationTarget.setAttribute('tabindex','-1');navigationTarget.focus({preventScroll:true});navigationTarget=null;}else if(menuButton.getClientRects().length){menuButton.focus();}});
dialog.querySelectorAll('a').forEach(a=>a.addEventListener('click',()=>{navigationTarget=a.origin===location.origin&&a.pathname===location.pathname&&a.hash?document.getElementById(decodeURIComponent(a.hash.slice(1))):null;closeMenu();}));
matchMedia('(min-width:800.02px)').addEventListener('change',e=>{if(e.matches)closeMenu();});
const offers=document.querySelector('.offers');
document.addEventListener('click',e=>{if(!offers.contains(e.target))offers.open=false;});
document.addEventListener('keydown',e=>{if(e.key==='Escape'&&offers.open){offers.open=false;offers.querySelector('summary').focus();}});
offers.addEventListener('focusout',e=>{if(!offers.contains(e.relatedTarget))offers.open=false;});
const previews={'own-site':['A design study of this website, in OpenPaperDesign. New UI/UX preview.','OpenPaperDesign’s own website, with desktop and mobile layout studies, inside the new workspace.'],architecture:['New UI/UX preview · development build. Real canvas, editable objects.','The new OpenPaperDesign workspace with an editable system diagram.'],design:['Interface studies in the new workspace. Development preview.','Public website and tile studies in the new OpenPaperDesign UI.'],whiteboard:['Six colours of sticky notes and hand-drawn shapes. New UI/UX development preview.','Coloured sticky notes and hand-drawn shapes in the new workspace.'],banner:['A tile banner as an image layer, beside editable vector layers. New UI/UX preview.','The selected gustavonline tile banner in the new OpenPaperDesign workspace.']};
document.querySelectorAll('[data-preview]').forEach(button=>button.addEventListener('click',()=>{const key=button.dataset.preview;if(!previews[key])return;document.querySelectorAll('[data-preview]').forEach(x=>x.setAttribute('aria-pressed',String(x===button)));const image=document.querySelector('#product-screenshot');image.src=`/assets/product-${key}.png`;document.querySelector('#preview-open').href=image.src;image.alt=previews[key][1];document.querySelector('#preview-caption').textContent=previews[key][0];}));
const platform=detectPlatform(navigator.userAgentData?.platform||navigator.platform,navigator.userAgent,navigator.maxTouchPoints);
const local=['127.0.0.1','localhost'].includes(location.hostname);
const platformLabels={macos:['Mac','Intel + Apple Silicon'],windows:['Windows','x64'],linux:['Linux','x64']};
const buildGuide='https://github.com/arcitai/openpaperdesign/blob/328623852a062023348275ecb3c51dea11abf5ad/docs/openpaperdesign/desktop.md#build-from-source';
function choiceForCurrentPlatform(key,manifest){return key===platform&&selectAsset(manifest,platform,local);}
function renderDownloads(manifest){
  const list=document.querySelector('#platform-downloads');list.replaceChildren();
  for(const [key,[name,arch]] of Object.entries(platformLabels)){
    const asset=selectAsset(manifest,key,local);
    const action=document.createElement('a');action.href=asset?asset.url:buildGuide;action.hidden=Boolean(choiceForCurrentPlatform(key,manifest));action.append(document.createTextNode(name));
    const detail=document.createElement('span');detail.textContent=asset?`${asset.architecture} ↓`:`Build from source · ${arch} ↗`;action.append(detail);
    action.setAttribute('aria-label',asset?`Download ${name} preview for ${asset.architecture}`:`Build ${name} from source`);action.title=asset?.status||'No packaged release yet';list.append(action);
  }
  const choice=downloadChoice(manifest,platform,local),primary=document.querySelector('#primary-download');
  primary.textContent=choice.label;primary.href=choice.href;
  const selected=document.querySelector('#download-primary');selected.hidden=!choice.asset;
  if(choice.asset){
    const action=document.querySelector('#desktop-download');action.textContent=choice.label;action.href=choice.href;
    document.querySelector('#download-meta').textContent=choice.meta;
    document.querySelector('#release-note').textContent=choice.meta;
  }else document.querySelector('#release-note').textContent=platform==='mobile'?'For Mac, Windows and Linux. Choose your desktop version below.':'Free and open source. No subscription or account needed.';
  if(typeof manifest.notice==='string')document.querySelector('#download-help').textContent=manifest.notice;
}
async function loadDownloads(){
  let manifest={platforms:{}};
  try{
    const response=await fetch('/releases.json');if(!response.ok)throw new Error('Release manifest unavailable');
    manifest=await response.json();
  }catch{manifest.notice='Release information could not be loaded. Check GitHub releases for the latest downloads.';}
  if(local){
    try{const preview=await fetch('/releases.preview.json');if(preview.ok)manifest=await preview.json();}catch{/* The committed manifest remains the safe fallback. */}
  }
  // Show the same-origin manifest first. A slow/rate-limited GitHub lookup must not
  // delay a known download. Reconcile published assets independently afterwards.
  renderDownloads(manifest);
  if(!local){const published=publishedManifest(await releasesLookup);if(published)renderDownloads(published);}
}
async function publishedReleases(){
  const releases=[];
  for(let page=1;page<=5;page++){
    const response=await fetch(`https://api.github.com/repos/arcitai/openpaperdesign/releases?per_page=100&page=${page}`,{headers:{Accept:'application/vnd.github+json'},signal:AbortSignal.timeout(6000)});
    if(!response.ok)throw new Error('GitHub release metadata unavailable');
    const batch=await response.json();if(!Array.isArray(batch))throw new Error('Invalid release list');
    releases.push(...batch);if(batch.length<100)return releases;
  }
  throw new Error('Release list exceeds the bounded lookup');
}
const releasesLookup=publishedReleases().catch(()=>null);
async function showObservedDownloads(){
  const count=document.querySelector('#download-count');
  const releases=await releasesLookup;
  if(!releases){count.textContent='Download count unavailable';return;}
  const total=appDownloadCount(releases)??0;
  count.textContent=`${total.toLocaleString()} ${total===1?'download':'downloads'}`;
  count.title='App archives downloaded from GitHub releases, including repeat downloads. Source code and local previews are excluded.';
  count.setAttribute('aria-label',`${count.textContent} from GitHub app releases, including repeat downloads`);
}
showObservedDownloads().catch(()=>{document.querySelector('#download-count').textContent='Download count unavailable';});
loadDownloads().catch(()=>{document.querySelector('#download-help').textContent='Release information could not be loaded. Check GitHub releases for the latest downloads.';});
