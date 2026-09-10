import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}});
 await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');
 await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible({timeout:60000});
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks=[{id:'waiting',agentId:'atlas',title:'Waiting check',nativeSessionId:null,status:'running',archived:false,createdAt:1,updatedAt:1,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'}];s.messages=[{id:'request',taskId:'waiting',role:'user',text:'Hello',createdAt:1}];q.setSnapshot(s);});
 await page.locator('.sidebar .task-select').filter({hasText:'Waiting check'}).click();
 await expect(page.locator('.messages .agent-waiting')).toContainText('Atlas is pondering…');
 await expect(page.locator('.agent-waiting .message-avatar')).toBeVisible();
 await page.emulateMedia({reducedMotion:'reduce'});
 expect(await page.locator('.waiting-spinner').evaluate(el=>getComputedStyle(el).animationName)).toBe('none');
 for(const status of ['completed','error','interrupted']) {
  await page.evaluate(status=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks[0].status=status;q.setSnapshot(s);},status);
  await expect(page.locator('.agent-waiting')).toHaveCount(0);
 }
 console.log('Waiting indicator, avatar, reduced motion, completion/error/interruption checks passed.');
}finally{await browser.close();}
