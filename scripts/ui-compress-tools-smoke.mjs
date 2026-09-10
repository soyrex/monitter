import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1500,height:1000}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks=[{id:'tools',agentId:'atlas',title:'Tool compression',nativeSessionId:null,status:'completed',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];s.events=Array.from({length:9},(_,i)=>({id:'tool-'+i,taskId:'tools',kind:'tool',title:'command '+i,detail:'output '+i,createdAt:n+i*575}));q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');
 await page.locator('.sidebar .task-select').click();
 await expect(page.locator('.activity-trigger')).toHaveCount(9);
 await page.getByRole('button',{name:'Preferences',exact:true}).click();
 await page.locator('.settings-nav').getByRole('button',{name:'Conversation',exact:true}).click();
 await page.getByRole('switch',{name:'Compress tool calls',exact:true}).check();
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.compressToolCalls)).toBe(true);
 await page.locator('.sidebar .task-select').click();
 const summary=page.locator('.activity-trigger');await expect(summary).toHaveCount(1);await expect(summary).toContainText('9 tool calls');await expect(summary).toContainText('4.6s');
 await summary.click();const box=page.getByRole('region',{name:'Expanded tool calls'});await expect(box).toBeVisible();await expect(box.locator('.call')).toHaveCount(9);await expect(page.getByRole('dialog',{name:/activity history/})).toHaveCount(0);
 await box.locator('pre').first().click();await expect(box).toBeVisible();await summary.click();await expect(box).toHaveCount(0);
 await page.getByRole('button',{name:'Preferences',exact:true}).click();await page.getByRole('switch',{name:'Compress tool calls',exact:true}).uncheck();await page.locator('.sidebar .task-select').click();await expect(page.locator('.activity-trigger')).toHaveCount(9);
 console.log('Compression toggle, 9-call count, 4.6s span, inline expansion and restoration passed.');
}finally{await browser.close();}
