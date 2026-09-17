import { webkit, expect } from '@playwright/test';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1320, height: 820 } });
const errors = [];
page.on('pageerror', error => {
  // A production browser preview has no Tauri callback registry. The fixture
  // supplies the complete Monitter bridge; ignore only this known shell probe.
  if (!error.message.includes("window.__TAURI_INTERNALS__.transformCallback")) errors.push(error.message);
});
try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18426');
  await expect(page.getByRole('button', { name: 'New chat', exact: true }).first()).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__,s=qa.snapshot(),now=Date.now();
    s.agents.push({ ...s.agents[0], id:'reviewer', name:'Reviewer', description:'Focused reviewer' });
    const task=(id,agentId,title,parentTaskId,status)=>({id,agentId,title,nativeSessionId:null,status,archived:false,createdAt:now,updatedAt:now,parentTaskId,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'});
    s.tasks=[task('parent','atlas','Parent chat',null,'running'),task('child-one','reviewer','Review tests','parent','completed'),task('child-two','reviewer','Inspect protocol','parent','running')];
    s.messages=[
      {id:'request',taskId:'parent',role:'user',text:'Delegate the review.',createdAt:now-3000},
      {id:'child-request',taskId:'child-two',role:'user',text:'Inspect the protocol',createdAt:now-850},
      {id:'child-update',taskId:'child-two',role:'assistant',text:'Reading the transport boundary now.',phase:'commentary',createdAt:now-450},
    ];
    s.collaborations=[
      {id:'done',kind:'delegation',fromAgentId:'atlas',fromTaskId:'parent',toAgentId:'reviewer',toTaskId:'child-one',text:'Review test coverage',requestId:'r1',status:'completed',result:'Found two missing cases.',error:null,createdAt:now-2000,updatedAt:now-1000},
      {id:'live',kind:'delegation',fromAgentId:'atlas',fromTaskId:'parent',toAgentId:'reviewer',toTaskId:'child-two',text:'Inspect the protocol',requestId:'r2',status:'running',result:null,error:null,createdAt:now-900,updatedAt:now-500},
    ];
    s.subagentSessions=[
      {id:'collaboration:done',source:'collaboration',parentTaskId:'parent',parentThreadId:null,collaborationId:'done',agentPath:null,agentThreadId:null,prompt:'Review test coverage',model:null,reasoningEffort:null,status:'completed',result:'Found two missing cases.',error:null,createdAt:now-2000,updatedAt:now-1000},
      {id:'collaboration:live',source:'collaboration',parentTaskId:'parent',parentThreadId:null,collaborationId:'live',agentPath:null,agentThreadId:null,prompt:'Inspect the protocol',model:null,reasoningEffort:null,status:'running',result:null,error:null,createdAt:now-900,updatedAt:now-500},
      {id:'codex:native-review',source:'codex',parentTaskId:'parent',parentThreadId:'parent-thread',collaborationId:null,agentPath:'/root/accessibility_review',agentThreadId:'native-review',prompt:'Inspect accessibility and report.',model:'gpt-test',reasoningEffort:'high',status:'running',result:null,error:null,createdAt:now-800,updatedAt:now-400},
    ];
    s.events=[
      {id:'spawn',taskId:'parent',kind:'collaboration',title:'Collaboration queued',detail:'done',createdAt:now-2000},
      {id:'finish',taskId:'parent',kind:'collaboration',title:'Collaboration completed',detail:'done',createdAt:now-1000},
      {id:'spawn-live',taskId:'parent',kind:'collaboration',title:'Collaboration queued',detail:'live',createdAt:now-900},
      {id:'native-live',taskId:'parent',kind:'subagent',title:'Sub-agent activity',detail:JSON.stringify({type:'subAgentActivity',id:'native-item',kind:'interacted',agentPath:'/root/accessibility_review',agentThreadId:'native-review'}),createdAt:now-400},
    ];
    s.subagentTranscripts={
      'codex:native-review':[
        {id:'native-user',role:'user',text:'Inspect accessibility and report.',createdAt:now-800},
        {id:'native-reasoning',role:'reasoning',text:'Checking keyboard and focus behavior.',createdAt:now-500},
      ],
    };
    qa.setSnapshot(s);
  });
  await page.getByRole('button',{name:/Parent chat/}).first().click();
  const pane=page.locator('.pane-leaf[data-pane-id="main"]');
  await expect(pane.getByRole('tablist',{name:'Subagent tasks'})).toBeVisible();
  await expect(pane.getByRole('tablist',{name:'Subagent tasks'}).getByText('Review tests',{exact:true})).toHaveCount(0);
  await expect(pane.getByRole('tablist',{name:'Subagent tasks'}).getByText('Inspect accessibility and report.',{exact:true})).toBeVisible();
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__,s=qa.snapshot(),now=Date.now();
    s.subagentSessions.push({id:'codex:native-second',source:'codex',parentTaskId:'parent',parentThreadId:'parent-thread',collaborationId:null,agentPath:'/root/second_live_review',agentThreadId:'native-second',prompt:'Review lifecycle updates',model:'gpt-test',reasoningEffort:'low',status:'running',result:null,error:null,createdAt:now,updatedAt:now});
    s.events.push({id:'native-second-live',taskId:'parent',kind:'subagent',title:'Sub-agent activity',detail:JSON.stringify({type:'subAgentActivity',id:'native-second-item',kind:'started',agentPath:'/root/second_live_review',agentThreadId:'native-second'}),createdAt:now});
    qa.setSnapshot(s);
  });
  await expect(pane.getByRole('tablist',{name:'Subagent tasks'}).getByText('Review lifecycle updates',{exact:true})).toBeVisible();
  expect(await pane.locator('.composer-area').evaluate(node => node.lastElementChild?.classList.contains('subagent-dock'))).toBe(true);
  const closedTabsY=(await pane.getByRole('tablist',{name:'Subagent tasks'}).boundingBox()).y;
  await pane.getByRole('tablist',{name:'Subagent tasks'}).getByRole('tab',{name:/Inspect accessibility and report/}).click();
  const visor=pane.getByRole('region',{name:'Subagent activity'});
  await page.waitForTimeout(300);
  const openTabsY=(await pane.getByRole('tablist',{name:'Subagent tasks'}).boundingBox()).y;
  expect(openTabsY).toBeLessThan(closedTabsY-80);
  await expect(visor).toContainText('Assignment');
  await expect(visor).toContainText('Inspect accessibility and report.');
  await expect(visor).toContainText('gpt-test · high');
  await expect(visor).toContainText('Checking keyboard and focus behavior.');
  const showSidebar=pane.getByRole('button',{name:'Show right sidebar',exact:true});
  if(await showSidebar.count()) await showSidebar.click();
  await pane.getByRole('tab',{name:/Subagents 4/}).click();
  const sidebar=pane.getByRole('region',{name:'Subagents'});
  await expect(sidebar.getByRole('heading',{name:'Active 3'})).toBeVisible();
  await expect(sidebar.getByRole('heading',{name:'Recent 1'})).toBeVisible();
  await expect(sidebar.getByText('Review tests',{exact:true})).toBeVisible();
  await expect(sidebar.getByText('Inspect protocol',{exact:true})).toBeVisible();
  await expect(sidebar.getByText('Inspect accessibility and report.',{exact:true})).toBeVisible();
  await expect(sidebar).not.toContainText('collaboration:');
  await expect(sidebar).not.toContainText('codex:native-review');
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__,s=qa.snapshot(),now=Date.now();
    const native=s.subagentSessions.find(item=>item.id==='codex:native-review');
    native.status='completed'; native.result='Accessibility review complete.'; native.updatedAt=now;
    s.events.push({id:'native-finished',taskId:'parent',kind:'subagent',title:'Sub-agent activity',detail:JSON.stringify({type:'subAgentActivity',id:'native-finished',kind:'completed',agentPath:'/root/accessibility_review',agentThreadId:'native-review'}),createdAt:now});
    qa.setSnapshot(s);
  });
  await expect(pane.getByRole('tablist',{name:'Subagent tasks'}).getByText('Inspect accessibility and report.',{exact:true})).toHaveCount(0);
  await expect(sidebar.getByRole('heading',{name:'Active 2'})).toBeVisible();
  await expect(sidebar.getByRole('heading',{name:'Recent 2'})).toBeVisible();
  await expect(sidebar.getByText('Inspect accessibility and report.',{exact:true})).toBeVisible();
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__,s=qa.snapshot(),now=Date.now();
    for (const item of s.subagentSessions.filter(item=>item.parentTaskId==='parent')) {
      item.status='completed'; item.updatedAt=now;
    }
    for (const item of s.collaborations.filter(item=>item.fromTaskId==='parent')) {
      item.status='completed'; item.updatedAt=now;
    }
    qa.setSnapshot(s);
  });
  await expect(pane.locator('.subagent-dock')).toHaveCount(0);
  if (process.env.MONITTER_SCREENSHOT) await page.screenshot({path:process.env.MONITTER_SCREENSHOT,fullPage:true});
  expect(errors).toEqual([]);
  console.log('Sliding live transcript visor and completed-to-recent lifecycle passed.');
} finally { await browser.close(); }
