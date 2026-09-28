import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));

try {
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18439/');
  const shell = page.locator('.app-shell').first();
  await expect(shell).toHaveClass(/mobile-navigation/, { timeout: 60000 });
  await expect(shell).not.toHaveClass(/mobile-main/);

  // The controls palette is available while the sidebar is the visible mobile screen.
  await page.keyboard.press('Meta+P');
  await page.getByRole('button', { name: 'New terminal', exact: true }).click();
  await expect(shell).toHaveClass(/mobile-main/);
  await expect(page.locator('.terminal-pane')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to chats' })).toBeVisible();
  expect(errors).toEqual([]);
  console.log('WebKit mobile controls palette opens a terminal from the sidebar.');
} finally {
  await browser.close();
}
