import { chromium, webkit, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
mkdirSync('/tmp/monitter-motion-qa', { recursive: true });
for (const engine of (process.env.MOTION_ENGINE === 'chromium' ? [chromium] : [chromium, webkit])) {
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
  page.setDefaultTimeout(20000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const original = Element.prototype.animate;
    window.__motionLog = [];
    Element.prototype.animate = function (frames, options) {
      window.__motionLog.push({ classes: this.className, ghost: this.hasAttribute('data-motion-ghost'), frames, options });
      return original.call(this, frames, options);
    };
    const qa = window.__MONITTER_QA__, s = qa.snapshot();
    s.settings.tabStyle = 'modern'; s.settings.interfaceScale = 100;
    for (const id of ['motion-a', 'motion-b', 'motion-c']) {
      s.tasks.push({ id, agentId: 'atlas', title: id, status: 'idle', archived: false, createdAt: 1, updatedAt: 1, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only', projectId: null, channelId: null, parentTaskId: null, nativeSessionId: null });
      s.messages.push({ id: `${id}-message`, taskId: id, role: 'assistant', text: `Conversation ${id}`, createdAt: 2 });
    }
    qa.setSnapshot(s);
  });
  const clearLog = () => page.evaluate(() => { window.__motionLog = []; });
  try {
    await page.goto(url, { timeout: 240000 });
    await page.locator('.sidebar').waitFor({ timeout: 240000 });
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'subtle');
    await clearLog();
    for (const mode of ['Activity', 'Projects', 'Standard']) await page.getByRole('button', { name: `${mode} view`, exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__motionLog.filter(x => String(x.classes).includes('sidebar-mode-content') && !x.ghost).length)).toBeGreaterThanOrEqual(3);
    const swaps = await page.evaluate(() => window.__motionLog.filter(x => String(x.classes).includes('sidebar-mode-content') && !x.ghost));
    expect(swaps[0].frames[0].transform).toContain('16px');
    expect(swaps.at(-1).frames[0].transform).toContain('-16px');
    await expect(page.locator('[data-motion-ghost]')).toHaveCount(0);

    // Selected tabs stay visually steady, focus and drafts survive navigation.
    const openTask = id => page.locator('.sidebar .task-select').filter({ hasText: id }).click();
    await openTask('motion-a');
    const composer = page.getByRole('textbox', { name: 'Task message', exact: true });
    await composer.fill('Keep this unsent draft');
    await openTask('motion-b');
    await page.locator('.tab-entry[data-tab-id="motion-a"] > .tab').click();
    await expect(composer).toHaveValue('Keep this unsent draft');
    await expect(composer).toBeFocused();
    const activeTab = page.locator('.tab-entry.active');
    await page.mouse.move(1300, 850);
    const before = await activeTab.evaluate(node => getComputedStyle(node).backgroundColor);
    await activeTab.hover();
    expect(await activeTab.evaluate(node => getComputedStyle(node).backgroundColor)).toBe(before);

    await clearLog();
    await page.getByRole('button', { name: 'Hide right sidebar', exact: true }).click();
    await page.getByRole('button', { name: 'Show right sidebar', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__motionLog.some(x => String(x.classes).includes('run-detail') && !x.ghost))).toBeTruthy();
    await page.getByRole('tab', { name: 'Approvals', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Approval history', exact: true })).toBeVisible();

    // Dialogs must remain keyboard usable while opening/closing and restore focus.
    await composer.focus();
    await page.keyboard.press(`${modifier}+p`);
    const palette = page.getByRole('dialog', { name: 'Controls', exact: true });
    await expect(palette).toBeVisible();
    await expect(palette.locator('input')).toBeFocused();
    await palette.locator('input').fill('Two columns');
    await page.keyboard.press('Escape');
    await expect(palette).toHaveCount(0);
    await expect(composer).toBeFocused();
    await page.keyboard.press(`${modifier}+p`);
    await page.getByRole('dialog', { name: 'Controls', exact: true }).getByText('Two columns', { exact: true }).click();
    await expect(page.locator('.pane-leaf')).toHaveCount(2);
    const dimmed = page.locator('.pane-leaf.dimmed');
    await expect(dimmed).toHaveCount(1);
    await expect.poll(() => dimmed.evaluate(node => getComputedStyle(node).opacity)).toBe('1');

    // Local motion settings, all themes/densities, OS override, reload persistence.
    await page.getByRole('button', { name: 'Preferences', exact: true }).click();
    await page.locator('.settings-nav button').filter({ hasText: 'Appearance' }).click();
    const motionCard = page.locator('.setting-card').filter({ has: page.getByRole('heading', { name: 'Motion', exact: true }) });
    await motionCard.getByRole('button', { name: 'Off', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
    await clearLog();
    await page.getByRole('button', { name: 'Projects view', exact: true }).click();
    expect(await page.evaluate(() => window.__motionLog.length)).toBe(0);
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
    await page.getByRole('button', { name: 'Preferences', exact: true }).click();
    await page.locator('.settings-nav button').filter({ hasText: 'Appearance' }).click();
    await motionCard.getByRole('button', { name: 'Subtle', exact: true }).click();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'subtle');
    for (const theme of ['light', 'dark']) for (const density of ['tight', 'normal', 'spacious']) {
      await page.evaluate(({ theme, density }) => { const q = window.__MONITTER_QA__, s = q.snapshot(); s.settings.theme = theme; s.settings.interfaceDensity = density; q.setSnapshot(s); }, { theme, density });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('html')).toHaveAttribute('data-density', density);
      await page.screenshot({ path: `/tmp/monitter-motion-qa/${engine.name()}-${theme}-${density}.png` });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('.app-shell').first()).toHaveClass(/mobile-navigation/);
    expect(errors).toEqual([]);
    console.log(`${engine.name()}: navigation, focus, draft, hover, detail, modal, dimming, preferences and theme/density matrix passed`);
  } catch (error) {
    console.error('PAGE ERRORS', errors);
    console.error('SETTINGS', await page.locator('.settings-content').allTextContents());
    await page.screenshot({ path: `/tmp/monitter-motion-qa/${engine.name()}-failure.png` });
    throw error;
  } finally { await browser.close(); }
}
