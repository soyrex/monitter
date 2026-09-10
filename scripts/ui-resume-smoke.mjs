import {chromium,expect} from '@playwright/test';
import {mkdirSync,writeFileSync} from 'node:fs';
mkdirSync('verification',{recursive:true});
const browser=await chromium.launch({headless:true}),passed=[],errors=[];
const page=await browser.newPage({viewport:{width:1200,height:800}});page.on('pageerror',e=>errors.push(e.message));
const calls=()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='resumeTask'));
try{
  await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18421');await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible();
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot(),now=Date.now();s.tasks.push({id:'resume-chat',agentId:'atlas',title:'Resume this chat',nativeSessionId:'saved-native-id',status:'interrupted',archived:false,createdAt:now,updatedAt:now,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'});q.setSnapshot(s)});
  await page.locator('.sidebar .task-select').filter({hasText:'Resume this chat'}).click();
  const input=page.getByLabel('Task message',{exact:true});await input.fill('Keep my draft');
  await page.evaluate(()=>{const b=window.__MONITTER_BRIDGE__,original=b.resumeTask;let release;const pending=new Promise(r=>release=r);window.__MONITTER_QA__.release=release;b.resumeTask=async(...args)=>{b.resumeTask=original;await pending;return original(...args)}});
  await page.getByRole('button',{name:'Resume',exact:true}).click();await expect(page.locator('.composer-footer button[aria-label]').last()).toHaveAccessibleName('Starting task');
  await page.evaluate(()=>window.__MONITTER_QA__.release());await expect.poll(()=>calls().then(c=>c.length)).toBe(1);
  expect((await calls())[0].args.taskId).toBe('resume-chat');await expect(input).toHaveValue('Keep my draft');await expect(page.getByRole('button',{name:'Resume',exact:true})).toHaveCount(0);
  expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks[0].nativeSessionId)).toBe('saved-native-id');
  expect(await page.evaluate(()=>window.__MONITTER_QA__.calls.some(c=>c.method==='getResumeCommand'))).toBe(false);
  passed.push('Resume starts the same native task, displays startup, and preserves the unsent draft without a terminal command');
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks[0].status='completed';q.setSnapshot(s)});
  await input.fill('/resume');await input.press('Enter');await expect.poll(()=>calls().then(c=>c.length)).toBe(2);await expect(input).toHaveValue('');
  passed.push('/resume uses the same background continuation action');
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks[0].status='error';s.resumeFailure='Saved session is unavailable';q.setSnapshot(s)});
  await input.fill('A draft after failure');await page.getByRole('button',{name:'Resume',exact:true}).click();await expect(page.getByText('Saved session is unavailable',{exact:true})).toBeVisible();await expect(input).toHaveValue('A draft after failure');
  passed.push('resume errors are visible and leave drafts intact');expect(errors).toEqual([]);
  console.log(JSON.stringify({passed,errors},null,2));writeFileSync('verification/ui-resume-results.json',JSON.stringify({passed,errors},null,2));
}catch(error){writeFileSync('verification/ui-resume-results.json',JSON.stringify({passed,errors,error:String(error)},null,2));await page.screenshot({path:'verification/ui-resume-failure.png'});throw error}finally{await browser.close()}
