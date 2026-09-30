import { chromium, expect } from '@playwright/test';

// Uses the browser-only bridge fixture. No native runtime or agent is started.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'New chat', exact: true }).first()).toBeVisible({ timeout: 30000 });

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const bridge = window.__MONITTER_BRIDGE__;
    qa.holdSettingsSave = (fail = false) => {
      let release;
      const pending = new Promise((resolve, reject) => {
        release = () => fail ? reject(new Error('deliberate settings failure')) : resolve();
      });
      const original = bridge.saveSettings;
      qa.settingsSaveStarted = false;
      qa.releaseSettingsSave = release;
      bridge.saveSettings = async settings => {
        bridge.saveSettings = original;
        qa.settingsSaveStarted = true;
        await pending;
        return original(settings);
      };
    };
  });

  const openControls = async () => {
    await page.keyboard.press('Meta+p');
    const palette = page.getByRole('dialog', { name: 'Controls', exact: true });
    await expect(palette).toBeVisible();
    return palette;
  };
  await page.evaluate(() => window.__MONITTER_QA__.holdSettingsSave());
  const palette = await openControls();
  await palette.getByRole('button', { name: /Show tool activity/ }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.settingsSaveStarted)).toBe(true);
  await expect(palette).toHaveCount(0, { timeout: 1500 });

  // The held write no longer owns the modal layer or global busy flag.
  const newChat = page.getByRole('button', { name: 'New chat', exact: true }).first();
  await expect(newChat).toBeEnabled();
  await newChat.click();
  const composer = page.getByRole('textbox', { name: 'Task message', exact: true });
  await expect(composer).toBeEditable();
  await composer.fill('Still responsive during a pending settings save.');
  await page.evaluate(() => window.__MONITTER_QA__.releaseSettingsSave());
  await expect(page.getByRole('status').filter({ hasText: 'Setting saved.' })).toBeVisible();

  await page.evaluate(() => window.__MONITTER_QA__.holdSettingsSave(true));
  const retryPalette = await openControls();
  await retryPalette.getByRole('button', { name: /Show tool activity/ }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.settingsSaveStarted)).toBe(true);
  await expect(retryPalette).toHaveCount(0, { timeout: 1500 });
  await page.evaluate(() => window.__MONITTER_QA__.releaseSettingsSave());
  await expect(page.getByRole('alert').filter({ hasText: 'Could not save settings: deliberate settings failure' })).toBeVisible();
  console.log('Chromium: delayed palette settings saves close promptly, leave the composer usable, and report failures.');
} finally {
  await browser.close();
}
