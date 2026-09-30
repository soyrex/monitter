import { chromium, webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Delay real fixture bridge calls indefinitely until the test releases them.
// These checks exercise the rendered app without starting native agents.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18483';
const fixture = readFileSync('scripts/ui-fixture.js', 'utf8');
const setup = `
  document.documentElement?.setAttribute('data-monitter-lan', '1');
  document.addEventListener('DOMContentLoaded', () => document.documentElement.setAttribute('data-monitter-lan', '1'));
  const qa = window.__MONITTER_QA__, bridge = window.__MONITTER_BRIDGE__;
  const state = qa.snapshot();
  state.tasks = ['alpha', 'beta'].map((id, index) => ({
    id, agentId: 'atlas', title: id === 'alpha' ? 'Alpha chat' : 'Beta chat',
    nativeSessionId: null, status: 'completed', archived: false,
    createdAt: Date.now() + index, updatedAt: Date.now() + index, parentTaskId: null,
    channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test',
    provider: 'codex', model: '', sandbox: 'read-only'
  }));
  state.channels = [{ id: 'team', name: 'Team channel', description: '', agentIds: ['atlas'], messages: [] }];
  qa.setSnapshot(state);
  qa.held = {};
  qa.hold = method => {
    const original = bridge[method];
    const held = qa.held[method] = { started: false };
    bridge[method] = async (...args) => {
      bridge[method] = original;
      held.started = true;
      held.args = args;
      await new Promise((resolve, reject) => {
        held.release = resolve;
        held.reject = () => reject(new Error('Delayed ' + method + ' failure'));
      });
      return original(...args);
    };
  };
`;

for (const [engine, browserType] of [['WebKit', webkit], ['Chromium', chromium]]) {
  const browser = await browserType.launch({ headless: true });
  const errors = [];
  const open = async extra => {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript({ content: fixture + setup + (extra || '') });
    await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120000 });
    await expect(page.getByRole('button', { name: 'New chat with Atlas', exact: true })).toBeVisible({ timeout: 60000 });
    return page;
  };
  const task = async (page, title) => {
    await page.keyboard.press('Meta+k');
    await page.getByRole('dialog', { name: 'Switch to', exact: true }).getByText(title, { exact: true }).click();
    // The closing switch palette briefly adopts the Controls label; wait for
    // its exit animation before issuing another palette keyboard shortcut.
    await expect(page.locator('dialog.palette')).toHaveCount(0);
    await expect(page.locator('.task-layout .conversation-head')).toContainText(title);
  };
  const input = page => page.getByLabel('Task message', { exact: true });
  const send = page => page.getByRole('button', { name: /^(Send task message|Starting task)$/ });
  const hold = (page, method) => page.evaluate(method => window.__MONITTER_QA__.hold(method), method);
  const started = (page, method) => expect.poll(() => page.evaluate(method => !!window.__MONITTER_QA__.held[method]?.started, method)).toBe(true);
  const release = (page, method, fail = false) => page.evaluate(({ method, fail }) => window.__MONITTER_QA__.held[method][fail ? 'reject' : 'release'](), { method, fail });
  const calls = (page, method) => page.evaluate(method => window.__MONITTER_QA__.calls.filter(call => call.method === method), method);
  try {
    // An unresolved send owns only its chat; new typing and another chat work.
    let page = await open();
    await task(page, 'Alpha chat');
    await hold(page, 'sendMessage');
    await input(page).fill('First alpha message');
    await send(page).click();
    await started(page, 'sendMessage');
    await input(page).fill('Next alpha draft');
    await expect(send(page)).toBeDisabled();
    await input(page).press('Control+Enter');
    await task(page, 'Beta chat');
    await input(page).fill('Beta can send independently');
    await expect(send(page)).toBeEnabled();
    await send(page).click();
    await expect.poll(async () => (await calls(page, 'sendMessage')).length).toBe(1);
    await task(page, 'Alpha chat');
    await release(page, 'sendMessage', true);
    await expect(input(page)).toHaveValue('Next alpha draft');
    await expect(page.getByText('First alpha message', { exact: true })).toBeVisible();
    await expect(page.getByText('Not confirmed', { exact: true })).toBeVisible();
    expect((await calls(page, 'sendMessage')).map(call => call.args.taskId)).toEqual(['beta']);

    // A retry must reserve its target before checking whether the first send
    // was accepted. Two synchronous clicks must not cause two submissions.
    await hold(page, 'getSnapshot');
    await page.locator('.delivery-retry').evaluate(button => { button.click(); button.click(); });
    await started(page, 'getSnapshot');
    await expect(page.locator('.delivery-retry')).toBeDisabled();
    await expect(send(page)).toBeDisabled();
    await release(page, 'getSnapshot', true);
    await expect(page.locator('.delivery-retry')).toBeEnabled();
    await expect(page.getByText(/Could not refresh before retrying/)).toBeVisible();
    expect((await calls(page, 'sendMessage')).map(call => call.args.taskId)).toEqual(['beta']);
    await hold(page, 'getSnapshot');
    await page.locator('.delivery-retry').evaluate(button => { button.click(); button.click(); });
    await started(page, 'getSnapshot');
    await release(page, 'getSnapshot');
    await expect.poll(async () => (await calls(page, 'sendMessage')).length).toBe(2);
    expect((await calls(page, 'sendMessage')).map(call => call.args.taskId)).toEqual(['beta', 'alpha']);
    await expect(input(page)).toHaveValue('Next alpha draft');
    console.log(`${engine}: delayed task send and recovery passed.`);
    await page.close();

    // A global foreground command in one chat must not block sending elsewhere.
    page = await open(`state.tasks[0].status = 'running'; qa.setSnapshot(state);`);
    await task(page, 'Alpha chat');
    await hold(page, 'cancelTask');
    await page.keyboard.press('Meta+p');
    await page.getByRole('dialog', { name: 'Controls', exact: true }).getByRole('button', { name: 'Stop current chat', exact: true }).click();
    await started(page, 'cancelTask');
    await expect(page.getByRole('dialog', { name: 'Controls', exact: true })).toHaveCount(0);
    await task(page, 'Beta chat');
    await input(page).fill('Send while another chat stops');
    await expect(send(page)).toBeEnabled();
    await send(page).click();
    await expect.poll(async () => (await calls(page, 'sendMessage')).length).toBe(1);
    await release(page, 'cancelTask');
    console.log(`${engine}: send during unrelated stop passed.`);
    await page.close();

    // Channel sends also have local duplicate guards and preserve later input.
    page = await open();
    await page.keyboard.press('Meta+k');
    await page.getByRole('dialog', { name: 'Switch to', exact: true }).getByText('Team channel', { exact: true }).click();
    const channelInput = page.getByLabel('Channel message', { exact: true });
    const channelSend = page.getByRole('button', { name: /^(Send channel message|Starting channel message)$/ });
    await hold(page, 'sendChannelMessage');
    await channelInput.fill('@Atlas First channel message');
    await channelSend.click();
    await started(page, 'sendChannelMessage');
    await channelInput.fill('Next channel draft');
    await expect(channelSend).toBeDisabled();
    await channelInput.press('Control+Enter');
    await release(page, 'sendChannelMessage', true);
    await expect(channelInput).toHaveValue('Next channel draft');
    await expect(page.locator('.optimistic-message')).toContainText('First channel message');
    await expect(page.getByText('Not confirmed', { exact: true })).toBeVisible();
    expect(await calls(page, 'sendChannelMessage')).toHaveLength(0);
    console.log(`${engine}: delayed channel send and recovery passed.`);
    await page.close();

    // Slow routing and creation must keep the original send separate from edits.
    page = await open(`
      state.agents[0].jevRouting = 'recommend'; qa.setSnapshot(state);
      bridge.planJevRoute = async () => ({ traceId: 'delayed-route', promptFingerprint: 'fixture',
        decision: { task_kind: 'coding', model_tier: 'frontier', reasoning_level: 'high', execution_mode: 'inspect', permission_tier: 'safe', confidence: 0.8, rationale: 'Fixture', escalation_conditions: [] },
        classifierEvidence: { provider: 'Fixture', model: 'fixture', latencyMs: 1, inputTokens: 1, outputTokens: 1, costUsd: null }
      });
      bridge.recordJevRoute = async () => {};
      qa.hold('planJevRoute'); qa.hold('createTask');
    `);
    await page.getByRole('button', { name: 'New chat with Atlas', exact: true }).click();
    await input(page).fill('Original routed message');
    await input(page).press('Control+Enter');
    await started(page, 'planJevRoute');
    await expect(input(page)).toBeFocused();
    await page.keyboard.type('Follow-up typed during routing');
    await expect(send(page)).toBeDisabled();
    await input(page).press('Control+Enter');
    await release(page, 'planJevRoute');
    await started(page, 'createTask');
    await expect(input(page)).toHaveValue('Follow-up typed during routing');
    await input(page).fill('Follow-up typed during creation');
    await task(page, 'Beta chat');
    await input(page).fill('Unrelated beta draft');
    await release(page, 'createTask');
    await expect.poll(async () => (await calls(page, 'sendMessage')).length).toBe(1);
    expect((await calls(page, 'sendMessage'))[0].args.text).toBe('Original routed message');
    expect(await calls(page, 'createTask')).toHaveLength(1);
    await expect(input(page)).toHaveValue('Unrelated beta draft');
    await expect(input(page)).toBeFocused();
    await task(page, 'Original routed message');
    await expect(input(page)).toHaveValue('Follow-up typed during creation');
    console.log(`${engine}: delayed routing, creation and tab switch passed.`);
    await page.close();

    // Replacing the setup/pending/transcript DOM must preserve keyboard focus.
    page = await open(`qa.hold('createTask');`);
    await page.getByRole('button', { name: 'New chat with Atlas', exact: true }).click();
    await input(page).fill('Keyboard-created chat');
    await input(page).press('Control+Enter');
    await started(page, 'createTask');
    await expect(input(page)).toBeFocused();
    await page.keyboard.type('Keep typing through creation');
    await release(page, 'createTask');
    await expect(page.locator('.task-layout')).toBeVisible();
    await expect(input(page)).toHaveValue('Keep typing through creation');
    await expect(input(page)).toBeFocused();
    await page.close();

    // A failed creation retains both the submitted copy and subsequent edits.
    page = await open(`qa.hold('createTask');`);
    await page.getByRole('button', { name: 'New chat with Atlas', exact: true }).click();
    await input(page).fill('Creation that will fail');
    await input(page).press('Control+Enter');
    await started(page, 'createTask');
    await expect(input(page)).toBeFocused();
    await input(page).fill('Do not erase this newer draft');
    await input(page).press('Control+Enter');
    await release(page, 'createTask', true);
    await expect(input(page)).toHaveValue('Do not erase this newer draft');
    await expect(input(page)).toBeFocused();
    await expect(page.getByText('Not confirmed', { exact: true })).toBeVisible();
    await expect(page.getByText('Creation that will fail', { exact: true }).last()).toBeVisible();
    await page.locator('.delivery-retry').click();
    await expect(page.getByRole('region', { name: 'Starting chat', exact: true })).toHaveCount(0);
    await expect(input(page)).toHaveValue('Do not erase this newer draft');
    expect(await calls(page, 'createTask')).toHaveLength(0);
    await page.close();

    expect(errors).toEqual([]);
    console.log(`${engine}: delayed task/channel sends, unrelated commands, routed draft switches and failed creation preserve responsive input and draft text.`);
  } finally {
    await browser.close();
  }
}
