import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18450';
const fixture = readFileSync('scripts/ui-fixture.js', 'utf8');

async function open(viewport) {
  const browser = await webkit.launch({ headless: true });
  const page = await browser.newPage({ viewport });
  await page.addInitScript({ content: `${fixture}
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.settings.autoHideTabs = true;
    snapshot.tasks.push({ id:'auto-hide-task', agentId:'atlas', title:'Auto-hide target', nativeSessionId:null, status:'idle', archived:false, createdAt:now, updatedAt:now, parentTaskId:null, channelId:null, projectId:null, hostId:'local', cwd:'/tmp/monitter-ui-test', provider:'codex', model:'', sandbox:'read-only' });
    qa.setSnapshot(snapshot);` });
  await page.goto(url);
  return { browser, page };
}

const desktop = await open({ width: 1200, height: 800 });
try {
  const workspace = desktop.page.locator('.workspace').first();
  const topbar = workspace.locator(':scope > .topbar');
  await expect(workspace).toHaveClass(/auto-hide-tabs/);
  const before = await topbar.boundingBox();
  const pane = await workspace.boundingBox();
  if (!before || !pane) throw new Error('Desktop pane did not render.');
  expect(before.y).toBeLessThan(pane.y);
  await desktop.page.mouse.move(pane.x + pane.width / 2, pane.y + 20);
  await expect(workspace).toHaveClass(/tab-revealed/);
  await expect.poll(async () => (await topbar.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(pane.y - 1);
  await desktop.page.mouse.move(pane.x + pane.width / 2, pane.y + 70);
  await expect(workspace).not.toHaveClass(/tab-revealed/);
  await expect.poll(async () => (await topbar.boundingBox())?.y ?? Infinity).toBeLessThan(pane.y);
  console.log('Desktop auto-hide: any mouse movement in the pane top 40px reveals tabs, and leaving it hides them.');
} finally { await desktop.browser.close(); }

const mobile = await open({ width: 390, height: 844 });
try {
  const workspace = mobile.page.locator('.workspace').first();
  const topbar = workspace.locator(':scope > .topbar');
  await expect(workspace).not.toHaveClass(/auto-hide-tabs/);
  const pane = await workspace.boundingBox(), bar = await topbar.boundingBox();
  if (!pane || !bar) throw new Error('Mobile pane did not render.');
  expect(bar.y).toBeGreaterThanOrEqual(pane.y - 1);
  console.log('Mobile auto-hide: tabs remain visible.');
} finally { await mobile.browser.close(); }
