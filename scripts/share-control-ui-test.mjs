// Focused browser harness for the owner-side exact-chat invitation flow.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium, expect as baseExpect } from '@playwright/test';
const expect=baseExpect.configure({timeout:15000}),root=process.cwd(),modules=existsSync(join(root,'node_modules'))?join(root,'node_modules'):resolve(root,'../monitter/node_modules');
const temp=await mkdtemp(join(tmpdir(),'monitter-share-control-'));let relay,server,browser;
const taskId='11111111-1111-4111-8111-111111111111';const snapshot={hosts:[],agents:[{id:'agent-1',name:'Codex',description:'',instructions:'private',avatar:null,provider:'codex',model:'x',hostId:'host-1',cwd:'/private',color:'#000',sandbox:'read-only',expertise:[],responsibilities:[],skills:[],collaborationEnabled:true}],tasks:[{id:taskId,agentId:'agent-1',title:'Launch plan',nativeSessionId:'private',archived:false,status:'idle',createdAt:1,updatedAt:1,parentTaskId:null,channelId:null,projectId:null,hostId:'host-1',cwd:'/private',provider:'codex',model:'x',sandbox:'read-only'}],messages:[],events:[],channels:[],projects:[],collaborations:[],queuedMessages:[],approvalRequests:[],settings:{accent:'#3978d4',theme:'light',interfaceScale:100,showToolActivity:true,showReasoningSummaries:true,sendWithEnter:false,sidebarView:'standard',userName:'Owner'}};
function startRelay(){relay=spawn(process.execPath,['scripts/relay-server.mjs'],{cwd:root,env:{...process.env,MONITTER_RELAY_PORT:'0',MONITTER_RELAY_HOST:'127.0.0.1'},stdio:['ignore','pipe','pipe']});return new Promise((ok,bad)=>{let text='';const timer=setTimeout(()=>bad(Error('Relay startup timed out.')),15000),read=b=>{text+=b;const found=text.match(/listening on (ws:\/\/[^\s]+)/);if(found){clearTimeout(timer);ok(found[1]);}};relay.stdout.on('data',read);relay.stderr.on('data',read);relay.once('error',bad);relay.once('exit',code=>bad(Error(`Relay exited ${code}: ${text}`)));});}
try {
  const bundle=join(temp,'bundle.js'),config=join(temp,'rolldown.config.mjs'),fixture=resolve(root,'scripts/fixtures');
  await writeFile(config,`import { existsSync } from 'node:fs';import { resolve } from 'node:path';import compiler from ${JSON.stringify(join(modules,'svelte/compiler/index.js'))};const root=${JSON.stringify(root)},fixture=${JSON.stringify(fixture)};export default {input:resolve(fixture,'share-control-entry.js'),output:{file:${JSON.stringify(bundle)},format:'es'},plugins:[{name:'focused',resolveId(id){if(id==='@lucide/svelte')return resolve(fixture,'share-control-lucide-stub.js');if(id==='qrcode')return resolve(fixture,'share-control-qrcode-stub.js');if(id.startsWith('$lib/')){const file=resolve(root,'src/lib',id.slice(5));return existsSync(file)?file:file+'.ts';}},transform(code,id){if(id.endsWith('.svelte'))return {code:compiler.compile(code,{filename:id,generate:'client',css:'injected'}).js.code.replaceAll('import.meta.env','({})'),map:null};return {code:code.replaceAll('import.meta.env','({})'),map:null};}}]};`);
  execFileSync(join(modules,'.bin/rolldown'),['--config',config,'--platform','browser'],{cwd:root,stdio:'inherit'});
  server=createServer(async(req,res)=>{const path=new URL(req.url,'http://x').pathname;if(path==='/bundle.js'){res.setHeader('content-type','text/javascript');res.end(await readFile(bundle));}else {res.setHeader('content-type','text/html');res.end('<!doctype html><div id="app"></div>');}});await new Promise(ok=>server.listen(0,'127.0.0.1',ok));
  const relayUrl=await startRelay(),base=`http://127.0.0.1:${server.address().port}`;browser=await chromium.launch();const context=await browser.newContext();
  const sends=[];await context.addInitScript(snapshot=>{window.__MONITTER_TEST_BRIDGE__={invoke:async(command,args)=>{if(command==='get_snapshot')return snapshot;if(command==='send_message'){window.__shareSends.push(args);return snapshot;}throw Error('Unexpected '+command);}};window.__shareSends=[];},snapshot);
  await context.route(relayUrl.replace('ws:','http:')+'/pairing',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({code:'123456789',expiresAt:Date.now()+300000,revocationToken:'test-token'})}));
  const desktop=await context.newPage(),phone=await context.newPage(),errors=[];desktop.on('pageerror',error=>errors.push(error.message));await desktop.goto(base);await desktop.evaluate(async()=>{const m=await import('/bundle.js');m.mountHarness();});
  const dialog=desktop.getByRole('dialog',{name:'Share this chat'});await expect(dialog.getByRole('textbox',{name:'Primary operator name'})).toHaveValue('Owner');await dialog.getByRole('textbox',{name:'Share relay address'}).fill(relayUrl);await dialog.getByRole('button',{name:'Create one-use share link'}).click();
  const raw=await dialog.getByAltText('One-use sharing QR code').getAttribute('src'),link=decodeURIComponent(raw.slice('data:text/plain,'.length));assert.match(link,/^https:\/\/share\.monitter\.com\/share#invite=/);assert.ok(!link.includes('tauri:'));
  const invitation=new URLSearchParams(new URL(link).hash.slice(1)).get('invite');await phone.goto(base);await phone.evaluate(async invitation=>{const m=await import('/bundle.js');window.__visitor=await m.createMobileSession(invitation,{name:'Roger',role:'visitor'});},invitation);
  await expect(dialog.getByRole('button',{name:'Approve Roger'})).toBeVisible();await dialog.getByRole('button',{name:'Approve Roger'}).click();await expect.poll(()=>phone.evaluate(()=>window.__visitor.getStatus())).toBe('connected');const activeShare=await desktop.evaluate(async()=>{const m=await import('/bundle.js');let value;const unsubscribe=m.activeOperatorShare.subscribe(next=>value=next);unsubscribe();return {value,taskScopeFrozen:Object.isFrozen(value.taskIds),projectScopeFrozen:Object.isFrozen(value.projectIds)};});assert.deepEqual(activeShare.value,{primary:{name:'Owner',role:'primary user'},visitor:{name:'Roger',role:'visitor'},taskIds:[taskId],projectIds:[]});assert.equal(activeShare.taskScopeFrozen,true);assert.equal(activeShare.projectScopeFrozen,true);const shared=await phone.evaluate(()=>window.__visitor.getSnapshot());assert.deepEqual(shared.tasks.map(task=>task.id),[taskId]);assert.equal(shared.tasks[0].cwd,'');assert.deepEqual(shared.events,[]);assert.equal(shared.sharing.visitor.name,'Roger');assert.equal(shared.sharing.primary.name,'Owner');assert.equal(shared.sharing.appearance.variables['--accent'],'#3978d4');
  await desktop.evaluate(()=>document.documentElement.style.setProperty('--accent','#8755c7'));const recoloured=await phone.evaluate(()=>window.__visitor.getSnapshot());assert.equal(recoloured.sharing.appearance.variables['--accent'],'#8755c7');
  await phone.evaluate(taskId=>window.__visitor.sendMessage(taskId,'Hello'),taskId);await expect.poll(()=>desktop.evaluate(()=>window.__shareSends.length)).toBe(1);const sent=await desktop.evaluate(()=>window.__shareSends[0]);assert.match(sent.text,/^\[Two human operators are collaborating in this chat: Owner \(primary user\), Roger \(visitor\)\./);assert.match(sent.text,/@\(Roger\): Hello$/);assert.deepEqual(errors,[]);console.log('ShareControl UI: profile prefill, Roger approval, immutable exact-chat scope, and visitor attribution passed.');
  // Workspace sharing has a bounded selection scroller; its controls stay put.
  const workspaceContext = await browser.newContext({viewport:{width:1280,height:900}});
  const workspaceSnapshot = structuredClone(snapshot);
  workspaceSnapshot.projects = Array.from({length:12},(_,i)=>({id:`project-${i}`,name:`Project ${i+1}`,description:'',icon:'folder',color:'#397e61',workspaces:[]}));
  workspaceSnapshot.tasks = [
    {...snapshot.tasks[0],id:'project-chat',title:'Inside a project',projectId:'project-0'},
    ...Array.from({length:35},(_,i)=>({...snapshot.tasks[0],id:`loose-${i}`,title:`Loose chat ${i+1}`})),
  ];
  await workspaceContext.addInitScript(snapshot=>{window.__MONITTER_TEST_BRIDGE__={invoke:async command=>{if(command==='get_snapshot')return snapshot;throw Error('Unexpected '+command);}};},workspaceSnapshot);
  await workspaceContext.route(relayUrl.replace('ws:','http:')+'/pairing',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify({code:'987654321',expiresAt:Date.now()+300000,revocationToken:'test-token'})}));
  const workspace=await workspaceContext.newPage(),collaborator=await workspaceContext.newPage();
  workspace.on('pageerror',error=>errors.push(error.message));
  await workspace.goto(base);
  await workspace.evaluate(async()=>{const m=await import('/bundle.js');m.mountHarness({taskId:null});});
  const workspaceDialog=workspace.getByRole('dialog',{name:'Share workspace'});
  await workspaceDialog.getByLabel('Share relay address').fill(relayUrl);
  await workspaceDialog.getByRole('button',{name:'Create one-use share link'}).click();
  const workspaceQr=await workspaceDialog.getByAltText('One-use sharing QR code').getAttribute('src');
  const workspaceInvite=new URLSearchParams(new URL(decodeURIComponent(workspaceQr.slice('data:text/plain,'.length))).hash.slice(1)).get('invite');
  await collaborator.goto(base);
  await collaborator.evaluate(async invite=>{const m=await import('/bundle.js');window.__visitor=await m.createMobileSession(invite,{name:'Luke',role:'visitor'});},workspaceInvite);
  await workspaceDialog.getByRole('button',{name:'Approve Luke'}).click();
  const selection=workspaceDialog.getByRole('region',{name:'Sharing selection'});
  await expect(selection).toBeVisible();
  assert.deepEqual(await selection.getByRole('heading').allTextContents(),['Projects','Loose chats']);
  await expect(selection.getByRole('checkbox')).toHaveCount(47);
  await expect(selection.getByText('Inside a project',{exact:true})).toHaveCount(0);
  await selection.getByRole('checkbox',{name:'Project 1 Shares its current chats',exact:true}).check();
  const scope=await collaborator.evaluate(()=>window.__visitor.getSnapshot());
  assert.deepEqual(scope.tasks.map(task=>task.id),['project-chat']);
  for(const size of [{width:1280,height:900},{width:390,height:680}]) {
    await workspace.setViewportSize(size);
    const close=workspaceDialog.getByRole('button',{name:'Close sharing'});
    const revoke=workspaceDialog.getByRole('button',{name:'End sharing and revoke access'});
    const before={close:await close.boundingBox(),revoke:await revoke.boundingBox()};
    assert(await selection.evaluate(node=>node.scrollHeight>node.clientHeight));
    await selection.evaluate(node=>node.scrollTop=node.scrollHeight);
    const after={close:await close.boundingBox(),revoke:await revoke.boundingBox()};
    assert.deepEqual(after,before);
    assert(before.close.y>=0 && before.revoke.y+before.revoke.height<size.height);
    assert.equal(await workspaceDialog.evaluate(node=>node.scrollTop),0);
    await selection.getByRole('checkbox',{name:'Loose chat 35 Codex',exact:true}).check();
    await selection.evaluate(node=>node.scrollTop=0);
  }
  await workspace.screenshot({path:'/tmp/monitter-share-selection-mobile.png'});
  assert.deepEqual(errors,[]);
  console.log('Share workspace: projects first, loose chats only, live scope, independently scrolling selection and fixed controls passed.');
  await workspaceContext.close();

} finally {await browser?.close();relay?.kill('SIGTERM');await new Promise(ok=>server?server.close(ok):ok());await rm(temp,{recursive:true,force:true});}
