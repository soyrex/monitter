import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await chromium.launch({ headless: true, executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

try {
  await page.addInitScript({ content: readFileSync('scripts/ui-fixture.js', 'utf8') + `
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.tasks.push({ id:'font-chat', agentId:'atlas', title:'Font preview', nativeSessionId:null, status:'idle', archived:false, createdAt:now, updatedAt:now, parentTaskId:null, channelId:null, projectId:null, hostId:'local', cwd:'/tmp', provider:'codex', model:'', sandbox:'read-only' });
    snapshot.messages.push({ id:'font-message', taskId:'font-chat', role:'user', text:'A font weight preview message.', createdAt:now, attachments:[] });
    qa.setSnapshot(snapshot);
  ` });
  await page.goto('http://127.0.0.1:18421/monitter-app-ui/', { timeout: 60000 });
  await page.locator('.sidebar .task-select').filter({ hasText: 'Font preview' }).click();
  await expect(page.locator('.message-content').first()).toBeVisible();
  await page.keyboard.press('Meta+,');
  await page.getByRole('navigation', { name: 'Settings categories' }).getByRole('button', { name: /Typography/ }).click();

  for (const [label, key, value] of [
    ['Interface font weight', 'interfaceFontWeight', 500],
    ['Chat font weight', 'chatFontWeight', 600],
    ['Terminal font weight', 'terminalFontWeight', 300],
  ]) {
    const slider = page.getByRole('slider', { name: label });
    await expect(slider).toHaveValue('400');
    await slider.evaluate((input, next) => {
      input.value = String(next);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }, value);
    const cssName = '--' + key.replace(/[A-Z]/g, letter => '-' + letter.toLowerCase());
    await expect.poll(() => page.evaluate(name => document.documentElement.style.getPropertyValue(name), cssName)).toBe(String(value));
    await slider.evaluate(input => input.dispatchEvent(new Event('change', { bubbles: true })));
    await expect.poll(() => page.evaluate(name => window.__MONITTER_QA__.snapshot().settings[name], key)).toBe(value);
  }

  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement).fontWeight)).toBe('500');
  await expect.poll(() => page.locator('.sidebar .task-select').first().evaluate(node => getComputedStyle(node).fontWeight)).toBe('500');
  await page.locator('.tabs').getByRole('button', { name: 'Font preview', exact: true }).click();
  await expect.poll(() => page.locator('.message-content').first().evaluate(node => getComputedStyle(node).fontWeight)).toBe('600');

  await page.locator('.settings-tab .tab').click();
  await page.evaluate(() => {
    const bridge = window.__MONITTER_BRIDGE__, original = bridge.saveSettings;
    bridge.saveSettings = async () => { bridge.saveSettings = original; throw Error('Weight save failed'); };
  });
  const slider = page.getByRole('slider', { name: 'Chat font weight' });
  await slider.evaluate(input => {
    input.value = '700';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await expect(page.locator('.settings-pane').getByRole('alert')).toContainText('Weight save failed');
  await expect.poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue('--chat-font-weight'))).toBe('600');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.chatFontWeight)).toBe(600);
  await expect(slider).toHaveValue('600');
  expect(errors).toEqual([]);
  console.log('Font weight sliders preview, save, render, and roll back after a failed save.');
} finally {
  await browser.close();
}
