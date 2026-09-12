import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch();
try {
  for (const mobile of [false, true]) {
    const page = await browser.newPage({ viewport: mobile ? {width:390,height:844} : {width:1280,height:900}, isMobile: mobile, hasTouch: mobile });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
      const qa=window.__MONITTER_QA__, state=qa.snapshot();
      state.tasks=[{id:'protocol-chat',agentId:'atlas',title:'Protocol chat',status:'running',archived:false,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only',nativeSessionId:'thread-1',createdAt:1,updatedAt:1}];
      state.messages=[{id:'stream-one',taskId:'protocol-chat',role:'assistant',text:'Hello',createdAt:10,streamStatus:'streaming'}];
      state.approvalRequests=[{id:'approval-one',taskId:'protocol-chat',provider:'codex',runId:'turn-1:request-1',tool:'commandExecution',summary:'Run a harmless command?',detail:'pwd',risk:'low',status:'pending',createdAt:11,resolvedAt:null,decision:null}];
      qa.setSnapshot(state);
      window.__MONITTER_BRIDGE__.resolveApproval=async(id,decision)=>{qa.calls.push({method:'resolveApproval',id,decision});const s=qa.snapshot();s.approvalRequests[0].status=decision==='deny'?'denied':'approved';qa.setSnapshot(s);return s;};
      window.__MONITTER_BRIDGE__.resolveInput=async(id,response)=>{qa.calls.push({method:'resolveInput',id,response});const s=qa.snapshot();s.approvalRequests[0].status='approved';s.approvalRequests[0].response=response;qa.setSnapshot(s);return s;};
    `);
    await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18443/');
    const select = page.locator('[data-task-id="protocol-chat"] .task-select');
    if (mobile) await select.tap(); else await select.click();
    const approve = page.getByRole('button', {name:'Approve once approval request',exact:true});
    if (mobile) await approve.tap(); else await approve.click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='resolveApproval').length)).toBe(1);
    await expect(page.getByRole('region',{name:'Approval history'})).toContainText('Approved once');
    await page.evaluate(() => {
      const qa=window.__MONITTER_QA__, s=qa.snapshot();
      s.messages[0].text='Hello from the same streaming message';
      s.messages[0].streamStatus='complete';
      s.approvalRequests[0]={...s.approvalRequests[0],id:'question-one',status:'pending',summary:'Choose a colour',input:{kind:'questions',schema:null,url:null,questions:[{id:'colour',header:'Colour',question:'Which colour?',isSecret:false,options:[{label:'Blue',description:'Use blue'}]}]}};
      qa.setSnapshot(s);
    });
    await expect(page.locator('article.message').filter({hasText:'Hello'})).toHaveCount(1);
    await expect(page.locator('article.message').filter({hasText:'Hello'})).toContainText('same streaming message');
    await page.getByLabel('Which colour?',{exact:true}).fill('Blue');
    const submit=page.getByRole('button',{name:'Submit response',exact:true});
    if(mobile) await submit.tap(); else await submit.click();
    await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='resolveInput').length)).toBe(1);
    const response=await page.evaluate(()=>window.__MONITTER_QA__.calls.find(c=>c.method==='resolveInput'));
    expect(response).toMatchObject({id:'question-one',response:{answers:{colour:{answers:['Blue']}}}});
    expect(errors).toEqual([]);
    await page.close();
  }
  console.log('App-server desktop/mobile WebKit approval, question, and streaming UI tests passed.');
} finally { await browser.close(); }
