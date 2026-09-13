import { chromium, webkit, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18468';
for (const engine of [chromium, webkit]) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1471, height: 900 } });
  page.setDefaultTimeout(60000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.addInitScript({ path: 'scripts/ui-fixture.js' });
    await page.goto(url, { timeout: 120000 });
    await page.locator('.workspace').waitFor();
    await page.evaluate(() => {
      const qa = window.__MONITTER_QA__, snapshot = qa.snapshot();
      for (const id of ['phone-tabs-a', 'phone-tabs-b']) snapshot.tasks.push({
        id, agentId: 'atlas', title: id, nativeSessionId: null, status: 'idle', archived: false,
        createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: null,
        hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
      });
      qa.setSnapshot(snapshot);
    });
    for (const id of ['phone-tabs-a', 'phone-tabs-b']) {
      await page.locator('.sidebar .task-select').filter({ hasText: id }).click();
    }
    const pane = page.locator('.pane-leaf[data-pane-id="main"]');
    const trigger = pane.locator('.tab-picker-trigger');
    for (const width of [390, 430, 431, 600, 760, 800, 1471, 2700, 390, 1471]) {
      await page.setViewportSize({ width, height: 900 });
      if (width <= 430) await expect(trigger).toBeVisible();
      else {
        await expect(trigger).toBeHidden();
        await expect(pane.locator('.tabs').getByRole('button', { name: 'phone-tabs-b', exact: true })).toBeVisible();
      }
    }
    // Pane width, not the desktop window width, controls the picker.
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+p' : 'Control+p');
    await page.getByRole('dialog', { name: 'Controls', exact: true }).getByText('Two columns', { exact: true }).click();
    await expect(page.locator('.pane-leaf')).toHaveCount(2);
    await expect(trigger).toBeHidden();
    await page.setViewportSize({ width: 1100, height: 900 });
    await expect.poll(() => pane.locator('.workspace').evaluate(node => node.clientWidth)).toBeLessThanOrEqual(430);
    await expect(trigger).toBeVisible();
    await trigger.click();
    await pane.locator('.tabs').getByRole('button', { name: 'phone-tabs-a', exact: true }).click();
    await expect(trigger).toContainText('phone-tabs-a');
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await page.setViewportSize({ width: 1471, height: 900 });
    await expect(trigger).toBeHidden();
    expect(errors).toEqual([]);
    console.log(`${engine.name()}: phone boundary, wider mobile layouts, wide desktop, split panes and dropdown selection passed`);
  } catch (error) {
    console.error({ engine: engine.name(), errors, body: (await page.locator('body').innerText()).slice(0, 1500) });
    throw error;
  } finally {
    await browser.close();
  }
}
