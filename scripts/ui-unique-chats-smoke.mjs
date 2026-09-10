import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1500,height:950}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks=[{id:'unique-chat',agentId:'atlas',title:'Unique chat',nativeSessionId:null,status:'completed',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];s.channels=[{id:'unique-channel',name:'Unique channel',description:'',agentIds:['atlas'],messages:[]}];q.setSnapshot(s);`});
 await page.addInitScript(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.shortcutMode='vim';q.setSnapshot(s);});await page.goto('http://127.0.0.1:18433');await page.locator('.sidebar .task-select').click({timeout:60000});
 await page.getByLabel('Task message',{exact:true}).fill('Keep this draft');
 await page.keyboard.press('Escape');await page.keyboard.type(':');const command=page.getByRole('textbox',{name:'Monitter command'});await command.fill('vsplit');await command.press('Enter');await expect(page.locator('.pane-leaf')).toHaveCount(2);
 const secondary=page.locator('.pane-leaf').last(),secondaryId=await secondary.getAttribute('data-pane-id');
 await page.locator('.sidebar .task-select').click();await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id','main');
 await expect(page.locator('.tab-entry[data-tab-kind="task"][data-tab-id="unique-chat"]')).toHaveCount(1);await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('Keep this draft');
 await secondary.click({position:{x:30,y:100}});await page.locator('.sidebar').getByRole('button',{name:/Unique channel/}).click();await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id',secondaryId);
 await page.locator('.sidebar .task-select').click();await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id','main');
 await page.locator('.sidebar').getByRole('button',{name:/Unique channel/}).click();await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id',secondaryId);
 await expect(page.locator('.tab-entry[data-tab-kind="channel"][data-tab-id="unique-channel"]')).toHaveCount(1);
 console.log('Sidebar focuses existing chat/channel owner across panes without duplicate tabs; unsent draft preserved.');
}finally{await browser.close();}
