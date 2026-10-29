#!/usr/bin/env node
// Optional maintainer tool. The application and CI use the committed PNG/SVG outputs.
// Requires sharp (development only); source geometry remains in assets/brand/*.svg.
const fs=require('node:fs/promises');
const path=require('node:path');
const sharp=require('sharp');
const root=path.resolve(__dirname,'..');
async function render(){
 const light=await fs.readFile(path.join(root,'assets/brand/openpaperdesign.svg'),'utf8');
 const dark=await fs.readFile(path.join(root,'assets/brand/openpaperdesign-dark.svg'),'utf8');
 const homePath=path.join(root,'crates/openpaperdesign-serve/src/home.html');
 const home=require('node:fs').existsSync(homePath)?await fs.readFile(homePath,'utf8'):null;
 // The compact Home consumer is present on the new-UI integration branch.
 if(home && home.includes('<span class="brand-mark"')){
  const body=light.slice(light.indexOf('>')+1,light.lastIndexOf('</svg>')).replace(/<title>.*?<\/title>/,'');
  await fs.writeFile(homePath,home.replace(/<span class="brand-mark"[^>]*>.*?<\/span>/,
    `<span class="brand-mark" aria-hidden="true"><svg viewBox="0 0 64 64">${body}</svg></span>`));
 }
 for(const [theme,source] of [['light',light],['dark',dark]]){
  await sharp(Buffer.from(source)).resize(96,96).png().toFile(path.join(root,`assets/brand/home-logo-${theme}.png`));
 }
 // App icons have an opaque tile; the mark itself is the same inverse artwork.
 const body=dark.slice(dark.indexOf('>')+1,dark.lastIndexOf('</svg>')).replace(/<title>.*?<\/title>/,'');
 const icon=`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><rect x="40" y="40" width="944" height="944" rx="212" fill="#202124"/><g color="#fafafa" transform="translate(115 115) scale(12.4)">${body}</g></svg>`;
 await fs.writeFile(path.join(root,'packaging/icons/icon.svg'),icon+'\n');
 await sharp(Buffer.from(icon)).resize(1024,1024).png().toFile(path.join(root,'packaging/icons/icon-1024.png'));
 for(const [target,source] of [['logo.svg',light],['composer-icon.svg',light],['logo-dark.svg',dark],['composer-icon-dark.svg',dark]]){
  await fs.writeFile(path.join(root,'plugin/openpaperdesign/assets',target),source);
 }
}
render().catch(error=>{console.error(error);process.exitCode=1;});
