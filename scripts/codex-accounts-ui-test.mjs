// Browser-only fixtures; start an isolated preview and set MONITTER_TEST_URL.
import { chromium, expect as baseExpect } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const expect = baseExpect.configure({ timeout: 30_000 });
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18431';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 980 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    // Native event registration is inert in this browser-only harness.
    window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => 1 };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  });
  await page.goto(url, { timeout: 120_000 });
  await expect(page.getByRole('button', { name: 'Preferences', exact: true }).first()).toBeVisible();

  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  let settings = page.getByRole('region', { name: 'Settings', exact: true });
  await settings.getByRole('button', { name: 'Agents', exact: true }).click();
  const account = settings.getByLabel('Codex account', { exact: true });
  await expect(account).toBeVisible();
  await account.selectOption('/Users/alex/.codex-work');
  await settings.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].codexHome)).toBe('/Users/alex/.codex-work');

  // Switching editors and returning loads the saved selection.
  await settings.getByLabel('Select agent', { exact: true }).selectOption('');
  await settings.getByLabel('Select agent', { exact: true }).selectOption('atlas');
  await expect(settings.getByLabel('Codex account', { exact: true })).toHaveValue('/Users/alex/.codex-work');

  // A newly created task pins the account; changing the agent later does not rewrite it.
  await page.evaluate(async () => {
    await window.__MONITTER_BRIDGE__.createTask({ agentId: 'atlas', title: 'Pinned account chat', nativeSessionId: null, parentTaskId: null, channelId: null, projectId: null, cwd: null });
  });
  const pinned = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.at(-1));
  expect(pinned.codexHome).toBe('/Users/alex/.codex-work');
  await settings.getByLabel('Codex account', { exact: true }).selectOption('/Users/alex/.codex');
  await settings.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].codexHome)).toBe('/Users/alex/.codex');
  expect(await page.evaluate(id => window.__MONITTER_QA__.snapshot().tasks.find(task => task.id === id).codexHome, pinned.id)).toBe('/Users/alex/.codex-work');

  // Switching away from Codex clears the private account selection before save.
  await settings.getByLabel('Harness', { exact: true }).selectOption('claude');
  await settings.getByRole('button', { name: 'Save agent', exact: true }).click();
  const afterClaude = await page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0]);
  expect(afterClaude.provider).toBe('claude');
  expect(afterClaude.codexHome ?? null).toBe(null);
  await settings.getByLabel('Harness', { exact: true }).selectOption('codex');
  await settings.getByLabel('Codex account', { exact: true }).selectOption('/Users/alex/.codex-work');
  await settings.getByRole('button', { name: 'Save agent', exact: true }).click();

  const usage = page.locator('.usage-provider').filter({ hasText: 'Codex' });
  const accountSelector = usage.getByLabel('Codex account', { exact: true });
  if (!(await accountSelector.isVisible())) await page.getByRole('button', { name: /MODEL USAGE/ }).click();
  await expect(accountSelector).toBeVisible();
  await accountSelector.selectOption('/Users/alex/.codex');
  await expect(usage.locator('.ring-value')).toHaveText('12');
  await accountSelector.selectOption('/Users/alex/.codex-work');
  await expect(usage.locator('.ring-value')).toHaveText('67');
  await expect(usage).toHaveAttribute('title', /67%/);
  await page.setViewportSize({ width: 980, height: 980 });
  await expect(accountSelector).toBeVisible();
  await expect(usage.locator('.ring-value')).toHaveText('67');
  await page.screenshot({ path: 'verification/codex-accounts.png', fullPage: true });
  expect(errors).toEqual([]);
  console.log('Codex account UI passed: discovery/persistence, task pinning, provider clearing, distinct 12%/67% usage, and no page errors.');
} catch (error) {
  console.error('Page errors:', errors);
  console.error((await page.locator('body').innerText()).slice(0, 4000));
  await page.screenshot({ path: 'verification/codex-accounts-error.png', fullPage: true });
  throw error;
} finally {
  await browser.close();
}
