import { expect, webkit } from '@playwright/test';

// Uses only the isolated browser fixture; no harness/account calls.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18472';
const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible({ timeout: 60000 });
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.tasks = [{
      id: 'new-title-chat', agentId: 'atlas', title: 'Old pane title', nativeSessionId: 'native-before-clear',
      status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null,
      projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
    }];
    qa.setSnapshot(state);
  });

  await page.locator('.sidebar .task-select').filter({ hasText: 'Old pane title' }).click();
  const composer = page.getByLabel('Task message', { exact: true });
  await composer.fill('/new Frontend Fixes');
  await expect(page.getByRole('menu', { name: 'Monitter commands' })).toContainText('/new');
  await composer.press('Enter');

  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].title)).toBe('Frontend Fixes');
  await expect(composer).toHaveValue('');
  const result = await page.evaluate(() => ({
    task: window.__MONITTER_QA__.snapshot().tasks[0],
    messages: window.__MONITTER_QA__.snapshot().messages,
    calls: window.__MONITTER_QA__.calls.filter(call => ['clearTaskContext', 'renameTask', 'sendMessage'].includes(call.method)),
  }));
  expect(result.task.nativeSessionId).toBe(null);
  expect(result.messages.at(-1)?.text).toBe('Context Cleared');
  expect(result.calls.map(call => call.method)).toEqual(['clearTaskContext', 'renameTask']);
  expect(result.calls[1].args).toEqual({ id: 'new-title-chat', title: 'Frontend Fixes' });

  const renameCount = result.calls.filter(call => call.method === 'renameTask').length;
  await page.evaluate(() => {
    const bridge = window.__MONITTER_BRIDGE__, original = bridge.clearTaskContext;
    bridge.clearTaskContext = async () => {
      bridge.clearTaskContext = original;
      throw Error('Clear failed fixture');
    };
  });
  await composer.fill('/new Must Not Replace Title');
  await composer.press('Enter');
  await expect(page.getByText('Clear failed fixture', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].title)).toBe('Frontend Fixes');
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'renameTask').length)).toBe(renameCount);

  const unexpectedErrors = errors.filter(error => !error.includes("window.__TAURI_INTERNALS__.transformCallback"));
  expect(unexpectedErrors).toEqual([]);
  console.log('/new <title> clears provider context before renaming the pane and never sends command text.');
} finally {
  await browser.close();
}
