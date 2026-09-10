import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Browser QA only: the injected fixture never starts a native provider.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18421';
const output = 'verification/ui-composer-status-results.json';
mkdirSync('verification', { recursive: true });

const browser = await chromium.launch({ headless: true });
const passed = [];
const pageErrors = [];
let page;

try {
  page = await browser.newPage({ viewport: { width: 1180, height: 760 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Monitter menu', exact: true })).toBeVisible({ timeout: 30000 });

  const snapshot = () => page.evaluate(() => window.__MONITTER_QA__.snapshot());
  const calls = method => page.evaluate(method => window.__MONITTER_QA__.calls.filter(call => call.method === method), method);
  const taskMessage = page.getByLabel('Task message', { exact: true });
  const channelMessage = page.getByLabel('Channel message', { exact: true });
  const composerAction = () => page.locator('.composer-footer button[aria-label]').last();
  const openDraft = async title => {
    await page.keyboard.press('Meta+p');
    const palette = page.getByRole('dialog');
    await expect(palette).toBeVisible();
    await palette.getByText('New chat', { exact: true }).click();
    if (title) {
      const advanced = page.locator('details.draft-advanced');
      await advanced.evaluate(element => { element.open = true; });
      await page.getByLabel('Task title', { exact: true }).fill(title);
    }
  };
  const defer = async method => page.evaluate(method => {
    const bridge = window.__MONITTER_BRIDGE__;
    const original = bridge[method];
    let release;
    const pending = new Promise(resolve => { release = resolve; });
    window.__MONITTER_QA__.deferred = { method, release };
    bridge[method] = async (...args) => {
      bridge[method] = original;
      await pending;
      return original(...args);
    };
  }, method);
  const release = () => page.evaluate(() => window.__MONITTER_QA__.deferred.release());
  const seed = state => page.evaluate(state => window.__MONITTER_QA__.setSnapshot(state), state);

  // The initial draft has no native task yet. Its action remains an icon-only Send control.
  await openDraft('Pending startup');
  await taskMessage.fill('Create this task after a deliberately delayed bridge response.');
  await expect(composerAction()).toHaveAccessibleName('Send task message');
  await expect(composerAction()).toHaveText('');
  await defer('createTask');
  await composerAction().click();
  await expect(composerAction()).toHaveAccessibleName('Starting task');
  await expect(composerAction()).toHaveText('');
  await page.screenshot({ path: 'verification/ui-composer-pending.png' });
  await release();
  await expect.poll(() => calls('sendMessage').then(entries => entries.length)).toBe(1);
  let startingState = await snapshot();
  const startingTask = startingState.tasks.find(task => task.title === 'Pending startup');
  // A stored running status and provider startup stderr are not proof of this turn doing work.
  startingState.messages.unshift(
    { id: 'historical-user', taskId: startingTask.id, role: 'user', text: 'Old turn', createdAt: Date.now() - 2_000 },
    { id: 'historical-assistant', taskId: startingTask.id, role: 'assistant', text: 'Old result', createdAt: Date.now() - 1_900 },
  );
  startingState.events = [
    { id: 'historical-tool', taskId: startingTask.id, kind: 'tool', title: 'Old tool', detail: 'prior turn', createdAt: Date.now() - 1_000 },
    { id: 'startup-stderr', taskId: startingTask.id, kind: 'error', title: 'Provider stderr', detail: 'native startup diagnostic', createdAt: Date.now() },
  ];
  await seed(startingState);
  await expect(composerAction()).toHaveAccessibleName('Stop current task');
  await expect(composerAction().locator('.spin')).toHaveCount(1);
  await expect(composerAction()).toHaveText('');
  passed.push('draft pending/startup transitions from icon-only Send through progress to Stop');

  // Startup is cancellable even before a current-turn activity event arrives.
  const started = (await snapshot()).tasks.find(task => task.title === 'Pending startup');
  await composerAction().click();
  await expect.poll(() => calls('cancelTask').then(entries => entries.at(-1)?.args)).toBe(started.id);
  await expect(composerAction()).toHaveAccessibleName('Send task message');
  passed.push('running startup with only stderr remains cancellable and resets to Send');

  // A delayed send in chat A must not lend its progress indicator to chat B.
  await openDraft('Second chat');
  await taskMessage.fill('Create second task.');
  await composerAction().click();
  await expect.poll(() => snapshot().then(state => state.tasks.some(task => task.title === 'Second chat'))).toBe(true);
  let state = await snapshot();
  const second = state.tasks.find(task => task.title === 'Second chat');
  await expect(composerAction()).toHaveAccessibleName('Stop current task');
  await expect(composerAction().locator('.spin')).toHaveCount(1);
  // A current-turn tool event proves activity and upgrades the startup spinner to Stop.
  state.events.push({ id: 'second-tool', taskId: second.id, kind: 'tool', title: 'Current turn tool', detail: 'QA fixture event', createdAt: Date.now() });
  await seed(state);
  await expect(composerAction()).toHaveAccessibleName('Stop current task');
  await expect(composerAction().locator('.spin')).toHaveCount(0);
  await expect(taskMessage).toBeDisabled();
  const sendsWhileRunning = (await calls('sendMessage')).length;
  await taskMessage.dispatchEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true, cancelable: true });
  await expect.poll(() => calls('sendMessage').then(entries => entries.length)).toBe(sendsWhileRunning);
  await composerAction().click();
  await page.getByRole('button', { name: 'Pending startup', exact: true }).first().click();
  await taskMessage.fill('A delayed follow-up.');
  await defer('sendMessage');
  await composerAction().click();
  await expect(composerAction()).toHaveAccessibleName('Starting task');
  await page.getByRole('button', { name: 'Second chat', exact: true }).first().click();
  await expect(composerAction()).toHaveAccessibleName('Send task message');
  await release();
  await expect.poll(() => calls('sendMessage').then(entries => entries.length)).toBe(3);
  passed.push('current-turn activity unlocks Stop; switching chats never displays another chat’s pending spinner');

  // Terminal/error snapshots must restore a Send action rather than leaving a stale Stop.
  let terminal = await snapshot();
  terminal.tasks.find(task => task.id === second.id).status = 'completed';
  await seed(terminal);
  await expect(composerAction()).toHaveAccessibleName('Send task message');
  terminal = await snapshot();
  terminal.tasks.find(task => task.id === second.id).status = 'error';
  await seed(terminal);
  await expect(composerAction()).toHaveAccessibleName('Send task message');
  passed.push('completed and failed task snapshots restore Send');

  // The task action popover is a top-layer control, not content clipped by a scrolled pane.
  const clippingState = await snapshot();
  clippingState.settings.interfaceScale = 200;
  clippingState.messages.push(...Array.from({ length: 18 }, (_, index) => ({
    id: `menu-history-${index}`, taskId: second.id, role: 'assistant',
    text: `Long menu clipping history ${index}. ${'Scroll only the message pane. '.repeat(14)}`,
    createdAt: Date.now() + index,
  })));
  await seed(clippingState);
  await page.setViewportSize({ width: 760, height: 360 });
  const messageViewport = page.locator('.messages');
  await messageViewport.evaluate(element => { element.scrollTop = element.scrollHeight; });
  const taskActions = page.getByRole('button', { name: 'Task actions', exact: true });
  await taskActions.click();
  const overflow = page.locator('.task-menu');
  await expect(overflow).toBeVisible();
  const actionButtons = overflow.getByRole('button');
  expect(await actionButtons.count()).toBeGreaterThan(1);
  for (let index = 0; index < await actionButtons.count(); index++) {
    const action = actionButtons.nth(index);
    const box = await action.boundingBox();
    expect(box).not.toBeNull();
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(360);
    const exposed = await action.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
      return hit === element || element.contains(hit);
    });
    expect(exposed).toBe(true);
  }
  await actionButtons.first().click();
  passed.push('task action menu remains fully exposed and clickable above a scrolled pane at 200% scale');

  // Channel status is derived from only its linked tasks; unrelated running chats cannot supply Stop.
  const base = await snapshot();
  const now = Date.now();
  base.channels.push({ id: 'status-channel', name: 'Status channel', description: 'QA status scope.', agentIds: ['atlas'], messages: [] });
  base.tasks.push(
    { id: 'channel-running', agentId: 'atlas', title: 'Channel worker', nativeSessionId: null, status: 'running', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: 'status-channel', projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' },
    { id: 'unrelated-running', agentId: 'atlas', title: 'Unrelated worker', nativeSessionId: null, status: 'running', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' },
  );
  await seed(base);
  await page.getByRole('button', { name: /Status channel/ }).first().click();
  await expect(channelMessage).toBeVisible();
  await expect(composerAction()).toHaveAccessibleName('Stop channel tasks');
  await composerAction().click();
  await expect.poll(() => calls('cancelTask').then(entries => entries.at(-1)?.args)).toBe('channel-running');
  expect((await calls('cancelTask')).map(entry => entry.args)).not.toContain('unrelated-running');
  await expect(composerAction()).toHaveAccessibleName('Send channel message');
  passed.push('channel Stop is scoped to linked running tasks and ignores unrelated chat activity');

  // A channel without a running linked task stays ready to send even if a different chat runs.
  const scoped = await snapshot();
  scoped.tasks.find(task => task.id === 'channel-running').status = 'completed';
  scoped.tasks.find(task => task.id === 'unrelated-running').status = 'running';
  await seed(scoped);
  await expect(composerAction()).toHaveAccessibleName('Send channel message');
  await channelMessage.fill('A scoped channel delivery.');
  await page.locator('.recipient-picker').getByRole('button', { name: 'Atlas', exact: true }).click();
  await composerAction().click();
  await expect.poll(() => calls('sendChannelMessage').then(entries => entries.length)).toBe(1);
  passed.push('channel send remains available when only unrelated chats are running');

  expect(pageErrors).toEqual([]);
  const report = { completedAt: new Date().toISOString(), passed, pageErrors };
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  const report = { completedAt: new Date().toISOString(), passed, pageErrors, error: String(error) };
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
  if (page) await page.screenshot({ path: 'verification/ui-composer-status-failure.png' });
  throw error;
} finally {
  await browser.close();
}
