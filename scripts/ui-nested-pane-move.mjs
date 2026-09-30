import { chromium, expect } from '@playwright/test';
const browser = await chromium.launch({headless:true});
try {
  const page = await browser.newPage({viewport:{width:1500,height:1000}});
  const errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  const kind=process.env.TAB_KIND ?? 'task';
  await page.addInitScript({path:'scripts/ui-fixture.js'});
  if(kind==='task') await page.addInitScript(()=>{
    const s=window.__MONITTER_QA__.snapshot();
    s.tasks=[0,1].map(i=>({id:`chat-${i}`,agentId:'atlas',title:`Regression chat ${i}`,status:'idle',archived:false,createdAt:1,updatedAt:1,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}));
    window.__MONITTER_QA__.setSnapshot(s);
  });
  page.setDefaultTimeout(180000);
  await page.goto(process.env.TEST_URL ?? 'http://127.0.0.1:18463', {timeout:180000});
  const panes=page.locator('.pane-leaf');
  await page.getByRole('button',{name:'Open terminal',exact:true}).waitFor();
  page.setDefaultTimeout(10000);
  const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');
  for(let i=0;i<2;i++) {
    if(kind==='task') await page.locator('.sidebar').getByRole('button',{name:`Regression chat ${i}`,exact:true}).click();
    else if(kind==='terminal') await page.getByRole('button',{name:'Open terminal',exact:true}).click();
    else if(kind==='empty') await page.keyboard.press(`${modifier}+t`);
    else {
      await page.keyboard.press(`${modifier}+Shift+c`);
      await page.getByRole('textbox',{name:'Task message',exact:true}).fill(`Test draft ${i}`);
    }
  }
  const tabs=page.locator(`.tab-entry[data-tab-kind="${kind}"]`);
  await expect(tabs).toHaveCount(2);
  const ids=await tabs.evaluateAll(nodes=>nodes.map(n=>n.dataset.tabId));
  async function split(id,target,edge) {
    const a=await page.locator(`[data-tab-id="${id}"] .tab`).boundingBox(),b=await target.boundingBox();
    await page.mouse.move(a.x+a.width/2,a.y+a.height/2);
    await page.mouse.down();
    await page.mouse.move(b.x+b.width*(edge==='right'?.98:.5),b.y+b.height*(edge==='bottom'?.98:.5),{steps:20});
    await page.mouse.up();
  }
  await split(ids[1],panes.first(),'right');
  await expect(panes).toHaveCount(2);
  await expect(tabs).toHaveCount(2);
  await split(ids[0],panes.last(),'bottom');
  await expect(panes).toHaveCount(2);
  for(const id of ids) await expect(page.locator(`.tab-entry[data-tab-id="${id}"]`)).toHaveCount(1);
  await expect(page.locator('.pane-split.column')).toHaveCount(1);
  await page.reload();
  await expect(panes).toHaveCount(2);
  await expect(tabs).toHaveCount(2);
  expect(errors).toEqual([]);
  console.log(`${kind}: nested split preserves both tabs, removes empty source and survives reload without runtime errors.`);
} finally { await browser.close(); }
