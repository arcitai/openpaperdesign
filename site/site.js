import { detectPlatform, selectAsset, appDownloadCount } from './downloads.js';
const root=document.documentElement;
const themeButton=document.querySelector('#theme-toggle');
const system=matchMedia('(prefers-color-scheme: dark)');
let preference;
try {preference=localStorage.getItem('openpaperdesign-site-theme');}catch{}
if(!['dark','light'].includes(preference))preference=null;
function applyTheme(theme){root.dataset.theme=theme;themeButton.setAttribute('aria-label',`Switch to ${theme==='dark'?'light':'dark'} theme`);document.querySelector('#theme-label').textContent=theme==='dark'?'Dark':'Light';document.querySelectorAll('img[src$="logo.svg"],img[src$="logo-dark.svg"]').forEach(img=>img.src=theme==='dark'?'/assets/logo-dark.svg':'/assets/logo.svg');}
applyTheme(preference||(system.matches?'dark':'light'));
themeButton.addEventListener('click',()=>{preference=root.dataset.theme==='dark'?'light':'dark';applyTheme(preference);try{localStorage.setItem('openpaperdesign-site-theme',preference);}catch{}});
system.addEventListener('change',()=>{if(!preference)applyTheme(system.matches?'dark':'light');});
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
const previews={'own-site':['This website, designed in OpenPaperDesign. New UI/UX development preview.','OpenPaperDesign’s own website, with desktop and mobile layout studies, inside the new workspace.'],architecture:['New UI/UX preview · development build. Real canvas, editable objects.','The new OpenPaperDesign workspace with an editable system diagram.'],design:['Interface studies in the new workspace. Development preview.','Public website and tile studies in the new OpenPaperDesign UI.'],whiteboard:['Six colours of sticky notes and hand-drawn shapes. New UI/UX development preview.','Coloured sticky notes and hand-drawn shapes in the new workspace.'],banner:['The selected LinkedIn banner as an image layer, beside editable text. New UI/UX preview.','The selected gustavonline tile banner in the new OpenPaperDesign workspace.']};
document.querySelectorAll('[data-preview]').forEach(button=>button.addEventListener('click',()=>{const key=button.dataset.preview;if(!previews[key])return;document.querySelectorAll('[data-preview]').forEach(x=>x.setAttribute('aria-pressed',String(x===button)));const image=document.querySelector('#product-screenshot');image.src=`/assets/product-${key}.png`;document.querySelector('#preview-open').href=image.src;image.alt=previews[key][1];document.querySelector('#preview-caption').textContent=previews[key][0];}));
const platform=detectPlatform(navigator.userAgentData?.platform||navigator.platform,navigator.userAgent,navigator.maxTouchPoints);
const local=['127.0.0.1','localhost'].includes(location.hostname);
const platformLabels={macos:['macOS','Apple Silicon'],windows:['Windows','x64'],linux:['Linux','x64']};
async function loadDownloads(){
  const response=await fetch('/releases.json');if(!response.ok)throw new Error('release manifest unavailable');
  let manifest=await response.json();
  if(local){
    const preview=await fetch('/releases.preview.json');
    if(preview.ok)manifest=await preview.json();
  }
  const list=document.querySelector('#platform-downloads');list.replaceChildren();
  for(const [key,[name,arch]] of Object.entries(platformLabels)){
    const asset=selectAsset(manifest,key,local);const article=document.createElement('article');article.dataset.detected=String(key===platform);
    const title=document.createElement('h3');title.textContent=name;const detail=document.createElement('p');detail.textContent=asset?.architecture||arch;
    const state=document.createElement('small');state.textContent=asset?.status||'No packaged release yet';
    const action=document.createElement('a');action.className=asset?'button':'text-link';action.textContent=asset?'Download preview ↓':'Build from source ↗';action.setAttribute('aria-label',asset?`Download ${name} preview for ${asset.architecture}`:`Build ${name} from source`);action.href=asset?asset.url:'https://github.com/arcitai/openpaperdesign#build-from-source';article.append(title,detail,state,action);if(key===platform){const detected=document.createElement('small');detected.textContent='Your operating system · check architecture';article.append(detected);}list.append(article);
  }
  const chosen=selectAsset(manifest,platform,local);const primary=document.querySelector('#primary-download');
  // User agents do not reliably distinguish Intel and Apple Silicon. Let the person
  // choose the explicitly labelled architecture instead of guessing an installer.
  if(platformLabels[platform]){primary.textContent=`Download free for ${platformLabels[platform][0]} ↓`;primary.href='#download';}
  else if(platform==='mobile'){primary.textContent='Available for desktop ↓';document.querySelector('#release-note').textContent='A desktop app for macOS, Windows and Linux. Choose a platform below.';}
  if(typeof manifest.notice==='string')document.querySelector('#download-help').textContent=manifest.notice;
}
loadDownloads().catch(()=>{document.querySelector('#download-help').textContent='Release information could not be loaded. Check GitHub releases for the latest downloads.';});
async function showObservedDownloads(){
  const releases=[];
  for(let page=1;page<=5;page++){
    const response=await fetch(`https://api.github.com/repos/arcitai/openpaperdesign/releases?per_page=100&page=${page}`,{headers:{Accept:'application/vnd.github+json'}});
    if(!response.ok)throw new Error('GitHub download totals unavailable');
    const batch=await response.json();if(!Array.isArray(batch))throw new Error('Invalid GitHub release list');
    releases.push(...batch);
    if(batch.length<100){
      const total=appDownloadCount(releases);if(total===null)return;
      const count=document.querySelector('#download-count');count.textContent=`${total.toLocaleString()} downloads from GitHub releases · includes repeat downloads`;
      count.hidden=false;return;
    }
  }
  throw new Error('Release list exceeds the bounded count lookup');
}
showObservedDownloads().catch(()=>{/* An unavailable number stays hidden, never estimated. */});
