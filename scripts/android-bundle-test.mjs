import {chromium} from '@playwright/test';
import {readFile,mkdtemp,rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {resolve,extname} from 'node:path';
const directory=await mkdtemp(resolve('verification/android-apk-'));
execFileSync('unzip',['-q',process.argv[2]??'mobile-android/app/build/outputs/apk/debug/app-debug.apk','assets/*','-d',directory]);
const browser=await chromium.launch({channel:'chrome',headless:true});
try {
const page=await browser.newPage({viewport:{width:412,height:915}});
page.on('pageerror',e=>console.log('PAGE ERROR',e.message));
page.on('console',m=>console.log('CONSOLE',m.type(),m.text()));
await page.route('https://appassets.androidplatform.net/**',async route=>{
 const path=new URL(route.request().url()).pathname;
 const file=resolve(directory,'assets',path==='/mobile'?'index.html':'.'+path);
 try {await route.fulfill({body:await readFile(file),contentType:({'.html':'text/html','.js':'text/javascript','.css':'text/css','.woff2':'font/woff2','.json':'application/json'})[extname(file)]??'application/octet-stream'});} catch(e){console.log('MISSING',path);await route.fulfill({status:404,body:'Not found'});}
});
await page.goto('https://appassets.androidplatform.net/mobile');
await page.getByRole('button',{name:'Connect to desktop',exact:true}).waitFor({state:'visible',timeout:10000});
assert.equal(await page.evaluate(()=>window.isSecureContext && !!crypto.subtle),true);
console.log('APK bundled mobile interface rendered in Chromium with secure WebCrypto.');
await page.screenshot({path:'verification/android-production.png'});
} finally { await browser.close(); await rm(directory,{recursive:true,force:true}); }
