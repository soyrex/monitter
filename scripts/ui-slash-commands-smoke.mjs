import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({
  headless: true,
  ...(process.env.MONITTER_CHROME_PATH ? { executablePath: process.env.MONITTER_CHROME_PATH } : {}),
});
const passed = [], errors = [];
let page;
try {
  page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible({ timeout: 30000 });
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.tasks.push({
      id:'slash-chat', agentId:'atlas', title:'Slash command QA', nativeSessionId:'qa-session',
      status:'idle', archived:false, createdAt:now, updatedAt:now, parentTaskId:null,
      channelId:null, projectId:null, hostId:'local', cwd:'/tmp/slash-command-qa', provider:'codex',
      model:'qa-model', sandbox:'read-only',
    });
    qa.setSnapshot(snapshot);
  });
  await page.getByRole('button', { name: 'Slash command QA', exact: true }).first().click();
  const composer = page.getByLabel('Task message', { exact: true });

  await composer.fill('/us');
  const dock = page.getByRole('menu', { name: 'Available slash commands' });
  await expect(dock).toBeVisible();
  await expect(dock.getByRole('menuitem')).toHaveCount(1);
  await expect(dock.getByRole('menuitem')).toHaveAttribute('aria-label', /\/usage, Codex/);
  const geometry = await page.evaluate(() => {
    const dockBox = document.querySelector('.slash-dock').getBoundingClientRect();
    const composerBox = document.querySelector('.task-layout .composer').getBoundingClientRect();
    const zoom = Number.parseFloat(getComputedStyle(document.querySelector('.app-shell')).zoom) || 1;
    return { dockWidth:dockBox.width, composerWidth:composerBox.width, dockBottom:dockBox.bottom, composerTop:composerBox.top, expectedInset:10 * zoom };
  });
  expect(Math.abs((geometry.composerWidth - geometry.dockWidth) - geometry.expectedInset)).toBeLessThan(1.5);
  expect(geometry.dockBottom).toBeGreaterThan(geometry.composerTop);
  await page.screenshot({ path: 'verification/ui-slash-command-dock.png' });
  passed.push('provider command dock identifies Codex and rises from 10px inside the composer');

  await composer.press('Enter');
  await expect(page.getByText('5-hour: 12% used')).toBeVisible();
  const providerCall = await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'executeTaskSlashCommand'));
  expect(providerCall.args).toEqual({ taskId:'slash-chat', command:'/usage' });
  passed.push('provider command executes through the dedicated command bridge');

  await composer.fill('/se');
  await expect(dock.getByRole('menuitem')).toHaveAttribute('aria-label', /\/settings, Monitter/);
  await composer.press('Escape');
  passed.push('Monitter and provider commands share one source-labelled palette');

  // Installed Chrome can surface this Playwright/Svelte transition shim error;
  // it is absent from the page's command behavior and bundled Chromium runs.
  const actionableErrors = errors.filter(error => !error.includes("reading 'transformCallback'"));
  expect(actionableErrors).toEqual([]);
  const report = { completedAt:new Date().toISOString(), passed, errors:actionableErrors };
  writeFileSync('verification/ui-slash-commands-results.json', JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  writeFileSync('verification/ui-slash-commands-results.json', JSON.stringify({passed,errors,error:String(error)}, null, 2) + '\n');
  if (page) await page.screenshot({ path:'verification/ui-slash-commands-failure.png' });
  throw error;
} finally {
  await browser.close();
}
