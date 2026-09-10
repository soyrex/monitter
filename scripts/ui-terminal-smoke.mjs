import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18429';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const passed = [], errors = [];
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' }); await page.goto(url);
  await expect(page.getByRole('button', { name: 'Open terminal', exact: true })).toBeVisible({ timeout: 30000 });
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__, s=qa.snapshot();
    s.hosts.push({id:'ssh',name:'QA SSH',kind:'ssh',address:'qa.example',user:'',port:0,identityFile:'',defaultCwd:'~/remote',codexPath:'codex',claudePath:'',opencodePath:'',hermesPath:''});
    s.agents.push({...s.agents[0],id:'remote-agent',name:'Remote Atlas',hostId:'ssh',cwd:'~/remote'});
    s.tasks.push({id:'local-task',agentId:'atlas',title:'Local terminal task',nativeSessionId:null,status:'idle',archived:false,createdAt:Date.now(),updatedAt:Date.now(),parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'},{id:'ssh-task',agentId:'remote-agent',title:'SSH terminal task',nativeSessionId:null,status:'idle',archived:false,createdAt:Date.now(),updatedAt:Date.now(),parentTaskId:null,channelId:null,projectId:null,hostId:'ssh',cwd:'~/remote',provider:'codex',model:'',sandbox:'read-only'});qa.setSnapshot(s);
  });
  const main=page.locator('.pane-leaf[data-pane-id="main"]');
  const openTask=async name=>page.locator('.sidebar .task-select').filter({hasText:name}).first().click();
  const tab=(pane,name)=>pane.locator('.tabs').getByRole('button',{name,exact:true});
  async function drag(source,target) {
    const data=await page.evaluateHandle(()=>new DataTransfer());
    await source.dispatchEvent('dragstart',{dataTransfer:data});
    const box=await target.boundingBox();
    await target.dispatchEvent('dragover',{clientX:box.x+box.width/2,clientY:box.y+box.height/2,dataTransfer:data});
    await target.dispatchEvent('drop',{clientX:box.x+box.width/2,clientY:box.y+box.height/2,dataTransfer:data});
    await source.dispatchEvent('dragend').catch(()=>{});await data.dispose();
  }
  await openTask('Local terminal task'); await main.getByRole('button',{name:'Open terminal',exact:true}).click();
  await expect(page.locator('.terminal-pane .xterm')).toBeVisible();
  const localOpen=await page.evaluate(()=>window.__MONITTER_QA__.calls.findLast(call=>call.method==='openTerminal'));expect(localOpen.args.target).toEqual({taskId:'local-task'});
  const localId=await page.evaluate(()=>window.__MONITTER_QA__.terminals()[0].id); passed.push('opens local target and renders xterm');
  const input=page.locator('.terminal-pane .xterm-helper-textarea');await input.focus();await page.keyboard.type('echo fixture');await page.keyboard.press('Control+c');await page.keyboard.press('Control+p');
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='writeTerminal').map(call=>call.args.data))).toEqual(expect.arrayContaining(['\u0003','\u0010']));passed.push('routes Ctrl-C and Ctrl-P to shell');
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='writeTerminal').map(call=>call.args.data).join(''))).toContain('echo fixture');
  await page.keyboard.press('Meta+p');await expect(page.getByRole('dialog',{name:'Controls',exact:true})).toBeVisible();await page.keyboard.press('Escape');await input.focus();passed.push('Meta-P remains available to Monitter');
  await page.evaluate(()=>{const bridge=window.__MONITTER_BRIDGE__;window.__terminalWrite=bridge.writeTerminal;bridge.writeTerminal=async()=>{throw Error('Fixture input failure')};});
  await input.focus();await page.keyboard.type('x');await expect(page.locator('.terminal-state.error')).toContainText('Fixture input failure');
  const reads=await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='readTerminal').length);
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='readTerminal').length)).toBeGreaterThan(reads+2);
  await expect(page.locator('.terminal-state.error')).toContainText('Fixture input failure');
  await page.evaluate(()=>{window.__MONITTER_BRIDGE__.writeTerminal=window.__terminalWrite;});await input.focus();await page.keyboard.type('y');await expect(page.locator('.terminal-state.error')).toHaveCount(0);
  passed.push('input errors remain visible through successful background reads and clear after input recovers');

  await openTask('Local terminal task');await tab(main,'This Mac shell').click();
  expect(await page.evaluate(()=>window.__MONITTER_QA__.terminals()[0].id)).toBe(localId);await expect.poll(()=>page.locator('.xterm-rows').count()).toBeGreaterThan(0);passed.push('hide/reopen retains terminal');
  await main.getByRole('button',{name:'Pane layout',exact:true}).click();await page.getByRole('menuitem',{name:'Two columns',exact:true}).click();
  const second=page.locator('.pane-leaf').last();await drag(tab(main,'This Mac shell'),second);
  await expect(tab(second,'This Mac shell')).toBeVisible();
  await expect.poll(()=>page.evaluate(value=>window.__MONITTER_QA__.terminals().some(item=>item.id===value),localId)).toBe(true);
  expect(await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='openTerminal'&&call.args.target.taskId==='local-task').length)).toBe(1);passed.push('pane move retains process');
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.focusFollowsMouse=true;q.setSnapshot(s)});
  await expect(second).toHaveAttribute('data-focus-follows-mouse','true');await tab(main,'Local terminal task').click();await second.hover();await expect(second.locator('.xterm-helper-textarea')).toBeFocused();
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.focusFollowsMouse=false;q.setSnapshot(s)});
  passed.push('focus follows mouse gives the hovered terminal keyboard focus');

  await tab(second,'This Mac shell').click();const resizeBefore=await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='resizeTerminal').length);await page.setViewportSize({width:1120,height:720});await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='resizeTerminal').length)).toBeGreaterThan(resizeBefore);
  expect(await page.evaluate(()=>document.documentElement.scrollHeight>innerHeight||document.documentElement.scrollWidth>innerWidth)).toBe(false);passed.push('resize has no document overflow');
  await page.locator('.sidebar .task-select').filter({hasText:'SSH terminal task'}).first().click();const sshPane=page.locator('.pane-leaf').filter({has:page.locator('.tabs').getByRole('button',{name:'SSH terminal task',exact:true})});await sshPane.getByRole('button',{name:'Open terminal',exact:true}).click();
  const sshOpen=await page.evaluate(()=>window.__MONITTER_QA__.calls.findLast(call=>call.method==='openTerminal'));expect(sshOpen.args.target).toEqual({taskId:'ssh-task'});passed.push('opens SSH target');
  await page.evaluate(()=>{window.__MONITTER_QA__.terminalCloseFailure='Fixture close failure';});await page.getByRole('button',{name:/Close terminal QA SSH shell/,exact:true}).click();
  await expect(page.getByText(/Could not close terminal: Fixture close failure/)).toBeVisible();expect(await page.evaluate(()=>window.__MONITTER_QA__.terminals().some(item=>item.hostId==='ssh'))).toBe(true);
  await page.evaluate(()=>{window.__MONITTER_QA__.terminalCloseFailure=null;});await page.getByRole('button',{name:/Close terminal QA SSH shell/,exact:true}).click();await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.terminals().some(item=>item.hostId==='ssh'))).toBe(false);passed.push('retryable close');
  await page.screenshot({path:'verification/ui-terminal.png'});if(errors.length)throw Error(errors.join('\\n'));writeFileSync('verification/ui-terminal-smoke.json',JSON.stringify({passed},null,2));console.log('terminal smoke passed: '+passed.join('; '));
} catch(error) { console.log(await page.evaluate(()=>({focused:document.activeElement?.outerHTML,hasFocus:document.hasFocus(),panes:Array.from(document.querySelectorAll('.pane-leaf')).map(el=>({id:el.dataset.paneId,focus:el.dataset.focusFollowsMouse,cls:el.className})),dialogs:Array.from(document.querySelectorAll('dialog[open],[role="dialog"],:popover-open')).map(el=>el.outerHTML.slice(0,200))})));writeFileSync('verification/ui-terminal-smoke.json',JSON.stringify({passed,error:String(error),errors},null,2));throw error; } finally { await browser.close(); }
