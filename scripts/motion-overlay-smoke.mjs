import { chromium, webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464/';
const fixture = readFileSync('scripts/ui-fixture.js', 'utf8');

async function waitForPreview(browserType) {
  const probe = await browserType.launch({ headless: true });
  const page = await probe.newPage();
  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120_000 });
  } finally {
    await probe.close();
  }
}

async function run(browserType, name, motionOff = false) {
  const browser = await browserType.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript({ content: fixture });
  if (motionOff) {
    await page.addInitScript(() => localStorage.setItem('monitter.appearance.motion.v1', 'off'));
  }
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120_000 });
  await expect(page.locator('.sidebar')).toBeVisible({ timeout: 60_000 });

  const returnTarget = page.getByRole('button', { name: 'Preferences', exact: true });
  await returnTarget.focus();
  await page.keyboard.press('Meta+k');
  const palette = page.getByRole('dialog', { name: 'Switch to' });
  await expect(palette).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(palette).toHaveCount(0, { timeout: 1_000 });
  await expect(returnTarget).toBeFocused();

  await page.keyboard.press('Meta+k');
  await expect(palette.locator('input')).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(palette.getByRole('button', { name: 'Close command palette' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(palette.locator('input')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(palette).toHaveCount(0, { timeout: 1_000 });

  if (!motionOff) {
    // Escape is immediate, while a fresh open only starts after the inert closing
    // shell has completed its brief exit.
    await returnTarget.focus();
    await page.keyboard.press('Meta+p');
    const controls = page.getByRole('dialog', { name: 'Controls' });
    await expect(controls).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(controls).toHaveCount(0, { timeout: 1_000 });
    await returnTarget.focus();
    await page.waitForTimeout(150);
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+p' : 'Control+p');
    await expect(controls).toBeVisible({ timeout: 1_000 });
    await page.keyboard.press('Escape');
    await expect(controls).toHaveCount(0, { timeout: 1_000 });
  }

  const hosts = page.locator('button[aria-label="Hosts"]:visible').first();
  await hosts.focus();
  const hostsHandle = await hosts.elementHandle();
  await hosts.press('Enter');
  const modal = page.getByRole('dialog', { name: 'Hosts' });
  await expect(modal).toBeVisible();
  const modalFocusable = modal.locator('button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [href], [tabindex]:not([tabindex="-1"])');
  const modalFocusCount = await modalFocusable.count();
  if (modalFocusCount > 1) {
    await modalFocusable.first().focus();
    await page.keyboard.press('Shift+Tab');
    await expect(modalFocusable.nth(modalFocusCount - 1)).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(modalFocusable.first()).toBeFocused();
  }
  await page.keyboard.press('Escape');
  await expect(modal).toHaveCount(0, { timeout: 1_000 });
  await expect.poll(() => page.evaluate((element) => document.activeElement === element, hostsHandle)).toBe(true);

  if (errors.length) throw new Error(`${name}: ${errors.join('\n')}`);
  await browser.close();
  console.log(`${name}${motionOff ? ' motion off' : ''}: overlay lifecycle ok`);
}

if (process.env.MOTION_OVERLAY_SKIP_PROBE !== '1') await waitForPreview(chromium);
const targets = [[chromium, 'chromium'], [webkit, 'webkit']];
for (const [browserType, name] of targets.filter(([, name]) => !process.env.MOTION_OVERLAY_BROWSER || name === process.env.MOTION_OVERLAY_BROWSER)) {
  await run(browserType, name);
  if (process.env.MOTION_OVERLAY_MODE !== 'default') await run(browserType, name, true);
}
