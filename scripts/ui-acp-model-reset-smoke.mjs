import {chromium,expect} from '@playwright/test';

const browser=await chromium.launch({headless:true});
try{
  const page=await browser.newPage({viewport:{width:1440,height:900}});
  await page.addInitScript({path:'scripts/ui-fixture.js'});
  await page.addInitScript(()=>{
    const q=window.__MONITTER_QA__,s=q.snapshot(),now=Date.now();
    const saved={model:'missing-provider/Missing',reasoningEffort:null,fastMode:null};
    s.tasks=[{id:'jammed-acp-chat',agentId:'atlas',title:'Jammed ACP chat',nativeSessionId:'ses-existing',status:'error',archived:false,createdAt:now,updatedAt:now,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'acp',model:saved.model,sandbox:'workspace-write',modelSettings:saved}];
    s.messages=[];
    s.modelCatalog={models:[],current:saved,source:'Unavailable ACP session',warning:'Models become available after the ACP agent initializes a chat and advertises its choices.'};
    q.setSnapshot(s);
  });
  await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18433',{timeout:60000});
  await page.locator('.sidebar .task-select').filter({hasText:'Jammed ACP chat'}).click();
  const main=page.locator('.pane-leaf[data-pane-id="main"]');
  await expect(main.getByRole('button',{name:'Model: missing-provider/Missing',exact:true})).toBeVisible({timeout:60000});
  await main.getByRole('button',{name:'Model: missing-provider/Missing',exact:true}).click();
  const menu=page.getByRole('dialog',{name:'Model and reasoning',exact:true});
  await expect(menu.getByText(/Clear the saved choice below/)).toBeVisible();
  const reset=menu.getByRole('button',{name:'Use harness defaults'});
  await expect(reset).toBeEnabled();
  await reset.evaluate(button=>button.click());
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.calls.filter(call=>call.method==='setTaskModelSettings').at(-1)?.args.settings)).toEqual({model:'',reasoningEffort:null,fastMode:null});
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks[0].model)).toBe('');
  expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks[0].nativeSessionId)).toBe('ses-existing');
  console.log('A stopped ACP chat can clear an unavailable saved model without a catalog or session replay.');
}finally{
  await browser.close();
}
