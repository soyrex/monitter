import { webkit, expect as baseExpect } from '@playwright/test';
import { spawn } from 'node:child_process';
const expect = baseExpect.configure({timeout:30000});
const children=[];
function start(command,args,env,pattern){
  const child=spawn(command,args,{env:{...process.env,...env},stdio:['ignore','pipe','pipe']});children.push(child);
  return new Promise((resolve,reject)=>{
    let output='';const timer=setTimeout(()=>reject(Error('Test server startup timed out')),60000);
    child.once('error',e=>{clearTimeout(timer);reject(e);});
    child.once('exit',code=>{clearTimeout(timer);reject(Error(`Test server exited ${code}: ${output.slice(-1000)}`));});
    const receive=data=>{output+=data.toString();const match=output.match(pattern);if(match){clearTimeout(timer);resolve(match[1]);}};
    child.stdout.on('data',receive);child.stderr.on('data',receive);
  });
}
let browser;
try {
  const relayUrl=await start(process.execPath,['scripts/relay-server.mjs'],{MONITTER_RELAY_PORT:'0',MONITTER_RELAY_HOST:'127.0.0.1'},/listening on (ws:\/\/[^\s]+)/);
  const baseUrl=process.env.MONITTER_TEST_URL||await start(process.execPath,['node_modules/vite/bin/vite.js','--host','127.0.0.1','--port','0'],{NO_COLOR:'1'},/Local:\s+(http:\/\/[^\s]+)/);
  browser=await webkit.launch();
  const host=await browser.newPage();
  await host.goto(baseUrl,{waitUntil:'domcontentloaded',timeout:60000});
  const invitation=await host.evaluate(async(relayUrl)=>{
    const {createDesktopSession}=await import('/src/lib/controller/remote-client.ts');
    const taskId='11111111-1111-4111-8111-111111111111';
    const snapshot={hosts:[],agents:[{id:'a',name:'UI test agent',provider:'codex'}],tasks:[{id:taskId,agentId:'a',title:'Controller test',status:'idle',archived:false,updatedAt:1}],messages:[],events:[],channels:[],projects:[],collaborations:[],queuedMessages:[],settings:{}};
    snapshot.tasks.push({...snapshot.tasks[0],id:'22222222-2222-4222-8222-222222222222',title:'Other test chat'});
    window.testSends=0;
    window.desktop=await createDesktopSession(relayUrl,{
      getSnapshot:async()=>{if(window.holdSnapshot){window.snapshotStarted=true;await new Promise(resolve=>window.releaseSnapshot=resolve);}return snapshot;},
      sendMessage:async(id,text)=>{if(window.holdSend){window.sendStarted=true;await new Promise(resolve=>window.releaseSend=resolve);}window.testSends++;snapshot.messages.push({id:'m',taskId:id,text,role:'user'});return snapshot;},
      cancelTask:async()=>snapshot,resumeTask:async()=>snapshot,listTerminals:async()=>[],readTerminal:async()=>({chunks:[]})
    });
    return window.desktop.invitation;
  },relayUrl);
  const phone=await browser.newPage({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  const errors=[];phone.on('pageerror',e=>errors.push(e.message));
  await phone.goto(new URL('mobile',baseUrl).href,{waitUntil:'domcontentloaded',timeout:60000});
  await phone.evaluate(()=>{window.scanRequests=0;window.monitterAndroid={postMessage:value=>{if(value==='scanQR')window.scanRequests++;}};});
  await phone.getByRole('button',{name:'Scan desktop code'}).click();
  if(await phone.evaluate(()=>window.scanRequests)!==1)throw Error('Android scanner bridge was not invoked');
  await phone.evaluate(invitation=>window.dispatchEvent(new CustomEvent('monitter:qr',{detail:invitation})),invitation);
  await expect.poll(()=>host.evaluate(()=>window.desktop.getStatus())).toBe('pending');
  await expect(phone.getByRole('tab',{name:'Standard view',exact:true})).toHaveCount(0);
  const code=await host.evaluate(()=>window.desktop.getVerificationCode());
  await expect(phone.getByText(code,{exact:true})).toBeVisible();
  await host.evaluate(()=>window.desktop.approve());
  // Hold the response until assertions complete, independent of machine speed.
  await expect(phone.getByRole('tab',{name:'Standard view',exact:true})).toBeVisible();
  await host.evaluate(()=>{window.holdSnapshot=true;window.snapshotStarted=false;});
  await phone.getByRole('button',{name:'Refresh',exact:true}).click();
  await expect.poll(()=>host.evaluate(()=>window.snapshotStarted)).toBe(true);
  await phone.getByRole('tab',{name:'Activity view',exact:true}).click();
  await expect(phone.getByRole('tab',{name:'Activity view',exact:true})).toHaveAttribute('aria-selected','true');
  await host.evaluate(()=>{window.holdSnapshot=false;window.releaseSnapshot();});
  // Allow another completed background refresh; Activity must stay selected.
  await phone.waitForTimeout(2200);
  await expect(phone.getByRole('tab',{name:'Activity view',exact:true})).toHaveAttribute('aria-selected','true');
  await phone.getByRole('tab',{name:'Standard view',exact:true}).click();
  await phone.getByRole('button',{name:'Controller test'}).click();
  await phone.getByLabel('Message',{exact:true}).fill('Keep this draft');
  await phone.getByRole('button',{name:'Back to chats'}).click();
  await phone.getByRole('button',{name:'Other test chat'}).click();
  await expect(phone.getByLabel('Message',{exact:true})).toHaveValue('');
  await phone.getByRole('button',{name:'Back to chats'}).click();
  await phone.getByRole('button',{name:'Controller test'}).click();
  await expect(phone.getByLabel('Message',{exact:true})).toHaveValue('Keep this draft');
  await phone.getByLabel('Message',{exact:true}).fill('Mobile UI test message');
  await host.evaluate(()=>{window.holdSend=true;window.sendStarted=false;});
  await phone.getByRole('button',{name:'Send message'}).click();
  await expect.poll(()=>host.evaluate(()=>window.sendStarted)).toBe(true);
  await phone.getByRole('button',{name:'Back to chats'}).click();
  await phone.getByRole('button',{name:'Controller test'}).click();
  await expect(phone.getByLabel('Message',{exact:true})).toHaveValue('Mobile UI test message');
  await expect(phone.getByRole('button',{name:'Send message'})).toBeDisabled();
  await host.evaluate(()=>{window.holdSend=false;window.releaseSend();});
  await expect(phone.getByText('Mobile UI test message',{exact:true})).toBeVisible();
  await expect.poll(()=>host.evaluate(()=>window.testSends)).toBe(1);
  await phone.getByRole('button',{name:'Back to chats'}).click();
  await phone.getByRole('button',{name:'Preferences',exact:true}).click();
  const mobileScale=phone.getByRole('slider',{name:'Mobile interface scale',exact:true});
  await expect(mobileScale).toHaveValue('125');
  await mobileScale.fill('175');
  await expect.poll(()=>phone.evaluate(()=>Number(localStorage.getItem('monitter.interface-scale.v1:mobile')))).toBe(175);
  await expect.poll(()=>phone.evaluate(()=>getComputedStyle(document.querySelector('.mobile')).getPropertyValue('--viewer-interface-scale').trim())).toBe('1.75');
  expect(await phone.evaluate(()=>localStorage.getItem('monitter.interface-scale.v1:desktop'))).toBeNull();
  const mobileBounds=await phone.locator('.mobile').evaluate(element=>{const box=element.getBoundingClientRect();return {width:box.width,height:box.height};});
  expect(Math.abs(mobileBounds.width-390)).toBeLessThan(1);
  expect(Math.abs(mobileBounds.height-844)).toBeLessThan(1);
  await phone.screenshot({path:'verification/mobile-chat-test.png'});
  await phone.getByRole('button',{name:'Disconnect',exact:true}).click();
  await expect(phone.getByRole('button',{name:'Connect to desktop'})).toBeVisible();
  if(errors.length)throw Error(errors.join('\n'));
  console.log('WebKit phone UI: approval, chat, single send, disconnect passed (test bridge).');
} finally {await browser?.close();for(const child of children)child.kill('SIGTERM');}
