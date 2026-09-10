import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433');
  await expect(page.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible({ timeout: 60000 });
  await page.evaluate(() => {
    const q = window.__MONITTER_QA__, s = q.snapshot(), now = Date.now();
    s.tasks = [{ id: 'autoname-chat', agentId: 'atlas', title: 'Old chat title', nativeSessionId: 'native-stays', status: 'completed', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' }];
    s.messages = [{ id: 'chat-message', taskId: 'autoname-chat', role: 'user', text: 'Discuss the release checklist.', createdAt: now }];
    s.channels = [{ id: 'autoname-channel', name: 'Old channel title', description: '', agentIds: ['atlas'], messages: [{ id: 'channel-message', role: 'user', agentId: null, text: 'Coordinate the release.', createdAt: now, taskId: null }] }];
    q.setSnapshot(s);
  });
  await page.locator('.sidebar .task-select').filter({ hasText: 'Old chat title' }).click();
  const composer = page.getByLabel('Task message', { exact: true });
  await composer.fill('Unsent draft survives naming');
  await page.evaluate(()=>{const bridge=window.__MONITTER_BRIDGE__,original=bridge.autoname;bridge.autoname=async target=>{bridge.autoname=original;await new Promise(resolve=>window.__finishNaming=resolve);return original(target);};});
  await page.keyboard.press('Meta+p');
  await page.getByRole('dialog', { name: 'Controls' }).getByText('Auto-name current pane', { exact: true }).click();
  await expect(page.locator('.conversation-head .animated-title.naming')).toHaveText('Old chat title');
  await expect(page.locator('.tab-entry .animated-title.naming')).toHaveCount(1);
  await page.evaluate(()=>window.__finishNaming());
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].title)).toBe('Generated chat title');
  await expect(page.locator('.animated-title.naming')).toHaveCount(0);
  await expect(composer).toHaveValue('Unsent draft survives naming');
  const beforeSuggestions = await page.locator('.composer').boundingBox();
  await composer.fill('/autoname');
  const suggestions=page.getByRole('menu',{name:'Monitter commands'});
  await expect(suggestions).toBeVisible();
  await expect(composer).toBeFocused();
  const menuBounds=await suggestions.boundingBox(),inputBounds=await page.locator('.composer').boundingBox();
  expect(menuBounds.y+menuBounds.height).toBeLessThanOrEqual(inputBounds.y);
  expect(inputBounds.height).toBeCloseTo(beforeSuggestions.height,0);
  expect(inputBounds.y).toBeCloseTo(beforeSuggestions.y,0);
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'autoname').length)).toBe(2);
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].nativeSessionId)).toBe('native-stays');
  await expect(composer).toHaveValue('');

  await page.keyboard.press('Meta+k');
  await page.getByRole('dialog', { name: 'Switch to' }).getByText('Old channel title', { exact: true }).click();
  await page.getByLabel('Channel message', { exact: true }).fill('/autoname');
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().channels[0].name)).toBe('Generated channel title');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().channels[0].agentIds)).toEqual(['atlas']);

  await page.getByRole('button', { name: 'Open terminal', exact: true }).click();
  await expect(page.locator('.terminal-pane .xterm-helper-textarea')).toBeAttached();
  const terminal = await page.evaluate(() => window.__MONITTER_QA__.terminals()[0]);
  await page.keyboard.press('Meta+p');
  const controls = page.getByRole('dialog', { name: 'Controls' });
  await controls.getByText('Auto-name current pane', { exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.terminals()[0]?.title)).toBe('Generated terminal title');
  expect(await page.evaluate(() => window.__MONITTER_QA__.terminals()[0]?.id)).toBe(terminal.id);
  const calls = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'autoname'));
  expect(calls.map(call => Object.keys(call.args.target).filter(key => key !== 'content').sort().join(','))).toEqual(['taskId', 'taskId', 'channelId', 'terminalId']);
  console.log('Auto-name renames chat, channel, and terminal without changing native IDs or forwarding to the shell.');
} finally { await browser.close(); }
