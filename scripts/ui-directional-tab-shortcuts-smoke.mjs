import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true, ...(process.env.MONITTER_CHROME_CHANNEL ? { channel: process.env.MONITTER_CHROME_CHANNEL } : {}) });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18434';
const activePane = () => page.locator('.pane-leaf.active');
const tab = (pane, name) => pane.locator('.tabs').getByRole('button', { name, exact: true });

try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByLabel('Switch desktop workspace', { exact: true })).toBeVisible({ timeout: 30000 });
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    for (const id of ['A', 'B']) snapshot.tasks.push({ id, agentId: 'atlas', title: `Pane ${id}`, nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' });
    qa.setSnapshot(snapshot);
  });
  for (const name of ['Pane A', 'Pane B']) await page.locator('.sidebar .task-select').filter({ hasText: name }).click();
  const main = page.locator('.pane-leaf[data-pane-id="main"]');
  await main.getByLabel('Task message', { exact: true }).fill('B draft stays with tab');

  await page.keyboard.press(`${modifier}+Shift+ArrowRight`);
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  await expect(tab(main, 'Pane A')).toBeVisible();
  await expect(tab(activePane(), 'Pane B')).toBeVisible();
  await expect(activePane().getByLabel('Task message', { exact: true })).toHaveValue('B draft stays with tab');

  await page.keyboard.press(`${modifier}+Shift+ArrowDown`);
  await expect(page.locator('.pane-leaf')).toHaveCount(3);
  await expect(tab(activePane(), 'Pane B')).toBeVisible();
  const lowerId = await activePane().getAttribute('data-pane-id');
  await page.keyboard.press(`${modifier}+Alt+ArrowUp`);
  await expect(activePane()).not.toHaveAttribute('data-pane-id', lowerId);
  await page.keyboard.press(`${modifier}+Alt+ArrowDown`);
  await expect(activePane()).toHaveAttribute('data-pane-id', lowerId);

  await page.keyboard.press(`${modifier}+Shift+ArrowLeft`);
  await expect(tab(main, 'Pane B')).toBeVisible();
  await expect(activePane()).toHaveAttribute('data-pane-id', 'main');
  await page.keyboard.press(`${modifier}+Shift+ArrowUp`);
  await expect(page.locator('.pane-leaf')).toHaveCount(3);
  await expect(tab(activePane(), 'Pane B')).toBeVisible();
  await expect(tab(main, 'Pane A')).toBeVisible();
  await expect(activePane().getByLabel('Task message', { exact: true })).toHaveValue('B draft stays with tab');
  console.log('Directional tab moves, missing-pane splits, draft transfer, and vertical focus shortcuts passed.');
} finally {
  await browser.close();
}
