import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Start the development server first. The injected transport is confined to this test.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const results = [];
let page;
const errors = [];
try {
  page = await browser.newPage({ viewport: { width: 1440, height: 980 } });
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'New agent', exact: true }).first()).toBeVisible({ timeout: 30000 });
  const openDraftOptions = async () => {
    const options = page.locator('details.draft-advanced');
    await expect(options).toBeVisible();
    await options.evaluate(element => { element.open = true; });
    await expect.poll(() => options.evaluate(element => element.open)).toBe(true);
  };
  const openTaskActions = async action => {
    if (await action.isVisible().catch(() => false)) return;
    await page.getByRole('button', { name: 'Task actions', exact: true }).click();
    await expect(action).toBeVisible();
  };
  const appMenuButton = page.getByRole('button', { name: 'Monitter menu', exact: true });
  const openAppMenu = async entry => {
    if (await entry.isVisible().catch(() => false)) return;
    await appMenuButton.click();
    await expect(entry).toBeVisible();
  };

  await page.getByRole('button', { name: 'New agent', exact: true }).first().click();
  let dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name', { exact: true }).fill('Scout');
  await dialog.getByLabel('Description', { exact: true }).fill('UI verification agent');
  await dialog.getByLabel('Instructions', { exact: true }).fill('Read carefully. Never push changes.');
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents.length)).toBe(2);
  const scoutId = await page.evaluate(() => window.__MONITTER_QA__.snapshot().agents.find(agent => agent.name === 'Scout').id);
  results.push('create agent');

  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Review local workspace');
  await expect(page.getByLabel('Task message', { exact: true })).toBeVisible();
  await page.getByLabel('Task message', { exact: true }).fill('Check the workspace.');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(0);
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(0);
  results.push('new task stays a local draft until Send');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(1);
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  await expect(page.getByLabel('Task message', { exact: true })).toBeDisabled();
  await page.screenshot({ path: 'verification/ui-running-light.png' });
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(page.getByLabel('Task message', { exact: true })).toBeEnabled();
  results.push('send / running state / stop');

  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Retry created task');
  await page.getByLabel('Task message', { exact: true }).fill('TEST_FAILURE');
  const tasksBeforeRetry = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length);
  const createsBeforeRetry = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByText('Deliberate QA transport failure', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('TEST_FAILURE');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(tasksBeforeRetry + 1);
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(createsBeforeRetry + 1);
  await page.getByLabel('Task message', { exact: true }).fill('Retry without creating another task.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(task => task.title === 'Retry created task').status)).toBe('running');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(tasksBeforeRetry + 1);
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(createsBeforeRetry + 1);
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  results.push('draft send creates once; failed first send retries the same created task');

  // Draft tabs keep their own title, agent selection, and message without creating native work.
  const createsBeforeDraftTabs = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length);
  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Persistent draft A');
  await page.getByLabel('Agent', { exact: true }).selectOption(scoutId);
  await page.getByLabel('Task message', { exact: true }).fill('Draft A text persists.');
  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Persistent draft B');
  await page.getByLabel('Task message', { exact: true }).fill('Draft B text persists.');
  await page.getByRole('button', { name: 'Persistent draft A', exact: true }).click();
  await openDraftOptions();
  await expect(page.getByLabel('Task title', { exact: true })).toHaveValue('Persistent draft A');
  await expect(page.getByLabel('Agent', { exact: true })).toHaveValue(scoutId);
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('Draft A text persists.');
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(createsBeforeDraftTabs);

  // A delayed draft creation must only resolve its own tab; it must not navigate away from another draft.
  await page.evaluate(() => {
    const original = window.__MONITTER_BRIDGE__.createTask;
    let release;
    const pending = new Promise(resolve => { release = resolve; });
    window.__MONITTER_QA__.releaseDraftCreate = release;
    window.__MONITTER_BRIDGE__.createTask = async (...args) => {
      window.__MONITTER_BRIDGE__.createTask = original;
      await pending;
      return original(...args);
    };
  });
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await page.getByRole('button', { name: 'Persistent draft B', exact: true }).click();
  await page.evaluate(() => window.__MONITTER_QA__.releaseDraftCreate());
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.some(task => task.title === 'Persistent draft A'))).toBe(true);
  await openDraftOptions();
  await expect(page.getByLabel('Task title', { exact: true })).toHaveValue('Persistent draft B');
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('Draft B text persists.');
  results.push('draft tabs preserve selections and text; delayed creation stays scoped to its own draft');

  const preferences = page.getByRole('menuitem', { name: 'Preferences', exact: true });
  await openAppMenu(preferences);
  await preferences.click();
  dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'dark', exact: true }).click();
  await dialog.getByLabel('Custom accent colour', { exact: true }).fill('#8755c7');
  await dialog.getByLabel('Custom accent colour', { exact: true }).dispatchEvent('input');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.accent)).toBe('#8755c7');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await page.screenshot({ path: 'verification/ui-dark-violet.png' });
  results.push('theme / custom accent');

  await page.getByRole('button', { name: 'New channel', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name', { exact: true }).fill('release-room');
  await dialog.getByLabel('Description', { exact: true }).fill('A local channel for selected agents.');
  await dialog.getByRole('switch', { name: /Atlas/ }).check();
  await dialog.getByRole('switch', { name: /Scout/ }).check();
  await dialog.getByRole('button', { name: 'Save channel', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().channels.length)).toBe(1);
  results.push('create local channel');

  await page.getByRole('button', { name: /^release-room/ }).click();
  await page.getByLabel('Channel message', { exact: true }).fill('Please review this release.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByText('Choose at least one agent to receive this channel message.', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c => c.method === 'sendChannelMessage').length)).toBe(0);
  await page.locator('.recipient-picker').getByRole('button', { name: 'Scout', exact: true }).click();
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  const channelCall = await page.evaluate(() => window.__MONITTER_QA__.calls.find(c => c.method === 'sendChannelMessage'));
  expect(channelCall.args.agentIds).toEqual([scoutId]);
  results.push('channel routes only to explicitly selected agent');

  const hosts = page.getByRole('menuitem', { name: 'Hosts', exact: true });
  await openAppMenu(hosts);
  await hosts.click();
  await page.getByRole('dialog').getByRole('button', { name: 'Add host', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'SSH host', exact: true }).click();
  await dialog.getByLabel('Name', { exact: true }).fill('Mira QA');
  await dialog.getByLabel('Address', { exact: true }).fill('mira');
  await dialog.getByLabel('Default folder', { exact: true }).fill('~/monitter-test');
  await dialog.getByLabel('Codex CLI path', { exact: true }).fill('/home/alex/.npm-global/bin/codex');
  await dialog.getByRole('button', { name: 'Probe', exact: true }).click();
  await expect(dialog.getByText('Connection ready', { exact: true })).toBeVisible();
  await dialog.getByRole('button', { name: 'Save host', exact: true }).click();
  const savedHost = await page.evaluate(() => window.__MONITTER_QA__.snapshot().hosts.find(h => h.name === 'Mira QA'));
  expect(savedHost).toMatchObject({ kind: 'ssh', address: 'mira', user: '', port: 0, identityFile: '', defaultCwd: '~/monitter-test' });
  results.push('SSH host settings / probe / preserve SSH config defaults');

  await page.getByRole('button', { name: /Review local workspace/ }).first().click();
  const initialId = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].id);
  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Independent task');
  await page.getByLabel('Task message', { exact: true }).fill('Create the independent task.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.some(task => task.title === 'Independent task'))).toBe(true);
  let tasks = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks);
  const independentId = tasks.find(task => task.title === 'Independent task').id;
  expect(tasks.find(task => task.id === independentId).parentTaskId).toBe(null);
  const delegate = page.getByRole('button', { name: 'Delegate', exact: true });
  await openTaskActions(delegate);
  await delegate.click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Scout checks');
  await page.getByRole('combobox', { name: 'Agent', exact: true }).selectOption(scoutId);
  await page.getByLabel('Task message', { exact: true }).fill('Check the independent task.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.some(task => task.title === 'Scout checks'))).toBe(true);
  tasks = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks);
  const delegated = tasks.find(task => task.title === 'Scout checks');
  expect(delegated).toMatchObject({ parentTaskId: independentId, agentId: scoutId });
  expect(delegated.parentTaskId).not.toBe(initialId);
  results.push('independent tasks / explicit delegation to selected agent');

  await page.getByRole('button', { name: 'New task', exact: true }).first().click();
  await openDraftOptions();
  await page.getByLabel('Task title', { exact: true }).fill('Resume existing session');
  await page.getByLabel(/Existing native session ID/).fill('0198c016-2de4-7319-a926-4049fe7fb376');
  await page.getByLabel('Task message', { exact: true }).fill('Resume this native session.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.some(task => task.title === 'Resume existing session'))).toBe(true);
  tasks = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks);
  expect(tasks.find(task => task.title === 'Resume existing session').nativeSessionId).toBe('0198c016-2de4-7319-a926-4049fe7fb376');
  results.push('idle native session attachment preserves ID');

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const s = qa.snapshot(), taskId = s.tasks.at(-1).id;
    s.messages.push({ id: 'qa-markdown', taskId, role: 'assistant', createdAt: Date.now(), text: '**Safe markdown** <img src="x" onerror="window.__QA_XSS=true"> <script>window.__QA_XSS=true</script> [unsafe](javascript:window.__QA_XSS=true)' });
    qa.setSnapshot(s);
  });
  await expect(page.locator('.markdown strong').getByText('Safe markdown', { exact: true })).toBeVisible();
  expect(await page.locator('.markdown [onerror], .markdown script, .markdown a[href^="javascript:"]').count()).toBe(0);
  expect(await page.evaluate(() => window.__QA_XSS)).toBeUndefined();
  results.push('agent markdown sanitized before rendering');

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, s = qa.snapshot();
    s.events.push({ id: 'qa-large-diagnostic', taskId: s.tasks.at(-1).id, kind: 'error', title: 'Large harness diagnostic', detail: 'Diagnostic preview. ' + 'x'.repeat(50000) + ' END_OF_DIAGNOSTIC', createdAt: Date.now() });
    qa.setSnapshot(s);
  });
  const fullDetail = page.getByLabel('Full detail for Large harness diagnostic', { exact: true });
  await expect(fullDetail).not.toBeVisible();
  await page.getByLabel('Show full detail for Large harness diagnostic', { exact: true }).click();
  await expect(fullDetail).toBeVisible();
  await expect(fullDetail).toContainText('END_OF_DIAGNOSTIC');
  expect(await fullDetail.evaluate(el => el.clientHeight)).toBeLessThanOrEqual(250);
  await page.getByLabel('Show full detail for Large harness diagnostic', { exact: true }).click();
  results.push('large harness diagnostics collapse with full bounded details available');

  // Earlier draft-first creation checks intentionally send several chats. Reset their fixture
  // statuses so this section can still prove Stop is scoped to each of these two chats.
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    for (const task of state.tasks) task.status = 'idle';
    qa.setSnapshot(state);
  });
  await expect(page.getByLabel('Task message', { exact: true })).toBeEnabled();
  await page.getByLabel('Task message', { exact: true }).fill('Draft only for resumed session.');
  await page.getByRole('button', { name: /^Review local workspace/ }).first().click();
  await expect(page.getByLabel('Task message', { exact: true })).not.toHaveValue('Draft only for resumed session.');
  await page.getByLabel('Task message', { exact: true }).fill('Run the first task.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Resume existing session/ }).first().click();
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('Draft only for resumed session.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.filter(t => t.status === 'running').length)).toBe(2);
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.filter(t => t.status === 'running').length)).toBe(1);
  await page.getByRole('button', { name: /^Review local workspace/ }).first().click();
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.filter(t => t.status === 'running').length)).toBe(0);
  results.push('task drafts isolated / simultaneous runs / Stop targets selected task');

  const settingsButton = page.getByRole('menuitem', { name: 'Preferences', exact: true });
  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  await expect(dialog).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  expect(await dialog.evaluate(d => d.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(appMenuButton).toBeFocused();
  results.push('modal keyboard containment / Escape / focus restored');

  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  await expect(dialog.getByLabel('Interface scale', { exact: true })).toHaveValue('125');
  await dialog.getByLabel('Interface scale', { exact: true }).fill('200');
  await dialog.getByLabel('Interface scale', { exact: true }).dispatchEvent('change');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.interfaceScale)).toBe(200);
  await dialog.getByRole('button', {name: 'Reset to default · 125%', exact:true}).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.interfaceScale)).toBe(125);
  await expect.poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue('--interface-scale'))).toBe('1.25');
  await dialog.getByLabel('Interface scale', { exact: true }).fill('100');
  await dialog.getByLabel('Interface scale', { exact: true }).dispatchEvent('change');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  results.push('interface scale preference saved (native zoom verified separately)');
  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  const toolSwitch = dialog.getByRole('switch', {name:'Show tool activity',exact:true});
  await expect(toolSwitch).toBeChecked();
  await toolSwitch.focus();
  await toolSwitch.press('Space');
  await expect(toolSwitch).not.toBeChecked();
  await toolSwitch.press('Space');
  await expect(toolSwitch).toBeChecked();
  await page.screenshot({path:'verification/ui-appearance-toggles.png'});
  await dialog.getByRole('button', {name:'Close',exact:true}).click();
  results.push('toggle switches expose on/off state and support the Space key');

  await page.evaluate(taskId => {
    const state = window.__MONITTER_QA__.snapshot();
    state.events.push(
      {id:'qa-tool',taskId,kind:'tool',title:'QA tool call',detail:'safe tool output',createdAt:Date.now()},
      {id:'qa-reasoning',taskId,kind:'reasoning',title:'Reasoning',detail:'QA exposed reasoning summary.',createdAt:Date.now()+1},
    );
    window.__MONITTER_QA__.setSnapshot(state);
  }, initialId);
  await expect(page.locator('.messages').getByLabel('Tool activity: QA tool call', {exact:true})).toBeVisible();
  await expect(page.locator('.messages').getByLabel('Reasoning summary', {exact:true})).toBeVisible();
  await expect(page.locator('.messages').getByText('safe tool output', {exact:true})).not.toBeVisible();
  await page.locator('.messages').getByLabel('Tool activity: QA tool call', {exact:true}).click();
  await expect(page.locator('.messages').getByText('safe tool output', {exact:true})).toBeVisible();
  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Show tool activity', {exact:true}).uncheck();
  await dialog.getByRole('button', {name:'Close',exact:true}).click();
  await expect(page.locator('.messages').getByLabel('Tool activity: QA tool call', {exact:true})).toHaveCount(0);
  await expect(page.locator('.messages').getByLabel('Reasoning summary', {exact:true})).toBeVisible();
  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Show reasoning summaries', {exact:true}).uncheck();
  await dialog.getByRole('button', {name:'Close',exact:true}).click();
  await expect(page.locator('.messages .activity')).toHaveCount(0);
  await expect(page.locator('.event.tool, .event.reasoning')).toHaveCount(0);
  await openAppMenu(settingsButton);
  await settingsButton.click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Show tool activity', {exact:true}).check();
  await dialog.getByLabel('Show reasoning summaries', {exact:true}).check();
  await dialog.getByLabel('Enter to send', {exact:true}).check();
  await dialog.getByRole('button', {name:'Close',exact:true}).click();
  results.push('tool and reasoning blocks expand independently / toggles hide and restore recorded activity');

  const composer = page.getByLabel('Task message', {exact:true});
  const sendsBefore = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendMessage').length);
  await composer.fill('First line');
  await composer.press('Shift+Enter');
  await composer.pressSequentially('Second line');
  await composer.dispatchEvent('keydown', {key:'Enter',isComposing:true,bubbles:true});
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendMessage').length)).toBe(sendsBefore);
  await composer.press('Enter');
  await expect(page.getByRole('button', {name:'Stop',exact:true})).toBeVisible();
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendMessage').at(-1).args.text)).toBe('First line\nSecond line');
  await page.getByRole('button', {name:'Stop',exact:true}).click();
  await openAppMenu(settingsButton);
  await settingsButton.click();
  await page.getByRole('dialog').getByLabel('Enter to send', {exact:true}).uncheck();
  await page.getByRole('dialog').getByRole('button', {name:'Close',exact:true}).click();
  await composer.fill('Command mode');
  await composer.press('Enter');
  await expect(composer).toHaveValue('Command mode\n');
  await composer.press('Meta+Enter');
  await expect(page.getByRole('button', {name:'Stop',exact:true})).toBeVisible();
  await page.getByRole('button', {name:'Stop',exact:true}).click();
  results.push('Enter / Cmd+Enter preference with Shift+Enter newline and IME composition guard');

  await expect(page.locator('.topbar .tabs')).toBeVisible();
  expect(await page.locator('.topbar .tab-entry').count()).toBeGreaterThan(1);
  await expect(page.locator('.crumb, .conversation .tabs')).toHaveCount(0);
  const taskCountBeforeClose = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length);
  await page.getByRole('button', {name:'Close tab Independent task',exact:true}).click();
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.length)).toBe(taskCountBeforeClose);
  await page.screenshot({path:'verification/ui-header-activity.png'});
  results.push('multiple open task tabs in header / closing tab preserves task');

  await openAppMenu(settingsButton);
  await settingsButton.click();
  await page.getByRole('dialog').getByLabel('Enter to send', {exact:true}).check();
  await page.getByRole('dialog').getByRole('button', {name:'Close',exact:true}).click();
  await page.locator('.channel-row').filter({hasText:'release-room'}).click();
  await page.locator('.recipient-picker').getByRole('button', {name:'Scout',exact:true}).click();
  const channelComposer = page.getByLabel('Channel message', {exact:true});
  const channelSendsBefore = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendChannelMessage').length);
  await channelComposer.fill('Channel keyboard test');
  await channelComposer.press('Shift+Enter');
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendChannelMessage').length)).toBe(channelSendsBefore);
  await channelComposer.press('Enter');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='sendChannelMessage').length)).toBe(channelSendsBefore+1);
  await openAppMenu(settingsButton);
  await settingsButton.click();
  await page.getByRole('dialog').getByLabel('Enter to send', {exact:true}).uncheck();
  await page.getByRole('dialog').getByRole('button', {name:'Close',exact:true}).click();
  results.push('shared send-key preference applies to channel composer');

  await page.locator('.task-row').filter({hasText:'Review local workspace'}).click();
  await page.evaluate(taskId => {
    const state = window.__MONITTER_QA__.snapshot();
    for (let i=0; i<30; i++) state.messages.push({id:`qa-long-${i}`,taskId,role:'assistant',text:'A long conversation stays inside the message scroll area. '.repeat(5),createdAt:Date.now()+i});
    window.__MONITTER_QA__.setSnapshot(state);
  }, initialId);
  await page.setViewportSize({width:960,height:626});
  await expect(page.getByLabel('Task message', {exact:true})).toBeVisible();
  await expect.poll(() => page.locator('.composer').evaluate(el => el.getBoundingClientRect().bottom)).toBeLessThanOrEqual(626);
  expect(await page.locator('.topbar').evaluate(el => el.getBoundingClientRect().top)).toBe(0);
  expect(await page.locator('.messages').evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
  await page.screenshot({path:'verification/ui-large-scale-layout.png'});
  for (const viewport of [{width:720,height:490},{width:490,height:360}]) {
    await page.setViewportSize(viewport);
    await expect.poll(() => page.locator('.composer').evaluate(el => el.getBoundingClientRect().bottom)).toBeLessThanOrEqual(viewport.height);
    const bounds = await page.evaluate(() => ({
      root: document.documentElement.scrollHeight, height: innerHeight,
      width: document.documentElement.scrollWidth, viewportWidth: innerWidth,
      body: document.body.scrollHeight,
      messages: document.querySelector('.messages').clientHeight,
    }));
    expect(bounds.root).toBeLessThanOrEqual(bounds.height);
    expect(bounds.body).toBeLessThanOrEqual(bounds.height);
    expect(bounds.width).toBeLessThanOrEqual(bounds.viewportWidth);
    expect(bounds.messages).toBeGreaterThan(30);
    await page.mouse.move(viewport.width-120,viewport.height/2);
    await page.mouse.wheel(0,1600);
    expect(await page.locator('.topbar').evaluate(el => el.getBoundingClientRect().top)).toBe(0);
    expect(await page.evaluate(() => scrollY)).toBe(0);
  }
  await page.screenshot({path:'verification/ui-200-small-window.png'});
  await page.setViewportSize({width:1440,height:980});
  results.push('long conversations keep composer and header visible at a zoom-sized viewport');

  // Exercise real UI state transitions with explicitly labelled protocol fixtures.
  await page.evaluate(taskId => {
    const state = window.__MONITTER_QA__.snapshot();
    const now = Date.now();
    Object.assign(state.tasks.find(t=>t.id===taskId), {nativeSessionId:'qa-goal-native',status:'running'});
    state.goals = {[taskId]: {objective:'Verify the goal panel fixture',status:'active',tokensUsed:500,tokenBudget:2000}};
    state.messages.push({id:'qa-current-turn',taskId,role:'user',text:'QA current turn',createdAt:now});
    state.events.push({id:'qa-old-computer',taskId,kind:'computer',title:'Computer use',detail:JSON.stringify({id:'old',phase:'started',tool:'cua_repl.js'}),createdAt:now-1});
    window.__MONITTER_QA__.setSnapshot(state);
  }, initialId);
  await expect(page.getByRole('region', {name:'Task goal',exact:true})).toContainText('Verify the goal panel fixture');
  await expect(page.getByRole('progressbar', {name:'Goal token budget used'})).toHaveAttribute('max','2000');
  await expect(page.getByRole('region', {name:'Computer use activity'})).toHaveCount(0);
  const computerEvent = async (id,phase) => page.evaluate(({taskId,id,phase}) => {
    const state=window.__MONITTER_QA__.snapshot();
    state.events.push({id:crypto.randomUUID(),taskId,kind:'computer',title:'Computer use',detail:JSON.stringify({id,phase,tool:'cua_repl.js',summary:'QA browser action'}),createdAt:Date.now()});
    window.__MONITTER_QA__.setSnapshot(state);
  }, {taskId:initialId,id,phase});
  await computerEvent('live-tool','started');
  await expect(page.getByRole('region', {name:'Computer use activity'})).toContainText('QA browser action');
  await computerEvent('unrelated','completed');
  await expect(page.getByRole('region', {name:'Computer use activity'})).toBeVisible();
  await computerEvent('live-tool','completed');
  await expect(page.getByRole('region', {name:'Computer use activity'})).toHaveCount(0);
  await computerEvent('live-tool-2','started');
  await page.screenshot({path:'verification/ui-goal-computer-panel.png'});
  await page.setViewportSize({width:512,height:340});
  await expect.poll(() => page.locator('.composer').evaluate(el=>el.getBoundingClientRect().bottom)).toBeLessThanOrEqual(340);
  expect(await page.locator('.messages').evaluate(el=>el.clientHeight)).toBeGreaterThan(20);
  expect(await page.evaluate(()=>document.documentElement.scrollHeight)).toBeLessThanOrEqual(340);
  await page.setViewportSize({width:1440,height:980});
  await page.getByRole('button', {name:'Stop computer use',exact:true}).click();
  await expect(page.getByRole('region', {name:'Computer use activity'})).toHaveCount(0);
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c=>c.method==='cancelTask').at(-1).args)).toBe(initialId);
  results.push('native goal fixture / live computer lifecycle / historical activity ignored / scoped Stop');

  for (const provider of ['claude','opencode','hermes']) {
    await page.getByRole('button', {name:'New agent',exact:true}).first().click();
    dialog=page.getByRole('dialog');
    await dialog.getByLabel('Name', {exact:true}).fill(`QA ${provider}`);
    await dialog.getByLabel('Harness', {exact:true}).selectOption(provider);
    await expect(dialog.getByLabel('Permissions', {exact:true})).toHaveValue('harness-configured');
    await expect(dialog.getByLabel('Permissions', {exact:true}).locator('option')).toHaveCount(1);
    await dialog.getByRole('button', {name:'Save agent',exact:true}).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await page.evaluate(name=>window.__MONITTER_QA__.snapshot().agents.find(a=>a.name===name).sandbox,`QA ${provider}`)).toBe('harness-configured');
  }
  results.push('Claude / OpenCode / Hermes agent choices preserve explicit native permission policy');

  await page.keyboard.press('Meta+k');
  dialog = page.getByRole('dialog', {name:'Switch to',exact:true});
  await dialog.getByPlaceholder('Find a channel, chat or agent…').fill('release-room');
  await dialog.getByPlaceholder('Find a channel, chat or agent…').press('Enter');
  await expect(page.getByLabel('Channel message', {exact:true})).toBeVisible();
  await page.keyboard.press('Meta+k');
  dialog = page.getByRole('dialog', {name:'Switch to',exact:true});
  await dialog.getByPlaceholder('Find a channel, chat or agent…').fill('QA claude');
  await dialog.getByPlaceholder('Find a channel, chat or agent…').press('Enter');
  await expect(page.getByRole('heading', {name:'QA claude',exact:true})).toBeVisible();
  await page.getByRole('button', {name:'New chat with QA claude',exact:true}).click();
  await openDraftOptions();
  const claudeAgentId=await page.evaluate(()=>window.__MONITTER_QA__.snapshot().agents.find(a=>a.name==='QA claude').id);
  await expect(page.getByLabel('Agent',{exact:true})).toHaveValue(claudeAgentId);
  await page.getByLabel('Task title',{exact:true}).fill('Palette archive target');
  await page.getByLabel('Task message',{exact:true}).fill('A draft that survives archiving');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks.some(task=>task.title==='Palette archive target'))).toBe(true);
  const archivedId=await page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks.find(t=>t.title==='Palette archive target').id);
  await page.getByRole('button',{name:'Stop',exact:true}).click();
  await page.getByLabel('Task message',{exact:true}).fill('A draft that survives archiving');
  await page.getByRole('button',{name:'Archive chat Palette archive target',exact:true}).click();
  await expect(page.locator('.task-row').filter({hasText:'Palette archive target'})).toHaveCount(0);
  expect(await page.evaluate(id=>window.__MONITTER_QA__.snapshot().tasks.find(t=>t.id===id).archived,archivedId)).toBe(true);
  await page.keyboard.press('Meta+k');
  dialog=page.getByRole('dialog',{name:'Switch to',exact:true});
  await dialog.getByPlaceholder('Find a channel, chat or agent…').fill('Palette archive target');
  await expect(dialog).toContainText('Select to restore');
  await dialog.getByPlaceholder('Find a channel, chat or agent…').press('Enter');
  await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('A draft that survives archiving');
  expect(await page.evaluate(id=>window.__MONITTER_QA__.snapshot().tasks.find(t=>t.id===id).archived,archivedId)).toBe(false);
  await page.getByRole('button',{name:'Delete chat Palette archive target',exact:true}).click();
  await page.getByRole('dialog',{name:'Delete chat',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(page.locator('.task-row').filter({hasText:'Palette archive target'})).toHaveCount(1);
  await page.getByRole('button',{name:'Delete chat Palette archive target',exact:true}).click();
  await page.getByRole('dialog',{name:'Delete chat',exact:true}).getByRole('button',{name:'Delete chat',exact:true}).click();
  await expect(page.locator('.task-row').filter({hasText:'Palette archive target'})).toHaveCount(0);
  expect(await page.evaluate(id=>window.__MONITTER_QA__.snapshot().tasks.some(t=>t.id===id),archivedId)).toBe(false);
  results.push('Cmd-K channel / agent / chat switching, empty-agent plus, archive restore preserves draft, confirmed delete');

  await page.keyboard.press('Meta+p');
  dialog=page.getByRole('dialog',{name:'Controls',exact:true});
  const controlsSearch=dialog.getByPlaceholder('Find a control or setting…');
  await controlsSearch.fill('Enter to send');
  const enterBefore=await page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.sendWithEnter);
  await controlsSearch.press('Enter');
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.sendWithEnter)).toBe(!enterBefore);
  await expect(dialog.getByRole('button',{name:`Enter to send, ${!enterBefore?'on':'off'}`,exact:true})).toBeVisible();
  await controlsSearch.fill('');
  await page.setViewportSize({width:512,height:340});
  await expect.poll(()=>dialog.evaluate(el=>el.getBoundingClientRect().bottom)).toBeLessThanOrEqual(340);
  const compactPalette = await dialog.boundingBox();
  expect(compactPalette.x).toBeGreaterThanOrEqual(12);
  expect(compactPalette.x + compactPalette.width).toBeLessThanOrEqual(500);
  await controlsSearch.press('ArrowDown');
  await controlsSearch.press('ArrowDown');
  await page.keyboard.press('Tab');
  expect(await dialog.evaluate(el=>el.contains(document.activeElement))).toBe(true);
  await page.screenshot({path:'verification/ui-controls-small.png'});
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await page.setViewportSize({width:1440,height:980});
  await page.keyboard.press('Meta+k');
  const switcher = page.getByRole('dialog',{name:'Switch to',exact:true});
  const switcherBounds = await switcher.boundingBox();
  expect(Math.abs(switcherBounds.x + switcherBounds.width / 2 - 720)).toBeLessThan(1);
  expect(await switcher.evaluate(el=>getComputedStyle(el).color)).not.toBe('rgb(0, 0, 0)');
  await page.screenshot({path:'verification/ui-switcher.png'});
  await page.getByRole('button',{name:'Close command palette',exact:true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  results.push('Cmd-P settings toggle immediately / keyboard navigation / small viewport / Close via Enter');

  // Hold only the transport acknowledgement to exercise navigation during IPC.
  const holdSend = async method => page.evaluate(method => {
    const original = window.__MONITTER_BRIDGE__[method];
    let release;
    const pending = new Promise(resolve => { release = resolve; });
    window.__MONITTER_QA__.releaseSend = release;
    window.__MONITTER_BRIDGE__[method] = async (...args) => {
      window.__MONITTER_BRIDGE__[method] = original;
      await pending;
      return original(...args);
    };
  }, method);
  const switchTo = async title => {
    await page.keyboard.press('Meta+k');
    const search = page.getByRole('dialog',{name:'Switch to',exact:true}).getByPlaceholder('Find a channel, chat or agent…');
    await search.fill(title);
    await search.press('Enter');
  };
  await switchTo('Review local workspace');
  await page.getByLabel('Task message',{exact:true}).fill('Pending task send');
  await holdSend('sendMessage');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await switchTo('Independent task');
  await page.getByLabel('Task message',{exact:true}).fill('Keep this other draft');
  await page.evaluate(()=>window.__MONITTER_QA__.releaseSend());
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks.find(t=>t.title==='Review local workspace').status)).toBe('running');
  await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('Keep this other draft');
  await switchTo('Review local workspace');
  await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('');
  await page.getByRole('button',{name:'Stop',exact:true}).click();
  await switchTo('release-room');
  await page.locator('.recipient-picker').getByRole('button',{name:'Scout',exact:true}).click();
  await page.getByLabel('Channel message',{exact:true}).fill('Pending channel send');
  await holdSend('sendChannelMessage');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await switchTo('Independent task');
  await page.evaluate(()=>window.__MONITTER_QA__.releaseSend());
  await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().channels.find(c=>c.name==='release-room').messages.at(-1).text)).toBe('Pending channel send');
  await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('Keep this other draft');
  await switchTo('release-room');
  await expect(page.getByLabel('Channel message',{exact:true})).toHaveValue('');
  results.push('switching while task / channel sends are pending preserves unrelated drafts and clears only the sent draft');

  const preview = await browser.newPage();
  preview.on('pageerror', error => errors.push(error.message));
  await preview.goto(url);
  await expect(preview.getByText('Browser design preview — connect the native app to use hosts, agents, and tasks.', { exact: true })).toBeVisible();
  await expect(preview.getByRole('heading', { name: 'Start with one agent.', exact: true })).toBeVisible();
  expect(await preview.locator('body').innerText()).not.toContain('transformCallback');
  expect(await preview.locator('body').innerText()).not.toContain('Loading your workspace');
  await preview.close();
  results.push('browser preview has no fake agents or native bridge errors');

  expect(errors).toEqual([]);
  const report = { completedAt: new Date().toISOString(), passed: results, pageErrors: errors, screenshots: 'verification/' };
  writeFileSync('verification/ui-results.json', `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  console.error(error);
  console.error(JSON.stringify({ completed: results, pageErrors: errors, body: page ? (await page.locator('body').innerText()).slice(0,1600) : '' }));
  if (page) await page.screenshot({ path: 'verification/ui-failure.png' });
  throw error;
} finally {
  await browser.close();
}
