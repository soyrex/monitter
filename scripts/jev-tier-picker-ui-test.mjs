// Browser fixture only: selecting Jev tier models never invokes a live agent or Jev.
import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1200, height: 820 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
  const qa = window.__MONITTER_QA__, state = qa.snapshot();
  state.agents[0].jevRouting = 'safe_auto';
  state.agents[0].jevModelTiers = { fast: '', balanced: '', strong: '', frontier: 'GPT-6-Astra' };
  qa.setSnapshot(state);
`);
await page.addInitScript(() => {
  window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => 1 };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
});

try {
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120000 });
  await expect(page.getByRole('button', { name: 'New chat', exact: true }).first()).toBeVisible({ timeout: 30000 });
  await page.keyboard.press('Control+,');
  const settings = page.getByRole('region', { name: 'Settings', exact: true });
  await settings.getByRole('button', { name: 'Agents', exact: true }).click();
  await settings.getByRole('button', { name: 'Edit Atlas', exact: true }).click();
  const routing = settings.locator('details.agent-profile').filter({ has: page.locator('summary', { hasText: 'Jev routing' }) });
  await routing.locator('summary').click();
  await expect(routing.getByLabel('Fast model')).toBeEnabled();
  await expect(routing.getByLabel('Frontier model')).toHaveValue('GPT-6-Astra');
  await expect(routing.getByText('Choose an advertised model to replace this unverified ID.')).toBeVisible();
  await routing.getByLabel('Fast model').selectOption('qa-balanced');
  await routing.getByLabel('Balanced model').selectOption('qa-simple');
  await routing.getByLabel('Strong model').selectOption('');
  await routing.getByLabel('Frontier model').selectOption('qa-balanced');
  await settings.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].jevModelTiers)).toEqual({
    fast: 'qa-balanced', balanced: 'qa-simple', strong: '', frontier: 'qa-balanced',
  });
  await settings.getByRole('button', { name: 'Back to agents', exact: true }).click();
  await settings.getByRole('button', { name: 'Edit Atlas', exact: true }).click();
  await routing.locator('summary').click();
  await expect(routing.getByLabel('Fast model')).toHaveValue('qa-balanced');
  await expect(routing.getByLabel('Frontier model')).toHaveValue('qa-balanced');
  expect(errors).toEqual([]);
  console.log('Jev tier picker saved catalog model IDs, kept an empty tier, flagged an unverified ID, and restored selections.');
} finally {
  await browser.close();
}
