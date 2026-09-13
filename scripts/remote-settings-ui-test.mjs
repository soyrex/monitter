// Focused browser harness: compiles only SettingsPane + RemoteControl and their
// direct dependencies. It deliberately does not import AppSurface or start Vite.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium, expect as baseExpect } from '@playwright/test';
const expect=baseExpect.configure({timeout:15000}), root=process.cwd();
const modules=existsSync(join(root,'node_modules'))?join(root,'node_modules'):resolve(root,'../monitter/node_modules');
const temp=await mkdtemp(join(tmpdir(),'monitter-remote-settings-')); let relay,server,browser;
function start(args,pattern){const child=spawn(process.execPath,args,{cwd:root,env:{...process.env,MONITTER_RELAY_PORT:'0',MONITTER_RELAY_HOST:'127.0.0.1'},stdio:['ignore','pipe','pipe']});relay=child;return new Promise((ok,bad)=>{let text='';const timer=setTimeout(()=>bad(Error('Test relay startup timed out.')),15000);const read=b=>{text+=b;const m=text.match(pattern);if(m){clearTimeout(timer);ok([child,m[1]]);}};child.stdout.on('data',read);child.stderr.on('data',read);child.once('error',reason=>{clearTimeout(timer);bad(reason);});child.once('exit',code=>{clearTimeout(timer);bad(Error(`child exited ${code}: ${text}`));});});}
try {
  const config=join(temp,'rolldown.config.mjs'), bundle=join(temp,'bundle.js');
  await writeFile(config,`import { existsSync } from 'node:fs'; import { resolve } from 'node:path'; import compiler from ${JSON.stringify(join(modules,'svelte/compiler/index.js'))}; const root=${JSON.stringify(root)}; const fixture=resolve(root,'scripts/fixtures'); export default {input:resolve(fixture,'remote-settings-entry.js'),output:{file:${JSON.stringify(bundle)},format:'es'},plugins:[{name:'svelte-focused',resolveId(id){if(id==='@lucide/svelte')return resolve(fixture,'remote-settings-lucide-stub.js');if(id.startsWith('$lib/')){const file=resolve(root,'src/lib',id.slice(5));return existsSync(file)?file:file+'.ts';}},transform(code,id){if(id.endsWith('.svelte'))return {code:compiler.compile(code,{filename:id,generate:'client',css:'injected'}).js.code,map:null};}}]};`);
  execFileSync(join(modules,'.bin/rolldown'),['--config',config,'--platform','browser'],{cwd:root,stdio:'inherit'});
  const html='<!doctype html><div id="app"></div>';
  server=createServer(async(req,res)=>{const path=new URL(req.url,'http://x').pathname;if(path==='/bundle.js'){res.setHeader('content-type','text/javascript');res.end(await readFile(bundle));}else {res.setHeader('content-type','text/html');res.end(html);}}); await new Promise(ok=>server.listen(0,'127.0.0.1',ok)); const base=`http://127.0.0.1:${server.address().port}`;
  const relayResult=await start(['scripts/relay-server.mjs'],/listening on (ws:\/\/[^\s]+)/); relay=relayResult[0]; const relayUrl=relayResult[1];
  browser=await chromium.launch(); const context=await browser.newContext({viewport:{width:407,height:900}});
  await context.addInitScript(()=>{window.__MONITTER_TEST_BRIDGE__={invoke:async command=>command==='get_snapshot'?{hosts:[],agents:[],tasks:[],messages:[],events:[],channels:[],projects:[],collaborations:[],queuedMessages:[],settings:{}}:[]};const Original=window.WebSocket;window.__socketCount=0;window.WebSocket=class extends Original{constructor(...args){super(...args);window.__socketCount++;}};}); const desktop=await context.newPage(), phone=await context.newPage();
  const errors=[];desktop.on('pageerror',e=>errors.push(e.message));phone.on('pageerror',e=>errors.push(e.message));
  await context.route(relayUrl.replace('ws:','http:')+'/pairing',route=>route.fulfill({status:503,body:'Test registry unavailable'}));
  await desktop.goto(base); const invitation=await desktop.evaluate(async relayUrl=>{const m=await import('/bundle.js');await m.clearDesktopPairing();const record=await m.newDesktopPairing(relayUrl);await m.saveDesktopPairing(record);m.mountHarness();return JSON.stringify({version:2,relayUrl,room:record.identity.room,publicKey:m.bytesToBase64url(await m.exportPairingPublicKey(record.identity.keyPair.publicKey))});},relayUrl);
  const panel=desktop.getByRole('region',{name:'Settings'}).getByRole('region',{name:'Remote control'}); await expect(panel).toBeVisible(); await expect(desktop.getByRole('dialog',{name:'Remote control'})).toHaveCount(0); await expect(panel.getByRole('status')).toContainText('Ready for a phone'); await panel.getByRole('button',{name:'Create pairing code'}).click(); const qr=panel.getByAltText('Mobile pairing QR code'); await expect(qr).toBeVisible(); assert.ok(await qr.evaluate(i=>i.getBoundingClientRect().width<=Math.min(280,i.closest('.remote-panel').getBoundingClientRect().width)+1));
  await expect(panel.getByRole('alert')).toContainText('Pairing service unavailable');
  await desktop.screenshot({path:'verification/remote-settings-qr-407.png',fullPage:true});
  await phone.goto(base); await phone.evaluate(async invitation=>{const m=await import('/bundle.js');window.__phone=await m.createMobileSession(invitation);},invitation);
  await expect(panel.getByRole('status')).toContainText('Waiting for phone approval');
  await expect(panel.getByRole('alert')).toHaveCount(0);
  await panel.getByRole('button',{name:/approve phone/i}).click();
  await expect.poll(()=>phone.evaluate(()=>window.__phone.getStatus())).toBe('connected');
  await expect(panel.getByText('Phone 1',{exact:true})).toBeVisible();
  const sockets=await desktop.evaluate(()=>window.__socketCount);
  await desktop.getByRole('button',{name:'Appearance',exact:true}).click(); await expect(desktop.locator('.remote-panel')).toBeHidden();
  await phone.evaluate(()=>window.__phone.getSnapshot());
  await desktop.getByRole('button',{name:'Remote control',exact:true}).click(); await expect(panel).toBeVisible();
  await expect(panel.getByRole('status')).toContainText('Phone connected');
  await desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.unmount()); await expect(desktop.locator('.remote-panel')).toBeHidden();
  await phone.evaluate(()=>window.__phone.getSnapshot());
  await desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.mount()); await expect(panel).toBeVisible();
  await expect(panel.getByRole('status')).toContainText('Phone connected');
  assert.equal(await desktop.evaluate(()=>window.__socketCount),sockets,'Settings navigation must not recreate the live socket.');
  await desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.category('profile'));
  const name = desktop.getByRole('textbox',{name:'Your name'});
  await expect(name).toBeVisible();
  await name.fill('  Claudine  '); await name.blur();
  await expect.poll(()=>desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.saves.at(-1)?.userName)).toBe('Claudine');
  await name.fill(''); await name.blur();
  await expect.poll(()=>desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.saves.at(-1)?.userName)).toBe('');
  await name.fill('x'.repeat(81)); await name.blur();
  await expect(desktop.getByRole('alert')).toContainText('at most 80 characters');
  await expect.poll(()=>desktop.evaluate(()=>window.__REMOTE_SETTINGS_QA__.saves.length)).toBe(2);
  await name.fill('Save failure'); await name.blur();
  await expect(desktop.getByRole('alert')).toContainText('Profile save failed');
  await desktop.screenshot({path:'verification/remote-settings-embedded-407.png',fullPage:true}); assert.deepEqual(errors,[]); console.log('Focused embedded remote Settings: narrow QR, registry failure, explicit approval and live socket survival passed.');
} finally {await browser?.close(); relay?.kill('SIGTERM'); await new Promise(ok=>server?server.close(ok):ok()); await rm(temp,{recursive:true,force:true});}
