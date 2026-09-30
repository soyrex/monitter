import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18423/monitter-app-ui/';
const browser = await chromium.launch({ headless: true, executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url, { timeout: 90_000 });
  await expect(page.locator('.pane-leaf[data-pane-id="main"]')).toBeVisible({ timeout: 60_000 });
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    for (const id of ['left-chat', 'right-chat']) snapshot.tasks.push({
      id, agentId: 'atlas', title: id, nativeSessionId: null, status: 'idle', archived: false,
      createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null,
      hostId: 'local', cwd: '/tmp/monitter-composer-test', provider: 'codex', model: '', sandbox: 'read-only',
    });
    qa.setSnapshot(snapshot);
  });
  const main = page.locator('.pane-leaf[data-pane-id="main"]');
  for (const title of ['left-chat', 'right-chat']) {
    await page.locator('.sidebar .task-select').filter({ hasText: title }).click();
    await expect(main.locator('.tabs').getByRole('button', { name: title, exact: true })).toBeVisible();
  }
  await main.click({ position: { x: 100, y: 100 } });
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+p' : 'Control+p');
  await page.getByRole('dialog', { name: 'Controls', exact: true }).getByText('Two columns', { exact: true }).click();
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  const right = page.locator('.pane-leaf').last();
  const tab = main.locator('.tabs').getByRole('button', { name: 'right-chat', exact: true });
  const dataTransfer = await page.evaluateHandle(() => new DataTransfer());
  await tab.dispatchEvent('dragstart', { dataTransfer });
  const target = await right.boundingBox();
  await right.dispatchEvent('dragover', { clientX: target.x + target.width / 2, clientY: target.y + target.height / 2, dataTransfer });
  await right.dispatchEvent('drop', { clientX: target.x + target.width / 2, clientY: target.y + target.height / 2, dataTransfer });
  await tab.dispatchEvent('dragend').catch(() => {});
  await dataTransfer.dispose();
  await expect(right.locator('.tabs').getByRole('button', { name: 'right-chat', exact: true })).toBeVisible();

  await right.getByRole('button', { name: 'Expand composer' }).click();
  const expanded = right.locator('.composer.expanded');
  await expect(expanded).toBeVisible();
  const measure = () => right.evaluate(pane => {
    const chat = pane.querySelector('.conversation').getBoundingClientRect();
    const composer = pane.querySelector('.composer.expanded').getBoundingClientRect();
    const header = pane.querySelector('.pane-task-header').getBoundingClientRect();
    const surface = getComputedStyle(pane).backgroundColor;
    const style = getComputedStyle(pane.querySelector('.composer.expanded'));
    const blockout = getComputedStyle(pane.querySelector('.workspace'), '::after').content;
    return {
      left: Math.abs(composer.left - chat.left), right: Math.abs(composer.right - chat.right),
      top: Math.abs(composer.top - chat.top), bottom: Math.abs(composer.bottom - chat.bottom),
      belowHeader: composer.top >= header.bottom - 2,
      background: style.backgroundColor, surface, shadow: style.boxShadow,
      backdrop: style.backdropFilter, blockout,
    };
  });
  for (const size of [{ width: 1600, height: 900 }, { width: 1300, height: 760 }]) {
    await page.setViewportSize(size);
    await expect.poll(async () => {
      const geometry = await measure();
      return Math.max(geometry.left, geometry.right, geometry.top, geometry.bottom);
    }).toBeLessThanOrEqual(2);
    const geometry = await measure();
    assert.equal(geometry.belowHeader, true);
    assert.equal(geometry.background, geometry.surface);
    assert.equal(geometry.shadow, 'none');
    assert.equal(geometry.backdrop, 'none');
    assert.equal(geometry.blockout, 'none');
  }
  assert.equal(await main.locator('.composer.expanded').count(), 0, 'Neighbouring pane stays unexpanded');
  await right.getByRole('button', { name: 'Restore composer' }).click();
  await expect(right.locator('.composer.expanded')).toHaveCount(0);
  assert.deepEqual(errors, []);
  console.log('Right-pane expanded composer is pane-local and edge-to-edge at both viewport sizes');
} finally {
  await browser.close();
}
