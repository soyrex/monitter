import { chromium, webkit, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const engines = process.env.MOTION_ENGINE === 'chromium' ? [chromium] : process.env.MOTION_ENGINE === 'webkit' ? [webkit] : [chromium, webkit];
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';

for (const engine of engines) {
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  console.log(`${engine.name()}: starting`);
  page.setDefaultTimeout(30_000);
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const original = Element.prototype.animate;
    window.__paneMotion = [];
    Element.prototype.animate = function (frames, options) {
      window.__paneMotion.push({ pane: this.classList?.contains('pane-leaf'), frames, options });
      return original.call(this, frames, options);
    };
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.settings.shortcutMode = 'vim';
    snapshot.settings.interfaceScale = 100;
    snapshot.settings.dimInactivePanes = true;
    snapshot.settings.inactivePaneOpacity = .6;
    for (let i = 0; i < 18; i++) {
      const id = `motion-${i}`;
      snapshot.tasks.push({ id, agentId: 'atlas', title: id, nativeSessionId: null, status: 'idle', archived: false, createdAt: now + i, updatedAt: now + i, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' });
      snapshot.messages.push({ id: `${id}-message`, taskId: id, role: 'assistant', text: `Fixture conversation ${i}`, createdAt: now + i });
    }
    qa.setSnapshot(snapshot);
  });
  const command = async value => {
    await page.locator('.pane-leaf[data-pane-id="main"] .topbar').click();
    await page.keyboard.press('Escape');
    await page.keyboard.type(':');
    const input = page.getByRole('textbox', { name: 'Monitter command', exact: true });
    await input.fill(value); await input.press('Enter');
  };
  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
    await expect(page.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible({ timeout: 240_000 });
    console.log(`${engine.name()}: fixture ready`);
    const main = page.locator('.pane-leaf[data-pane-id="main"]');

    // Give a real terminal a lifecycle before exercising split, balance and close.
    console.log(`${engine.name()}: motion-0 selector count ${await page.locator('.sidebar .task-select[title="motion-0"]').count()}`);
    await page.locator('.sidebar .task-select[title="motion-0"]').click();
    console.log(`${engine.name()}: motion-0 selected`);
    await page.getByRole('button', { name: 'New terminal', exact: true }).click();
    await expect(main.locator('.terminal-pane .xterm')).toBeVisible();
    const terminalId = await page.evaluate(() => window.__MONITTER_QA__.terminals()[0].id);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'openTerminal').length)).toBe(1);
    console.log(`${engine.name()}: terminal open`);

    await page.evaluate(() => { window.__paneMotion = []; });
    await command('vsplit'); await expect(page.locator('.pane-leaf')).toHaveCount(2);
    await command('split'); await expect(page.locator('.pane-leaf')).toHaveCount(3);
    await command('wincmd =');
    await expect.poll(() => page.locator('.pane-split').count()).toBeGreaterThan(0);
    await expect.poll(() => page.evaluate(id => window.__MONITTER_QA__.terminals().some(item => item.id === id), terminalId)).toBe(true);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'openTerminal').length)).toBe(1);
    console.log(`${engine.name()}: split and balance`);

    // Pointer resizing must update layout immediately, without triggering a pane FLIP.
    await page.evaluate(() => { window.__paneMotion = []; });
    const splitter = page.getByRole('separator', { name: 'Resize panes', exact: true }).first();
    const box = await splitter.boundingBox();
    await page.mouse.move(box.x + .5, box.y + box.height / 2); await page.mouse.down();
    await page.mouse.move(box.x + 80, box.y + box.height / 2, { steps: 6 }); await page.mouse.up();
    await page.waitForTimeout(240);
    expect(await page.evaluate(() => window.__paneMotion.filter(item => item.pane).length)).toBe(0);
    console.log(`${engine.name()}: pointer resize`);

    // Closing other panes keeps the existing terminal process and its host mounted once.
    await main.locator('.topbar').click(); await command('only');
    await expect(page.locator('.pane-leaf')).toHaveCount(1);
    await expect(main.locator('.terminal-pane .xterm')).toBeVisible();
    expect(await page.evaluate(id => window.__MONITTER_QA__.terminals().some(item => item.id === id), terminalId)).toBe(true);
    console.log(`${engine.name()}: terminal close lifecycle`);

    // Lower configured opacity means stronger overlay, while both theme paths remain inert.
    await command('vsplit'); await expect(page.locator('.pane-leaf')).toHaveCount(2);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => { const q = window.__MONITTER_QA__, s = q.snapshot(); s.settings.theme = theme; s.settings.inactivePaneOpacity = .2; q.setSnapshot(s); }, theme);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const inactive = page.locator('.pane-leaf.dimmed');
      await expect(inactive).toHaveCount(1);
      expect(await inactive.locator('.pane-dim-overlay').evaluate(node => getComputedStyle(node).pointerEvents)).toBe('none');
      await expect.poll(() => inactive.evaluate(node => getComputedStyle(node).getPropertyValue('--pane-dim-strength').trim())).toBe('0.8');
      expect(await inactive.evaluate(node => getComputedStyle(node).opacity)).toBe('1');
    }
    for (const scheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.evaluate(() => { const q = window.__MONITTER_QA__, s = q.snapshot(); s.settings.theme = 'system'; q.setSnapshot(s); });
      await expect.poll(() => page.locator('html').getAttribute('data-theme')).toBe('system');
    }
    console.log(`${engine.name()}: dim themes`);

    // Deliberate selection scrolls a crowded strip and the final active tab is fully visible.
    await main.locator('.topbar').click();
    for (let i = 1; i < 18; i++) await page.locator(`.sidebar .task-select[title="motion-${i}"]`).click();
    const strip = main.locator('.tabs.tab-picker');
    await page.evaluate(() => { const last = document.querySelector('[data-tab-id="motion-0"]'); last.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); last.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await page.waitForTimeout(220);
    const visible = await page.evaluate(() => {
      const strip = document.querySelector('.pane-leaf[data-pane-id="main"] .tabs.tab-picker'), active = strip.querySelector('.tab-entry.active');
      const a = active.getBoundingClientRect(), b = strip.getBoundingClientRect();
      return a.left >= b.left - 1 && a.right <= b.right + 1 && strip.scrollLeft > 0;
    });
    expect(visible).toBe(true);
    console.log(`${engine.name()}: tab reveal`);

    expect(errors).toEqual([]);
    console.log(`${engine.name()}: split/close/balance terminal lifecycle, inert dim overlays, pointer resize, system themes, and tab reveal passed`);
  } finally { await browser.close(); }
}
