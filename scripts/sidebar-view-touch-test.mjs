import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

async function tap(button) {
  const box = await button.boundingBox();
  if (!box) throw new Error('Sidebar view control has no tappable bounds.');
  await page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2);
}

await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
try {
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18439/');
  const standard = page.getByRole('tab', { name: 'Agents view', exact: true });
  const activity = page.getByRole('tab', { name: 'Activity view', exact: true });
  const projects = page.getByRole('tab', { name: 'Projects view', exact: true });
  await expect(standard).toHaveAttribute('aria-selected', 'true', { timeout: 120000 });

  await tap(projects);
  await expect(projects).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.sidebar-mode-content > .section-label')).toHaveCount(0);

  await tap(activity);
  await expect(activity).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.sidebar-mode-content > .section-label')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('monitter.sidebar-view.v2:web'))).toBe('activity');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'saveSettings' && call.args.sidebarView))).toEqual([]);
  expect(errors).toEqual([]);
  console.log('WebKit touchscreen single-tap changes the sidebar view.');
} finally {
  await browser.close();
}
