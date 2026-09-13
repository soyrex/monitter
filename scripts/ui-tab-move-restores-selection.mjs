import { chromium, webkit, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const task = (id, title) => ({ id, agentId:'atlas', title, nativeSessionId:null, status:'idle', archived:false, createdAt:1, updatedAt:1, parentTaskId:null, channelId:null, projectId:null, hostId:'local', cwd:'/tmp/monitter-ui-test', provider:'codex', model:'', sandbox:'read-only' });

for (const engine of [chromium, webkit]) {
  const browser = await engine.launch({ headless:true });
  try {
    const page = await browser.newPage({ viewport:{ width:1400, height:900 } });
    const errors=[]; page.on('pageerror', error=>errors.push(error.message));
    await page.addInitScript({ path:'scripts/ui-fixture.js' });
    await page.addInitScript(items=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks.push(...items);q.setSnapshot(s);}, [task('first','First chat'),task('previous','Previously active'),task('last','Last in tab order')]);
    await page.goto(url,{waitUntil:'domcontentloaded',timeout:240_000});
    await expect(page.locator('.sidebar')).toBeVisible({timeout:240_000});
    for (const title of ['First chat','Previously active','Last in tab order']) await page.locator(`.sidebar .task-select[title="${title}"]`).click();
    // History, rather than visual order, must choose this tab after the move.
    await page.locator('.tab-entry[data-tab-id="previous"] .tab').click();
    await page.getByRole('button',{name:'Open terminal'}).click();
    const terminal=page.locator('.tab-entry[data-tab-kind="terminal"]');
    await expect(terminal).toHaveCount(1);
    const from=await terminal.locator('.tab').boundingBox(),to=await page.locator('.pane-leaf').boundingBox();
    await page.mouse.move(from.x+from.width/2,from.y+from.height/2);await page.mouse.down();
    await page.mouse.move(to.x+to.width*.98,to.y+to.height/2,{steps:14});await page.mouse.up();
    await expect(page.locator('.pane-leaf')).toHaveCount(2);
    const source=page.locator('.pane-leaf').filter({has:page.locator('[data-tab-id="first"]')});
    await expect(source.locator('.tab-entry[data-tab-id="previous"]')).toHaveClass(/active/);
    await expect(source.locator('.dashboard-overview')).toHaveCount(0);
    await expect(page.locator('.pane-leaf').filter({has:page.locator('.terminal-tab')})).toHaveCount(1);
    expect(errors).toEqual([]);
    console.log(`${engine.name()}: moving active terminal restores prior source selection`);
  } finally { await browser.close(); }
}
