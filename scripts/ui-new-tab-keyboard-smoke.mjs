import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true, ...(process.env.MONITTER_CHROME_PATH ? { executablePath: process.env.MONITTER_CHROME_PATH } : {}) });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => Object.defineProperty(navigator, 'platform', { value: 'MacIntel' }));
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433');
  await page.getByRole('button', { name: 'Preferences', exact: true }).waitFor();

  await page.keyboard.press('Meta+t');
  const choices = page.getByRole('region', { name: 'Choose pane content' });
  const chat = choices.getByRole('button', { name: 'New chat' });
  const terminal = choices.getByRole('button', { name: 'Terminal' });
  await expect(chat).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(terminal).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(chat).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(terminal).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(chat).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'New chat draft' })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Task message' })).toBeFocused();
  console.log('Cmd-T focuses New chat; arrows switch choices; Enter opens a chat draft.');
} finally {
  await browser.close();
}
