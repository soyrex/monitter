import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1500,height:950}}), errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.settings.shortcutMode='vim';s.tasks=['A','B','C'].map((title,i)=>({id:'vim-layout-'+title,agentId:'atlas',title,nativeSessionId:null,status:'idle',archived:false,createdAt:n+i,updatedAt:n+i,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}));q.setSnapshot(s);`});
  await page.goto('http://127.0.0.1:18433'); await page.getByRole('button',{name:'Preferences',exact:true}).waitFor({timeout:60000});
  async function command(value){await page.keyboard.press('Escape');await page.keyboard.type(':');const input=page.getByRole('textbox',{name:'Monitter command'});await input.fill(value);await input.press('Enter');}
  const panes=()=>page.locator('.pane-leaf');
  const geometry=()=>page.locator('.pane-split').evaluateAll(nodes=>nodes.map(node=>({axis:node.classList.contains('column')?'vertical':'horizontal',children:[...node.children].filter(child=>child.classList.contains('split-child')).map(child=>getComputedStyle(child).flex)})));
  const tabs=()=>page.locator('[data-tab-kind="task"]').evaluateAll(nodes=>nodes.map(node=>node.getAttribute('data-tab-id')).sort());

  await page.locator('.sidebar .task-select[title="A"]').click();await page.getByLabel('Task message',{exact:true}).fill('Draft A');
  await command('vsplit'); await expect(panes()).toHaveCount(2);
  await page.locator('.sidebar .task-select[title="B"]').click();await panes().nth(1).getByLabel('Task message',{exact:true}).fill('Draft B'); await command('split'); await expect(panes()).toHaveCount(3);await page.locator('.sidebar .task-select[title="C"]').click();await panes().last().getByLabel('Task message',{exact:true}).fill('Draft C');
  const beforeGeometry=await geometry(), beforeTabs=await tabs();
  await command('wincmd r'); expect(await tabs()).toEqual(beforeTabs); expect(await geometry()).toEqual(beforeGeometry);
  await command('wincmd x'); expect(await tabs()).toEqual(beforeTabs); expect(await geometry()).toEqual(beforeGeometry);
  for(const edge of ['H','J','K','L']) { await command('wincmd '+edge); expect(await tabs()).toEqual(beforeTabs); expect((await page.getByLabel('Task message',{exact:true}).evaluateAll(nodes=>nodes.map(node=>node.value))).sort()).toEqual(['Draft A','Draft B','Draft C']); }
  const beforeResize=await geometry(); await command('wincmd >'); await expect.poll(async()=>JSON.stringify(await geometry())).not.toBe(JSON.stringify(beforeResize));
  await command('only'); await expect(panes()).toHaveCount(1); expect(await tabs()).toEqual(beforeTabs);
  const active=await page.locator('.tab-entry.active').getAttribute('data-tab-id'); await command('99tabclose'); await expect(page.getByRole('alert')).toContainText('does not exist'); expect(await page.locator('.tab-entry.active').getAttribute('data-tab-id')).toBe(active);
  await page.keyboard.press('Escape');await page.keyboard.press('Control+w');await page.keyboard.press('s'); await expect(panes()).toHaveCount(2);
  expect(errors).toEqual([]);
  console.log('Vim layout commands preserve pane geometry, tabs, active chat, invalid targets, and Ctrl-W split.');
} finally { await browser.close(); }
