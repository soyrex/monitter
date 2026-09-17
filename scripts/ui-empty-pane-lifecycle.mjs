import { chromium, webkit, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const engines = process.env.LIFECYCLE_ENGINE === 'chromium' ? [chromium] : process.env.LIFECYCLE_ENGINE === 'webkit' ? [webkit] : [chromium, webkit];

const task = (id, title) => ({ id, agentId: 'atlas', title, nativeSessionId: null, status: 'idle', archived: false, createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' });

for (const engine of engines) {
  const browser = await engine.launch({ headless: true });
  try {
    for (const kind of ['task', 'draft', 'settings', 'terminal']) {
      const page = await browser.newPage({ viewport: { width: 1500, height: 960 } });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript({ path: 'scripts/ui-fixture.js' });
      await page.addInitScript(tasks => { const q = window.__MONITTER_QA__, s = q.snapshot(); s.tasks.push(...tasks); q.setSnapshot(s); }, [task('base', 'Base chat'), task('target', 'Target chat')]);
      try {
        await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
        await expect(page.locator('.sidebar .task-select[title="Base chat"]')).toBeVisible({ timeout: 240_000 });
        await page.locator('.sidebar .task-select[title="Base chat"]').click();
        await page.keyboard.press('Meta+p');
        await page.getByRole('dialog', { name: 'Controls', exact: true }).getByText('Two columns', { exact: true }).click();
        await expect(page.locator('.pane-leaf')).toHaveCount(2);
        const second = page.locator('.pane-leaf').last();
        await second.locator('.topbar').click();
        if (kind === 'task') await page.locator('.sidebar .task-select[title="Target chat"]').click();
        if (kind === 'draft') await second.getByRole('button', { name: 'New chat', exact: true }).click();
        if (kind === 'settings') await page.getByRole('button', { name: 'Preferences', exact: true }).click();
        if (kind === 'terminal') await second.getByRole('button', { name: 'Terminal', exact: true }).click();
        const pane = page.locator('.pane-leaf').filter({ has: kind === 'terminal' ? page.locator('.terminal-tab') : kind === 'settings' ? page.locator('[data-tab-kind="settings"]') : kind === 'draft' ? page.locator('[data-tab-kind="draft"]') : page.locator('[data-tab-id="target"]') });
        await expect(pane).toHaveCount(1);
        const close = pane.locator(`.tab-entry[data-tab-kind="${kind}"] .close-tab`);
        await expect(close).toHaveCount(1);
        await pane.locator(`.tab-entry[data-tab-kind="${kind}"]`).hover();
        await close.click();
        await expect(page.locator('.pane-leaf')).toHaveCount(1);
        await expect(page.locator('[data-tab-id="base"]')).toHaveCount(1);
        const root = await page.locator('.pane-grid-root').boundingBox(), survivor = await page.locator('.pane-leaf').boundingBox();
        expect(Math.abs(root.width - survivor.width)).toBeLessThan(2);
        expect(Math.abs(root.height - survivor.height)).toBeLessThan(2);
        expect(errors).toEqual([]);
      } finally { await page.close(); }
    }

    // The historical right-split then nested-bottom move must remove each empty source,
    // retain both tabs once, and leave the remaining panes occupying the grid.
    const page = await browser.newPage({ viewport: { width: 1500, height: 960 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript({ path: 'scripts/ui-fixture.js' });
    await page.addInitScript(tasks => { const q = window.__MONITTER_QA__, s = q.snapshot(); s.tasks.push(...tasks); q.setSnapshot(s); }, [task('nested-a', 'Nested A'), task('nested-b', 'Nested B')]);
    try {
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
      await expect(page.locator('.sidebar .task-select[title="Nested A"]')).toBeVisible({ timeout: 240_000 });
      for (const title of ['Nested A', 'Nested B']) await page.locator(`.sidebar .task-select[title="${title}"]`).click();
      const drag = async (source, target, x, y) => {
        const from = await source.boundingBox(), to = await target.boundingBox();
        await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2); await page.mouse.down();
        await page.mouse.move(to.x + to.width * x, to.y + to.height * y, { steps: 14 }); await page.mouse.up();
      };
      const panes = page.locator('.pane-leaf');
      await drag(page.locator('[data-tab-id="nested-b"] .tab'), panes.first(), .98, .5);
      await expect(panes).toHaveCount(2);
      await drag(page.locator('[data-tab-id="nested-a"] .tab'), panes.last(), .5, .98);
      await expect(panes).toHaveCount(2);
      for (const id of ['nested-a', 'nested-b']) await expect(page.locator(`[data-tab-id="${id}"]`)).toHaveCount(1);
      await expect(page.locator('.pane-split.column')).toHaveCount(1);
      await expect.poll(() => panes.evaluateAll(nodes => nodes.every(node => node.getAnimations().length === 0))).toBe(true);
      const root = await page.locator('.pane-grid-root').boundingBox();
      for (const box of await panes.evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect()))) {
        expect(box.width).toBeGreaterThan(20); expect(box.height).toBeGreaterThan(20);
        expect(box.left).toBeGreaterThanOrEqual(root.x - 1); expect(box.right).toBeLessThanOrEqual(root.x + root.width + 1);
      }
      await drag(page.locator('[data-tab-id="nested-a"] .tab'), panes.first(), .5, .5);
      await expect(panes).toHaveCount(1);
      await expect.poll(() => panes.evaluateAll(nodes => nodes.every(node => node.getAnimations().length === 0))).toBe(true);
      for (const id of ['nested-a', 'nested-b']) await expect(page.locator(`[data-tab-id="${id}"]`)).toHaveCount(1);
      const filled = await panes.boundingBox();
      expect(Math.abs(root.width - filled.width)).toBeLessThan(2);
      expect(Math.abs(root.height - filled.height)).toBeLessThan(2);
      expect(errors).toEqual([]);
    } finally { await page.close(); }
    console.log(`${engine.name()}: final task/draft/settings/terminal close collapse and nested right/bottom move passed`);
  } finally { await browser.close(); }
}
