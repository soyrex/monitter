import { webkit, expect } from '@playwright/test';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1320, height: 820 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18426');
  await expect(page.getByRole('button', { name: 'Open global overview', exact: true })).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => {
    const qa=window.__MONITTER_QA__,s=qa.snapshot(),now=Date.now();
    s.agents.push({ ...s.agents[0], id:'reviewer', name:'Reviewer', description:'Focused reviewer' });
    const task=(id,agentId,title,parentTaskId,status)=>({id,agentId,title,nativeSessionId:null,status,archived:false,createdAt:now,updatedAt:now,parentTaskId,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'});
    s.tasks=[task('parent','atlas','Parent chat',null,'running'),task('child-one','reviewer','Review tests','parent','completed'),task('child-two','reviewer','Inspect protocol','parent','running')];
    s.messages=[{id:'request',taskId:'parent',role:'user',text:'Delegate the review.',createdAt:now-3000}];
    s.collaborations=[
      {id:'done',kind:'delegation',fromAgentId:'atlas',fromTaskId:'parent',toAgentId:'reviewer',toTaskId:'child-one',text:'Review test coverage',requestId:'r1',status:'completed',result:'Found two missing cases.',error:null,createdAt:now-2000,updatedAt:now-1000},
      {id:'live',kind:'delegation',fromAgentId:'atlas',fromTaskId:'parent',toAgentId:'reviewer',toTaskId:'child-two',text:'Inspect the protocol',requestId:'r2',status:'running',result:null,error:null,createdAt:now-900,updatedAt:now-500},
    ];
    s.events=[
      {id:'spawn',taskId:'parent',kind:'collaboration',title:'Collaboration queued',detail:'done',createdAt:now-2000},
      {id:'finish',taskId:'parent',kind:'collaboration',title:'Collaboration completed',detail:'done',createdAt:now-1000},
      {id:'spawn-live',taskId:'parent',kind:'collaboration',title:'Collaboration queued',detail:'live',createdAt:now-900},
    ];
    qa.setSnapshot(s);
  });
  await page.getByRole('button',{name:/Parent chat/}).first().click();
  const pane=page.locator('.pane-leaf[data-pane-id="main"]');
  await expect(pane.getByText('Spawned Reviewer',{exact:true}).first()).toBeVisible();
  await expect(pane.getByText('Reviewer finished',{exact:true})).toBeVisible();
  const showSidebar=pane.getByRole('button',{name:'Show right sidebar',exact:true});
  if(await showSidebar.count()) await showSidebar.click();
  await pane.getByRole('tab',{name:/Subagents 2/}).click();
  await expect(pane.getByRole('region',{name:'Subagents'})).toContainText('1 running · 2 recent');
  await expect(pane.getByRole('region',{name:'Subagents'}).getByText('Found two missing cases.')).toBeVisible();
  expect(errors).toEqual([]);
  console.log('Rich subagent stream cards, running count, recent list and result preview passed.');
} finally { await browser.close(); }
