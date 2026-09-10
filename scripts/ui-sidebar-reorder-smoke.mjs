import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`
 const q=window.__MONITTER_QA__,s=q.snapshot(),now=Date.now();
 s.agents.push({...s.agents[0],id:'beta',name:'Beta'});
 s.projects=[{id:'p1',name:'First project',description:'',workspaces:[]},{id:'p2',name:'Second project',description:'',workspaces:[]}];
 s.channels=['one','two'].map(id=>({id,name:id,description:'',agentIds:['atlas'],messages:[]}));
 s.tasks=['a','b','c'].map((id,i)=>({id,agentId:'atlas',title:'Chat '+id,nativeSessionId:null,status:'idle',archived:false,createdAt:now,updatedAt:now+i,parentTaskId:null,channelId:null,projectId:'p1',hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'}));q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');
 const row=(group,id)=>page.locator(`[data-sidebar-sort-group="${group}"][data-sidebar-sort-id="${id}"]`);
 const order=group=>page.locator(`[data-sidebar-sort-group="${group}"]`).evaluateAll(nodes=>nodes.map(n=>n.dataset.sidebarSortId));
 async function drag(source,target,after=false){const a=await source.boundingBox(),b=await target.boundingBox();await page.mouse.move(a.x+Math.min(90,a.width/2),a.y+a.height/2);await page.mouse.down();await page.mouse.move(b.x+Math.min(90,b.width/2),b.y+b.height*(after?.85:.15),{steps:12});await expect(target).toHaveAttribute('data-sort-edge',after?'after':'before');await page.mouse.up();}
 await drag(row('agents','beta'),row('agents','atlas'));expect(await order('agents')).toEqual(['beta','atlas']);
 await drag(row('agent-chats:atlas','c'),row('agent-chats:atlas','a'));expect(await order('agent-chats:atlas')).toEqual(['c','a','b']);
 await drag(row('channels','two'),row('channels','one'));expect(await order('channels')).toEqual(['two','one']);
 const cancelSource=await row('channels','two').boundingBox(),cancelTarget=await row('channels','one').boundingBox();
 await page.mouse.move(cancelSource.x+70,cancelSource.y+cancelSource.height/2);await page.mouse.down();await page.mouse.move(cancelTarget.x+70,cancelTarget.y+cancelTarget.height*.9,{steps:8});await page.keyboard.press('Escape');await page.mouse.up();expect(await order('channels')).toEqual(['two','one']);await expect(page.locator('[data-sort-edge]')).toHaveCount(0);

 await expect(page.locator('.tab-entry:not(.dashboard-tab)')).toHaveCount(0);
 await page.reload();await expect(row('agents','atlas')).toBeVisible();expect(await order('agents')).toEqual(['beta','atlas']);expect(await order('agent-chats:atlas')).toEqual(['c','a','b']);expect(await order('channels')).toEqual(['two','one']);
 await row('agent-chats:atlas','a').locator('.task-select').click();await expect(page.getByRole('textbox',{name:'Task message',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Projects view',exact:true}).click();
 await drag(row('projects','p2'),row('projects','p1'));expect(await order('projects')).toEqual(['p2','p1']);
 await drag(row('project-chats:p1','a'),row('project-chats:p1','c'));expect(await order('project-chats:p1')).toEqual(['a','c','b']);
 expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks.map(t=>[t.agentId,t.projectId]))).toEqual([['atlas','p1'],['atlas','p1'],['atlas','p1']]);
 await page.getByRole('button',{name:'Activity view',exact:true}).click();expect(await page.locator('.activity-list .task-row').evaluateAll(nodes=>nodes.map(n=>n.dataset.taskId))).toEqual(['c','b','a']);
 expect(errors).toEqual([]);console.log('Pointer reordering: agents, chats, channels, projects; persistence, click navigation, unchanged ownership and chronological Activity passed.');
}finally{await browser.close();}
