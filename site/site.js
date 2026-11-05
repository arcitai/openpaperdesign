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
const previews={design:['A design in the plugin editor. Actual browser/plugin development preview.','A design open in the OpenPaperDesign plugin editor in the browser. Development preview.'],diagram:['A diagram built from Library symbols. Actual browser/plugin development preview.','An editable diagram of Library symbols joined by connectors in the plugin editor. Development preview.'],home:['Home lists saved Papers. Actual browser/plugin development preview.','OpenPaperDesign Home listing saved Papers in the plugin. Development preview.']};
document.querySelectorAll('[data-preview]').forEach(button=>button.addEventListener('click',()=>{const key=button.dataset.preview;if(!previews[key])return;document.querySelectorAll('[data-preview]').forEach(x=>x.setAttribute('aria-pressed',String(x===button)));const image=document.querySelector('#product-screenshot');image.src=`/assets/product-plugin-${key}.png`;document.querySelector('#preview-open').href=image.src;image.alt=previews[key][1];document.querySelector('#preview-caption').textContent=previews[key][0];}));
