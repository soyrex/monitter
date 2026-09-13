import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}});
 await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');
 await expect(page.locator('.app-shell')).toBeVisible({timeout:60000});
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks=[{id:'waiting',agentId:'atlas',title:'Waiting check',nativeSessionId:null,status:'running',archived:false,createdAt:1,updatedAt:1,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'}];s.messages=[{id:'request',taskId:'waiting',role:'user',text:'Hello',createdAt:1}];q.setSnapshot(s);});
 await page.locator('.sidebar .task-select').filter({hasText:'Waiting check'}).click();
 await expect(page.locator('.messages .reasoning-pending')).toHaveCount(1);
 await expect(page.locator('.reasoning-pending .message-avatar')).toBeVisible();
 await expect(page.locator('.waiting-spinner')).toHaveCount(0);
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.events=[{id:'thinking',taskId:'waiting',kind:'reasoning',title:'Reasoning',detail:'',createdAt:2}];q.setSnapshot(s);});
 await expect(page.locator('.messages .reasoning-pending')).toHaveCount(1);
 await expect(page.locator('.reasoning-pending .message-avatar')).toBeVisible();
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.messages.push({id:'reply',taskId:'waiting',role:'assistant',text:'Reply is arriving',createdAt:3,streamStatus:'streaming'});q.setSnapshot(s);});
 await expect(page.locator('.reasoning-pending')).toHaveCount(0);
 for(const status of ['completed','error','interrupted']) {
  await page.evaluate(status=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks[0].status=status;q.setSnapshot(s);},status);
  await expect(page.locator('.reasoning-pending')).toHaveCount(0);
 }
 console.log('Single avatar/thinking row, no spinner, reply replacement and completion/error/interruption checks passed.');
}finally{await browser.close();}
