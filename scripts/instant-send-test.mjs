import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch();
const page = await browser.newPage({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
const errors=[];
page.on('pageerror', error=>errors.push(error.message));
await page.addInitScript(readFileSync('scripts/ui-fixture.js','utf8') + `
  document.documentElement?.setAttribute('data-monitter-lan','1');
  document.addEventListener('DOMContentLoaded',()=>document.documentElement.setAttribute('data-monitter-lan','1'));
  const state=window.__MONITTER_QA__.snapshot();
  state.tasks=[{id:'instant-chat',agentId:'atlas',title:'Instant chat',status:'completed',archived:false,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only',nativeSessionId:null,createdAt:1,updatedAt:1}];
  window.__MONITTER_QA__.setSnapshot(state);
  const original=window.__MONITTER_BRIDGE__.sendMessage;
  window.__sendCalls=0;
  window.__MONITTER_BRIDGE__.sendMessage=async (...args)=>{
    window.__sendCalls++;
    await new Promise((resolve,reject)=>{window.__acceptSend=resolve;window.__failSend=()=>reject(new Error('Transport unavailable'));});
    return original(...args);
  };
`);
try {
  await page.route('**/api/access', route=>route.fulfill({json:{required:false}}));
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18440/');
  await page.locator('[data-task-id="instant-chat"] .task-select').click({timeout:60000});
  const composer=page.locator('.composer textarea');
  await composer.fill('Instant outgoing message');
  await page.getByRole('button',{name:'Send task message',exact:true}).click();
  const message=page.locator('article.message').filter({hasText:'Instant outgoing message'});
  await expect(message).toHaveCount(1);
  await expect(message.getByRole('status',{name:'Sending',exact:true})).toBeVisible();
  await expect(composer).toHaveValue('');
  await expect(composer).toBeEnabled();
  await composer.fill('My next draft');
  await page.evaluate(()=>window.__acceptSend());
  await expect(message).toHaveCount(1);
  await expect(message.getByRole('status',{name:'Sent',exact:true})).toBeVisible();
  await expect(composer).toHaveValue('My next draft');
  expect(await page.evaluate(()=>window.__sendCalls)).toBe(1);

  // A transport failure must retain the visible outgoing message and newer text.
  await composer.fill('Failure stays visible');
  await page.getByRole('button',{name:'Send task message',exact:true}).click();
  const failed=page.locator('article.message').filter({hasText:'Failure stays visible'});
  await expect(failed.getByRole('status',{name:'Sending',exact:true})).toBeVisible();
  await composer.fill('Do not overwrite this draft');
  await page.evaluate(()=>window.__failSend());
  await expect(failed).toContainText(/failed|not confirmed|could not|unable/i);
  await expect(composer).toHaveValue('Do not overwrite this draft');
  expect(await page.evaluate(()=>window.__sendCalls)).toBe(2);
  await failed.getByRole('button',{name:'Retry',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>window.__sendCalls)).toBe(3);
  await page.evaluate(()=>window.__acceptSend());
  await expect(page.getByRole('region',{name:'Queued messages'})).toContainText('Failure stays visible');
  await expect(failed).toHaveCount(0);
  await expect(composer).toHaveValue('Do not overwrite this draft');

  // A snapshot can confirm acceptance before the HTTP reply arrives.
  await page.evaluate(()=>{
    const state=window.__MONITTER_QA__.snapshot();
    state.tasks[0].status='completed';
    window.__MONITTER_QA__.setSnapshot(state);
    window.__MONITTER_BRIDGE__.sendMessage=async(taskId,text)=>{
      window.__sendCalls++;
      const fresh=window.__MONITTER_QA__.snapshot();
      fresh.messages.push({id:'early-ack',taskId,role:'user',text,createdAt:Date.now(),attachments:[]});
      window.__MONITTER_QA__.setSnapshot(fresh);
      await new Promise((resolve,reject)=>{window.__acceptSend=resolve;window.__failSend=()=>reject(new Error('Response lost after acceptance'));});
      return fresh;
    };
  });
  await composer.fill('Accepted before HTTP acknowledgement');
  await page.getByRole('button',{name:'Send task message',exact:true}).click();
  const early=page.locator('article.message').filter({hasText:'Accepted before HTTP acknowledgement'});
  await expect(early).toHaveCount(1);
  await expect(early.getByRole('status',{name:'Sent',exact:true})).toBeVisible();
  await page.evaluate(()=>window.__failSend());
  await expect(early).toHaveCount(1);
  await expect(early.getByRole('button',{name:'Retry',exact:true})).toHaveCount(0);
  await page.evaluate(()=>{
    window.__MONITTER_BRIDGE__.sendMessage=async()=>{
      window.__sendCalls++;
      return new Promise(()=>{});
    };
  });
  await composer.fill('Keep this across reload');
  await page.getByRole('button',{name:'Send task message',exact:true}).click();
  await expect(page.locator('article.message').filter({hasText:'Keep this across reload'}).getByRole('status',{name:'Sending',exact:true})).toBeVisible();
  await expect.poll(()=>page.evaluate(()=>sessionStorage.getItem('monitter.optimistic-outbox.v1:main') || '')).toContain('Keep this across reload');
  await page.reload();
  await page.locator('[data-task-id="instant-chat"] .task-select').click({timeout:60000});
  await expect(page.locator('article.message').filter({hasText:'Keep this across reload'})).toContainText(/not confirmed/i);
  expect(await page.evaluate(()=>window.__sendCalls)).toBe(0);
  expect(errors).toEqual([]);
  console.log('Instant send, acknowledgement reconciliation and failure draft preservation pass.');
} finally { await browser.close(); }
