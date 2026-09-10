import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1500,height:850}}), errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();for(let i=0;i<40;i++)s.tasks.push({id:'chat-'+i,agentId:'atlas',title:'Chat '+i,nativeSessionId:null,status:'idle',archived:false,createdAt:n-i,updatedAt:n-i,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'});s.messages.push({id:'user1',taskId:'chat-0',role:'user',text:'My message',createdAt:n});q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');
 const footer=page.locator('.sidebar-footer'), list=page.locator('.side-scroll'), brand=page.locator('.sidebar .brand');
 await expect(footer.getByRole('button')).toHaveCount(4);await expect(page.locator('.sidebar .main-sidebar-control')).toHaveCount(0);await expect(page.locator('.topbar > .main-sidebar-control')).toHaveCount(1);await expect(brand.getByRole('group',{name:'Sidebar view'})).toBeVisible();await page.getByRole('button',{name:'Collapse main sidebar',exact:true}).click();await page.getByRole('button',{name:'Expand main sidebar',exact:true}).click();
 await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toHaveCount(0);
 await expect(page.getByRole('button',{name:'Pane layout',exact:true})).toHaveCount(0);
 const fixed=await footer.boundingBox();await expect(brand).not.toHaveClass(/scrolled/);
 await list.evaluate(el=>el.scrollTop=el.scrollHeight);await expect(brand).toHaveClass(/scrolled/);expect(await footer.boundingBox()).toEqual(fixed);
 await list.evaluate(el=>el.scrollTop=0);await expect(brand).not.toHaveClass(/scrolled/);
 await footer.getByRole('button',{name:'Hosts',exact:true}).click();await expect(page.getByRole('dialog')).toBeVisible();await page.keyboard.press('Escape');
 await footer.getByRole('button',{name:'Archived chats',exact:true}).click();await expect(page.getByRole('dialog')).toBeVisible();await page.keyboard.press('Escape');
 await footer.getByRole('button',{name:'Agent directory',exact:true}).click();await expect(page.getByRole('textbox',{name:'Find agents'})).toBeVisible();await expect(page.getByRole('dialog')).toHaveCount(0);await expect(page.locator('.settings-tab')).toHaveCount(1);
 await page.locator('.settings-nav').getByRole('button',{name:'Conversation',exact:true}).click();const tint=page.getByRole('switch',{name:'Tint my messages'});await expect(tint).not.toBeChecked();await tint.check();await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.tintUserMessages)).toBe(true);
 await page.locator('.sidebar .task-select').filter({hasText:'Chat 0'}).click();await expect(page.locator('.message.user.tinted')).toHaveCount(1);
 await footer.getByRole('button',{name:'Preferences',exact:true}).click();await expect(tint).toBeChecked();await tint.uncheck();await page.locator('.sidebar .task-select').filter({hasText:'Chat 0'}).click();await expect(page.locator('.message.user.tinted')).toHaveCount(0);
 expect(errors).toEqual([]);console.log('Fixed sidebar controls, conditional scroll border, Settings directory and conversation tint passed.');
} finally {await browser.close();}
