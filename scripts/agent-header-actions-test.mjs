import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const testUrl = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18453/';
const browser = await webkit.launch({ headless: true });

const task = (id, agentId, title) => ({
  id, agentId, title, nativeSessionId: null, status: 'completed', archived: false,
  createdAt: 1, updatedAt: 2, parentTaskId: null, channelId: null, projectId: null,
  hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '',
  modelSettings: null, sandbox: 'read-only',
});

function fixture(base) {
  base.agents = [
    { ...base.agents[0], id: 'atlas', name: 'Atlas' },
    { ...base.agents[0], id: 'beta', name: 'Beta' },
    { ...base.agents[0], id: 'gamma', name: 'Gamma' },
  ];
  base.tasks = [
    task('atlas-chat', 'atlas', 'Atlas existing chat'),
    task('gamma-chat', 'gamma', 'Gamma existing chat'),
  ];
  base.messages = [];
  base.events = [];
  base.channels = [];
  return base;
}

async function run(width, mobile) {
  const page = await browser.newPage({
    viewport: { width, height: mobile ? 844 : 900 },
    isMobile: mobile,
    hasTouch: mobile,
  });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  if (mobile) {
    await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
    await page.route(testUrl, async route => {
      const response = await route.fetch();
      await route.fulfill({ response, body: (await response.text()).replace('<html', '<html data-monitter-lan="1"') });
    });
  }
  await page.goto(testUrl, { timeout: 90000 });
  await expect(page.getByRole('complementary', { name: 'Agents and tasks', exact: true })).toBeVisible({ timeout: 60000 });
  const initial = await page.evaluate(() => window.__MONITTER_QA__.snapshot());
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), fixture(initial));

  const sidebar = page.getByRole('complementary', { name: 'Agents and tasks', exact: true });
  await expect(sidebar).toBeVisible();
  await expect(sidebar.getByRole('tab', { name: 'Agents view', exact: true })).toBeVisible();
  await expect(sidebar.locator('.sidebar-mode-content > .section-label')).toHaveCount(0);

  for (const name of ['Atlas', 'Beta', 'Gamma']) {
    const row = sidebar.locator('.agent-row').filter({ hasText: name });
    await expect(row).toHaveCount(1);
    const newChat = row.getByRole('button', { name: `New chat with ${name}`, exact: true });
    await expect(newChat).toBeVisible();
    await expect(newChat).toHaveText('');
    await expect(newChat).toHaveCSS('opacity', '1');
    const rowBox = await row.boundingBox();
    const buttonBox = await newChat.boundingBox();
    expect(rowBox).not.toBeNull();
    expect(buttonBox).not.toBeNull();
    const paddingRight = await row.evaluate(el => parseFloat(getComputedStyle(el).paddingRight));
    expect(rowBox.x + rowBox.width - buttonBox.x - buttonBox.width).toBeCloseTo(paddingRight, 1);
    if (mobile) {
      expect(buttonBox.width).toBeCloseTo(44, 1);
      expect(buttonBox.height).toBeCloseTo(44, 1);
    }
    await expect(row.getByRole('button', { name: `Edit ${name}`, exact: true })).toHaveCount(0);
  }

  // Existing chats may be collapsed, but the agent header action remains available.
  const gamma = sidebar.locator('.agent-group').filter({ hasText: 'Gamma' });
  await gamma.getByRole('button', { name: 'Collapse chats for Gamma', exact: true }).click();
  await expect(gamma.locator('.task-tree')).toBeHidden();
  await expect(gamma.getByRole('button', { name: 'New chat with Gamma', exact: true })).toBeVisible();

  const forbidden = () => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => ['createTask', 'sendMessage', 'resumeTask'].includes(call.method)));
  const before = await forbidden();
  await sidebar.locator('.agent-row').filter({ hasText: 'Beta' }).getByRole('button', { name: 'New chat with Beta', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Task message', exact: true })).toBeVisible();
  await expect(page.getByLabel('Agent', { exact: true })).toHaveValue('beta');
  expect(await forbidden()).toEqual(before);
  await page.getByLabel('Task message', { exact: true }).fill(`Draft for ${width}`);
  expect(await forbidden()).toEqual(before);
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(1);
  const create = await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'createTask'));
  expect(create.args.agentId).toBe('beta');
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'resumeTask'))).toEqual([]);
  expect(errors).toEqual([]);
  console.log(`WebKit ${width}px ${mobile ? 'touch' : 'desktop'} agent header checks passed.`);
  await page.close();
}

try {
  await run(390, true);
  await run(320, true);
  await run(1280, false);
} finally {
  await browser.close();
}
