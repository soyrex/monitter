import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const output = 'verification/ui-collaboration-results.json';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const passed = [];
const pageErrors = [];
let page;

try {
  page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Monitter menu', exact: true })).toBeVisible({ timeout: 30000 });

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.agents.push({ id:'scout', avatar:null, name:'Scout', description:'Release and incident collaborator', instructions:'Use concise status updates.', provider:'codex', model:'', hostId:'local', cwd:'/tmp/scout', color:'#8755c7', sandbox:'read-only', expertise:['release engineering','incident response'], responsibilities:['Validate deployments'], skills:['Playwright','Go'], collaborationEnabled:true });
    state.agents.push({ id:'catalog', avatar:null, name:'Catalog', description:'Data catalogue collaborator', instructions:'Keep records accurate.', provider:'codex', model:'', hostId:'local', cwd:'/tmp/catalog', color:'#3978d4', sandbox:'read-only', expertise:['taxonomy'], responsibilities:['Maintain catalogues'], skills:['SQL'], collaborationEnabled:true });
    // Deliberately omit post-profile fields to exercise saved legacy data.
    state.agents.push({ id:'legacy', avatar:null, name:'Legacy', description:'Older saved agent', instructions:'Preserve old settings.', provider:'codex', model:'', hostId:'local', cwd:'/tmp/legacy', color:'#c27524', sandbox:'read-only' });
    state.tasks = [
      {id:'source-task',agentId:'atlas',title:'Source collaboration chat',nativeSessionId:null,status:'idle',archived:false,createdAt:now,updatedAt:now,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'},
      {id:'recipient-task',agentId:'scout',title:'Scout linked task',nativeSessionId:null,status:'completed',archived:false,createdAt:now,updatedAt:now,parentTaskId:'source-task',channelId:null,projectId:null,hostId:'local',cwd:'/tmp/scout',provider:'codex',model:'',sandbox:'read-only'},
    ];
    state.messages = [{id:'incoming-scout',taskId:'source-task',role:'assistant',text:'Saved sender identity is used here.',senderAgentId:'scout',collaborationId:'collaboration-ok',createdAt:now}];
    state.collaborations = [
      {id:'collaboration-ok',kind:'delegation',fromAgentId:'atlas',fromTaskId:'source-task',toAgentId:'scout',toTaskId:'recipient-task',text:'Validate the candidate.',requestId:'req-ok',status:'completed',result:'Validation passed.',error:null,createdAt:now,updatedAt:now},
      {id:'collaboration-error',kind:'message',fromAgentId:'atlas',fromTaskId:'source-task',toAgentId:'scout',toTaskId:'recipient-task',text:'Check deployment status.',requestId:'req-error',status:'error',result:null,error:'Recipient transport failed.',createdAt:now,updatedAt:now},
      {id:'collaboration-delivered',kind:'message',fromAgentId:'atlas',fromTaskId:'source-task',toAgentId:'scout',toTaskId:'recipient-task',text:'Inbox delivery.',requestId:'req-delivered',status:'running',result:'Delivered to the recipient’s active turn via its Monitter inbox.',error:'Partial result: recipient still processing.',createdAt:now,updatedAt:now},
    ];
    qa.setSnapshot(state);
  });

  const openDirectory = async () => {
    await page.getByRole('button', { name: 'Monitter menu', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Agent directory', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Agent directory', exact: true })).toBeVisible();
  };

  await openDirectory();
  let dialog = page.getByRole('dialog', { name: 'Agent directory', exact: true });
  const search = dialog.getByLabel('Find agents', { exact: true });
  await search.fill('playwright');
  await expect(dialog.getByText('Scout', { exact: true })).toBeVisible();
  await expect(dialog.getByText('Catalog', { exact: true })).toHaveCount(0);
  await page.screenshot({ path:'verification/ui-collaboration-directory.png' });
  await search.fill('no saved capability');
  await expect(dialog.getByText('No saved agents match this search.', { exact: true })).toBeVisible();
  await search.fill('release engineering');
  await dialog.getByRole('button', { name: 'Edit Scout', exact: true }).click();
  dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Collaboration profile', { exact: true })).toBeVisible();
  await dialog.getByText('Collaboration profile', { exact: true }).click();
  const expertise = dialog.getByLabel('Expertise', { exact: true });
  await expertise.fill('release engineering');
  await expertise.press('Enter');
  await expertise.type('quality assurance');
  await dialog.getByLabel('Responsibilities', { exact: true }).fill('Validate deployments\nEscalate incidents');
  await dialog.getByLabel('Skills', { exact: true }).fill('Playwright\nGo');
  await dialog.getByRole('switch', { name: 'Available for collaboration', exact: true }).uncheck();
  await page.screenshot({ path:'verification/ui-collaboration-profile.png' });
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents.find(agent => agent.id === 'scout'))).toMatchObject({ expertise:['release engineering','quality assurance'], responsibilities:['Validate deployments','Escalate incidents'], skills:['Playwright','Go'], collaborationEnabled:false });
  passed.push('directory capability search, empty results, and saved multiline collaboration profile');

  await openDirectory();
  dialog = page.getByRole('dialog', { name: 'Agent directory', exact: true });
  await dialog.getByLabel('Find agents', { exact: true }).fill('quality assurance');
  await expect(dialog.getByText(/Collaboration off/)).toBeVisible();
  await dialog.getByRole('button', { name: 'Edit Scout', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByText('Collaboration profile', { exact: true }).click();
  await dialog.getByRole('switch', { name: 'Available for collaboration', exact: true }).check();
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents.find(agent => agent.id === 'scout').collaborationEnabled)).toBe(true);
  passed.push('collaboration availability toggles round-trip through saved profile');

  await openDirectory();
  dialog = page.getByRole('dialog', { name: 'Agent directory', exact: true });
  await dialog.getByLabel('Find agents', { exact: true }).fill('Legacy');
  await dialog.getByRole('button', { name: 'Edit Legacy', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByText('Collaboration profile', { exact: true }).click();
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents.find(agent => agent.id === 'legacy'))).toMatchObject({ expertise:[], responsibilities:[], skills:[] });
  passed.push('editing a legacy saved agent normalizes missing collaboration profile fields');

  await page.getByRole('button', { name: /Source collaboration chat/ }).first().click();
  await expect(page.locator('.message-meta').getByText('Scout', { exact: true })).toBeVisible();
  const rows = page.locator('.collaboration-row');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText('Delegation · Scout');
  await expect(rows.nth(0)).toContainText('completed');
  await expect(rows.nth(0)).toContainText('Validation passed.');
  await expect(rows.nth(1)).toContainText('Agent message · Scout');
  await expect(rows.nth(1)).toContainText('error');
  await expect(rows.nth(1)).toContainText('Recipient transport failed.');
  await expect(rows.nth(2)).toContainText('Delivered');
  await expect(rows.nth(2)).toContainText('Delivered to the recipient’s active turn via its Monitter inbox.');
  await expect(rows.nth(2)).toContainText('Partial result: recipient still processing.');
  await page.screenshot({ path:'verification/ui-collaboration-lineage.png' });
  await rows.nth(0).click();
  await expect(page.getByRole('heading', { name: 'Scout linked task', exact: true })).toBeVisible();
  passed.push('saved sender identity, Delivered inbox sentinel, partial result/error, and linked task navigation');

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    state.settings.interfaceScale = 185;
    for (let i = 0; i < 45; i++) state.messages.push({id:`long-history-${i}`,taskId:'recipient-task',role:'assistant',text:'Long collaboration history remains inside the message viewport. '.repeat(12),senderAgentId:'scout',createdAt:Date.now()+i});
    for (let i = 0; i < 25; i++) state.agents.push({id:`directory-${i}`,avatar:null,name:`Directory ${i}`,description:'Long synthetic directory entry',instructions:'Fixture only.',provider:'codex',model:'',hostId:'local',cwd:'/tmp',color:'#397e61',sandbox:'read-only',expertise:['long directory capability'],responsibilities:[],skills:['search'],collaborationEnabled:true});
    qa.setSnapshot(state);
  });
  await page.setViewportSize({ width: 760, height: 520 });
  await expect.poll(() => page.locator('.messages').evaluate(element => element.scrollHeight > element.clientHeight)).toBe(true);
  const bounds = await page.evaluate(() => ({ root:document.documentElement.scrollHeight, body:document.body.scrollHeight, height:innerHeight, top:scrollY }));
  expect(bounds.root).toBeLessThanOrEqual(bounds.height);
  expect(bounds.body).toBeLessThanOrEqual(bounds.height);
  expect(bounds.top).toBe(0);
  await page.screenshot({ path:'verification/ui-collaboration-185.png' });
  passed.push('long collaboration history and directory data keep the document fixed at 185% scale');

  expect(pageErrors).toEqual([]);
  const report = { completedAt:new Date().toISOString(), passed, pageErrors, output };
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  writeFileSync(output, `${JSON.stringify({ completedAt:new Date().toISOString(), passed, pageErrors, error:String(error) }, null, 2)}\n`);
  if (page) await page.screenshot({ path:'verification/ui-collaboration-failure.png' });
  throw error;
} finally { await browser.close(); }
