import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1400,height:1000}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks=[{id:'queue',agentId:'atlas',title:'Queue edits',nativeSessionId:null,status:'running',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];s.queuedMessages=[{id:'q1',taskId:'queue',channelId:null,text:'Original',attachmentIds:['file-1'],createdAt:1,status:'queued'},{id:'q2',taskId:'queue',channelId:null,text:'Second',attachmentIds:[],createdAt:2,status:'queued'}];s.channels=[{id:'everyone',name:'Everyone',description:'',agentIds:['atlas'],messages:[]}];q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');await page.locator('.sidebar .task-select').click();
 const queue=page.getByRole('region',{name:'Queued messages'});await queue.getByRole('button',{name:/Edit queued message for/}).first().click();const input=queue.getByRole('textbox',{name:'Edit queued message',exact:true});await input.fill('Revised');await queue.getByRole('button',{name:'Save',exact:true}).click();await expect(input).toHaveCount(0);
 const saved=await page.evaluate(()=>window.__MONITTER_QA__.snapshot().queuedMessages);expect(saved.map(m=>m.id)).toEqual(['q1','q2']);expect(saved[0]).toMatchObject({text:'Revised',attachmentIds:['file-1'],createdAt:1,status:'queued'});
 await queue.getByRole('button',{name:/Edit queued message for/}).first().click();await input.fill('Discard this');await queue.getByRole('button',{name:'Cancel',exact:true}).click();await expect(queue).toContainText('Revised');
 await queue.getByRole('button',{name:/Edit queued message for/}).last().click();await input.fill('');await expect(queue.getByRole('button',{name:'Save',exact:true})).toBeDisabled();await input.fill('Keep unsaved draft');
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.queuedMessages[1].status='sending';q.setSnapshot(s);});await expect(queue.getByRole('button',{name:'Save',exact:true})).toBeDisabled();await expect(input).toHaveValue('Keep unsaved draft');await expect(queue.getByRole('status')).toContainText('has not been sent');
 await queue.getByRole('button',{name:'Cancel',exact:true}).click();
 await expect(page.locator('.composer')).not.toContainText('new line');
 await page.getByRole('complementary',{name:'Agents and tasks'}).getByRole('button',{name:/Everyone/}).click();const channel=page.getByRole('textbox',{name:'Channel message',exact:true});await channel.focus();expect(await channel.evaluate(el=>getComputedStyle(el).outlineStyle)).toBe('none');expect(await page.locator('.composer').evaluate(el=>getComputedStyle(el).boxShadow)).not.toBe('none');
 console.log('Queue save/cancel, attachment and order preservation, dispatch race, hints removal and channel focus styling passed.');
}finally{await browser.close();}
