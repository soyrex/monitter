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
  const agent = state.agents.find(agent => agent.id === 'atlas');
  agent.model = 'qa-balanced';
  agent.jevRouting = 'safe_auto';
  agent.jevModelTiers = { fast: '', balanced: '', strong: '', frontier: 'qa-frontier' };
  state.modelCatalog = {
    models: [
      { id: 'qa-balanced', name: 'QA Balanced', reasoningEfforts: [{ id: 'medium' }], defaultEffort: 'medium', supportsFast: false },
      { id: 'qa-frontier', name: 'QA Frontier', reasoningEfforts: [{ id: 'xhigh' }], defaultEffort: 'xhigh', supportsFast: false }
    ],
    current: { model: 'qa-balanced', reasoningEffort: 'medium', fastMode: false },
    source: 'Browser QA fixture', warning: null
  };
  qa.setSnapshot(state);
  window.__MONITTER_BRIDGE__.planJevRoute = async () => ({
    traceId: 'route-confidence-fixture', promptFingerprint: 'sha256:fixture',
    decision: {
      task_kind: 'architecture', model_tier: 'frontier', reasoning_level: 'xhigh',
      execution_mode: 'inspect', permission_tier: 'human_review_required', confidence: 0.05,
      question_confidences: { task_kind: 0.36, model_tier: 0.05, reasoning_level: 0.08, execution_mode: 0.60, permission_tier: 0.42 },
      rationale: 'Fixture architecture route.', escalation_conditions: []
    },
    autoRoutePolicy: { confidence: 0.36, threshold: 0.30, eligible: true, basis: 'task_kind' },
    classifierEvidence: { provider: 'Fixture', model: 'jev-test', latencyMs: 1, inputTokens: 1, outputTokens: 1, costUsd: null }
  });
  window.__MONITTER_BRIDGE__.recordJevRoute = async () => {};
`);

try {
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120000 });
  await expect(page.getByRole('button', { name: 'New chat', exact: true }).first()).toBeVisible({ timeout: 30000 });
  await page.getByRole('button', { name: 'New chat', exact: true }).first().click();
  const composer = page.getByLabel('Task message', { exact: true });
  await composer.fill('Review the architecture and propose a migration plan.');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();

  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'createTask')?.args.modelSettings)).toEqual({
    model: 'qa-frontier', reasoningEffort: 'xhigh', fastMode: null
  });
  expect(await page.locator('body').innerText()).not.toContain('The harness default model was kept');
  expect(errors).toEqual([]);
  console.log('Low unrelated Jev confidence does not block a qualifying frontier/xhigh route.');
} finally {
  await browser.close();
}
