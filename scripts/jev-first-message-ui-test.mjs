import { chromium, expect } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const systemChrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch(existsSync(systemChrome) ? { headless: true, executablePath: systemChrome } : { headless: true });
const page = await browser.newPage({ viewport: { width: 1200, height: 820 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
  document.documentElement?.setAttribute('data-monitter-lan', '1');
  document.addEventListener('DOMContentLoaded', () => document.documentElement.setAttribute('data-monitter-lan', '1'));
  const qa = window.__MONITTER_QA__, state = qa.snapshot();
  state.agents.find(agent => agent.id === 'atlas').jevRouting = 'recommend';
  qa.setSnapshot(state);
  window.__MONITTER_BRIDGE__.planJevRoute = async () => ({
    traceId: 'review-required-first-message',
    promptFingerprint: 'sha256:fixture',
    decision: {
      task_kind: 'architecture', model_tier: 'frontier', reasoning_level: 'xhigh',
      execution_mode: 'inspect', permission_tier: 'human_review_required', confidence: 0.25,
      rationale: 'Fixture review boundary.', escalation_conditions: ['Human review before effect.']
    },
    classifierEvidence: { provider: 'Fixture', model: 'jev-test', latencyMs: 1, inputTokens: 1, outputTokens: 1, costUsd: null }
  });
`);

try {
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120000 });
  await expect(page.getByRole('button', { name: 'New chat', exact: true }).first()).toBeVisible({ timeout: 30000 });
  await page.getByRole('button', { name: 'New chat', exact: true }).first().click();
  const composer = page.getByLabel('Task message', { exact: true });
  await expect(composer).toBeVisible();
  await composer.fill('Inspect the production deployment plan before any action.');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();

  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(1);
  await expect(page.getByText(
    'Jev flagged this for human review before any consequential effect. The chat was started without granting additional authority. Jev recommends frontier · xhigh reasoning (25% confidence). Your model selection was kept.',
    { exact: true },
  )).toBeVisible();
  expect(await page.locator('body').innerText()).not.toContain('It was not started');
  expect(errors).toEqual([]);
  console.log('Review-required Jev routing starts the first chat without granting authority.');
} finally {
  await browser.close();
}
