import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Exercise the real fetch bridge, with HTTP API responses isolated from real agents.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18435';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage();
const errors = [], calls = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `\n(() => {
  window.__LAN_FIXTURE__ = window.__MONITTER_QA__.snapshot();
  delete window.__MONITTER_TEST_BRIDGE__;
  delete window.__MONITTER_BRIDGE__;
  Object.defineProperty(crypto, 'randomUUID', { value: undefined });
})();`);
let rejected = false;
const unrestricted = process.env.MONITTER_TEST_UNRESTRICTED === '1';
await page.route('**/api/access', route => route.fulfill({ json: { required: !unrestricted } }));
await page.route('**/api/invoke', async route => {
  const { command } = route.request().postDataJSON();
  calls.push(command);
  if (!unrestricted && (route.request().headers().authorization !== 'Bearer 012345' || rejected)) {
    await route.fulfill({ status: 401, json: { ok: false, error: 'Unauthorized' } }); return;
  }
  const result = command === 'get_snapshot' ? await page.evaluate(() => window.__LAN_FIXTURE__)
    : command === 'get_model_catalog' ? { models: [], current: { model: '', reasoningEffort: null, fastMode: null }, source: 'test', warning: null } : [];
  await route.fulfill({ json: { ok: true, result } });
});
await page.route(url + '/', async route => {
  const response = await route.fetch();
  await route.fulfill({ response, body: (await response.text()).replace('<html ', '<html data-monitter-lan="1" ').replace('<head>', '<head><meta name="monitter-lan" content="1">') });
});
try {
  await page.goto(url + '/');
  if (!unrestricted) {
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible({ timeout: 60000 });
  expect(calls).toEqual([]);
  await expect(page.getByLabel('Access code', { exact: true })).toHaveAttribute('inputmode', 'numeric');
  await page.getByLabel('Access code', { exact: true }).fill('123');
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeDisabled();
  await page.getByLabel('Access code', { exact: true }).fill('999999');
  await expect(page.getByLabel('Access code', { exact: true })).toHaveAttribute('pattern', '[0-9]{6}');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Incorrect or expired access code');
  await page.getByLabel('Access code', { exact: true }).fill('012345');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.locator('[data-workspace-context]')).toBeVisible({ timeout: 30000 });
  } else {
    await expect(page.locator('[data-workspace-context]')).toBeVisible({ timeout: 30000 });
    await expect(page.getByLabel('Access code', { exact: true })).toHaveCount(0);
  }
  await expect.poll(() => calls.filter(command => command === 'get_snapshot').length).toBeGreaterThan(2);
  await page.getByRole('button', { name: 'Start a task', exact: true }).click();
  await expect(page.locator('[data-tab-kind="draft"]')).toBeVisible();
  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  await page.getByRole('button', { name: 'LAN access', exact: true }).click();
  await expect(page.getByText('You are connected to Monitter at', { exact: false })).toBeVisible();
  if (!unrestricted) {
    rejected = true;
    await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible({ timeout: 10000 });
  }
  expect(errors).toEqual([]);
  console.log('LAN browser login, authentication, full interface, polling and expiry checks passed.');
} catch (error) { console.error('Browser errors:', errors, '\nPage:', (await page.locator('body').innerText()).slice(0, 2000)); throw error; }
finally { await browser.close(); }
