import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Browser-only coverage for scoped desktop workspaces. The fixture replaces every
// native bridge method, so this never starts, resumes, or alters a real harness.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18431';
const output = 'verification/ui-scoped-workspaces-results.json';
mkdirSync('verification', { recursive: true });

const browser = await chromium.launch({ headless: true });
const passed = [], pageErrors = [];

async function newFixturePage(context, init) {
  const page = await context.newPage();
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(init);
  await page.goto(url);
  await expect(page.locator('[data-workspace-context]')).toBeVisible({ timeout: 30000 });
  return page;
}

try {
  // Verify that a legacy one-workspace session is carried forward as the All
  // activity workspace, without losing its task tab or unsent composer text.
  const legacyContext = await browser.newContext({ viewport: { width: 1280, height: 820 } });
  const legacyPage = await newFixturePage(legacyContext, () => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.tasks = [{ id: 'legacy-task', agentId: 'atlas', projectId: null, title: 'Legacy open chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, hostId: 'local', cwd: '/tmp/legacy', provider: 'codex', model: '', sandbox: 'read-only' }];
    state.settings.sidebarView = 'activity';
    qa.setSnapshot(state);
    localStorage.removeItem('monitter.workspaces.v2');
    localStorage.setItem('monitter.workspace.v1', JSON.stringify({
      version: 1, layout: { id: 'main' }, activePaneId: 'main', panes: {}, sidebarCollapsed: false,
      collapsedAgents: {}, collapsedProjects: {}, terminals: [],
      main: {
        overviewOpen: true, settingsOpen: false, settingsCategory: 'appearance', openTerminalIds: [], selectedTerminalId: null,
        openEmptyIds: [], selectedEmptyId: null, openTaskIds: ['legacy-task'], openDraftIds: ['legacy-draft'], openChannelIds: [],
        tabOrder: [{ kind: 'task', id: 'legacy-task' }, { kind: 'draft', id: 'legacy-draft' }],
        taskDrafts: { 'legacy-draft': { id: 'legacy-draft', text: 'Legacy draft text', title: 'Legacy draft', agentId: 'atlas', projectId: '', parentId: null, nativeSessionId: '', cwd: '/tmp/legacy' } },
        drafts: { 'task:legacy-task': 'Legacy unsent composer' }, selectedTaskId: 'legacy-task', currentDraftId: null,
        selectedChannelId: null, pane: 'task', focusedAgentId: null, focusedProjectId: null, showDetail: true, detailTab: 'run',
        queuedAttachments: {}, attachmentContexts: {}, channelRecipients: {},
      },
    }));
  });
  await expect(legacyPage.locator('[data-tab-kind="task"][data-tab-id="legacy-task"]')).toBeVisible();
  const migrated = await legacyPage.evaluate(() => JSON.parse(localStorage.getItem('monitter.workspaces.v2')));
  expect(migrated).toMatchObject({ version: 2, activeWorkspaceKey: 'all' });
  expect(migrated.workspaces.all.main).toMatchObject({ openTaskIds: ['legacy-task'], drafts: { 'task:legacy-task': 'Legacy unsent composer' } });
  expect(await legacyPage.evaluate(() => localStorage.getItem('monitter.workspace.v1'))).not.toBeNull();
  passed.push('migrates the legacy workspace into All activity without losing an open tab or draft');
  await legacyContext.close();

  const corruptContext = await browser.newContext({ viewport: { width: 1280, height: 820 } });
  const corruptPage = await newFixturePage(corruptContext, () => {
    localStorage.setItem('monitter.workspaces.v2', '{this is deliberately malformed');
  });
  await expect(corruptPage.getByRole('alert')).toContainText('Could not restore workspace state');
  expect(await corruptPage.evaluate(() => localStorage.getItem('monitter.workspaces.v2'))).toBe('{this is deliberately malformed');
  passed.push('a malformed v2 workspace remains untouched and reports a visible restore error');
  await corruptContext.close();

  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await newFixturePage(context, () => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    const makeTask = (id, agentId, projectId, title, updatedAt) => ({
      id, agentId, projectId, title, nativeSessionId: null, status: 'idle', archived: false,
      createdAt: updatedAt, updatedAt, parentTaskId: null, channelId: null, hostId: 'local',
      cwd: `/tmp/${id}`, provider: 'codex', model: '', sandbox: 'read-only',
    });
    state.agents = [
      { ...state.agents[0], id: 'north', name: 'North', cwd: '/tmp/north' },
      { ...state.agents[0], id: 'south', name: 'South', cwd: '/tmp/south' },
    ];
    state.projects = [
      { id: 'beacon', name: 'Beacon', description: '', icon: 'folder', color: '#397e61', workspaces: [] },
      { id: 'harbor', name: 'Harbor', description: '', icon: 'folder', color: '#397e61', workspaces: [] },
    ];
    state.tasks = [
      makeTask('overlap', 'north', 'beacon', 'Overlap chat', now + 4),
      makeTask('north-harbor', 'north', 'harbor', 'North harbor chat', now + 3),
      makeTask('south-beacon', 'south', 'beacon', 'South beacon chat', now + 2),
      makeTask('approval-chat', 'south', 'harbor', 'Approval needed', now + 1),
    ];
    state.channels = [{ id: 'global-updates', name: 'Global updates', description: 'Fixture global channel.', agentIds: ['north', 'south'], messages: [] }];
    state.approvalRequests = [{ id: 'approval-1', taskId: 'approval-chat', provider: 'claude', runId: 'run-1', tool: 'Write', summary: 'Write a QA file', detail: 'Fixture approval only.', risk: 'medium', status: 'pending', createdAt: now, resolvedAt: null, decision: null }];
    state.settings = { ...state.settings, sidebarView: 'activity' };
    qa.setSnapshot(state);
    if (!sessionStorage.getItem('scoped-workspaces-seeded')) {
      localStorage.removeItem('monitter.workspace.v1');
      localStorage.removeItem('monitter.workspaces.v2');
      sessionStorage.setItem('scoped-workspaces-seeded', 'true');
    }
  });
  const workspaceContext = page.locator('[data-workspace-context]').first();
  const expectWorkspace = key => expect(workspaceContext).toHaveAttribute('data-workspace-key', key);
  const ensureSidebarOpen = async () => {
    const standard = page.getByRole('button', { name: 'Standard view', exact: true });
    if (!await standard.isVisible()) {
      await page.getByRole('separator', { name: 'Resize main sidebar', exact: true }).press('Enter');
      await expect(standard).toBeVisible();
    }
  };
  const chooseSidebarView = async label => {
    await ensureSidebarOpen();
    const button = page.getByRole('button', { name: `${label} view`, exact: true });
    if (await button.getAttribute('aria-pressed') !== 'true') await button.click();
    await expect(button).toHaveAttribute('aria-pressed', 'true');
  };
  const chooseAgent = async (id, name) => {
    await chooseSidebarView('Standard');
    await page.getByRole('button', { name: `Open agent ${name}`, exact: true }).click();
    await expectWorkspace(`agent:${id}`);
  };
  const chooseProject = async (id, name) => {
    await chooseSidebarView('Projects');
    await page.getByRole('button', { name: `Open project ${name}`, exact: true }).click();
    await expectWorkspace(`project:${id}`);
  };
  const overlapTab = () => page.locator('[data-tab-kind="task"][data-tab-id="overlap"] > button.tab');
  const forbidden = () => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => ['createTask', 'sendMessage', 'resumeTask', 'cancelTask', 'closeTerminal'].includes(call.method)));
  const controls = async label => {
    await page.keyboard.press('Meta+p');
    const dialog = page.getByRole('dialog', { name: 'Controls', exact: true });
    await expect(dialog).toBeVisible();
    await dialog.getByText(label, { exact: true }).click();
  };
  const closeDraft = async () => {
    const close = page.getByRole('button', { name: 'Close draft', exact: true });
    await page.locator('[data-tab-kind="draft"]').hover();
    await close.click();
    await expect(close).toHaveCount(0);
  };

  await chooseAgent('north', 'North');
  await expect(page.getByRole('button', { name: 'Overview', exact: true })).toHaveCount(0);
  await expect(page.locator('.agent-group .task-row')).toHaveCount(4);
  await chooseProject('beacon', 'Beacon');
  await expect(page.locator('.project-group .task-row')).toHaveCount(4);
  passed.push('the picker is absent and the sidebar remains global across agent and project workspaces');

  // A globally visible chat can jump directly from another agent's workspace
  // to its owner, while each workspace keeps its separate tab layout.
  await chooseAgent('north', 'North');
  await page.locator('.agent-group .task-select').filter({ hasText: 'South beacon chat' }).click();
  await expectWorkspace('agent:south');
  await expect(page.getByRole('heading', { name: /South beacon chat/ })).toBeVisible();
  passed.push('a global sidebar chat switches directly to its owning agent workspace');

  // Create UI-only state in North, then make sure the other scope cannot overwrite it.
  await chooseAgent('north', 'North');
  await page.locator('.agent-group .task-select').filter({ hasText: 'Overlap chat' }).click();
  const composer = page.getByLabel('Task message', { exact: true });
  await composer.fill('North-only unsent draft');
  await controls('Two columns');
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  await controls('New terminal');
  await expect(page.locator('.terminal-tab')).toHaveCount(1);
  await chooseProject('beacon', 'Beacon');
  await page.locator('.project-group .task-select').filter({ hasText: 'Overlap chat' }).click();
  await expect(page.locator('[data-tab-kind="task"][data-tab-id="overlap"]')).toBeVisible();
  await expect(composer).toHaveValue('North-only unsent draft');
  await chooseAgent('north', 'North');
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  await expect(page.locator('.terminal-tab')).toHaveCount(1);
  await overlapTab().click();
  await expect(composer).toHaveValue('North-only unsent draft');
  expect(await forbidden()).toEqual([]);
  passed.push('switching scopes restores selection, split panes, terminal and unsent text without harness actions');

  // In Standard view an agent header is itself workspace navigation, not merely
  // a collapsible label, and restores that agent's saved tab state.
  await chooseProject('beacon', 'Beacon');
  await chooseSidebarView('Standard');
  await page.getByRole('button', { name: 'Open agent North', exact: true }).click();
  await expectWorkspace('agent:north');
  await overlapTab().click();
  await expect(composer).toHaveValue('North-only unsent draft');
  passed.push('agent header switches to and restores its independent workspace');

  // A fresh local draft is scoped before it ever creates a task or sends text.
  await chooseProject('beacon', 'Beacon');
  await controls('New chat');
  await expect(page.getByLabel('Agent', { exact: true })).toHaveValue('north');
  await expect(page.getByLabel('Project', { exact: true })).toHaveValue('beacon');
  expect(await forbidden()).toEqual([]);
  passed.push('new chat inherits the active project scope without creating a task');

  // A project-scoped draft remains in its project when only its agent changes.
  await page.getByLabel('Agent', { exact: true }).selectOption('south');
  await expectWorkspace('project:beacon');
  await expect(page.getByLabel('Agent', { exact: true })).toHaveValue('south');
  await closeDraft();

  // Changing a draft owner from an agent workspace moves the local-only tab
  // into the new owner's workspace.
  await chooseAgent('north', 'North');
  await controls('New chat');
  await page.getByLabel('Agent', { exact: true }).selectOption('south');
  await expectWorkspace('agent:south');
  await expect(page.getByLabel('Agent', { exact: true })).toHaveValue('south');
  await closeDraft();
  await chooseAgent('north', 'North');
  await overlapTab().click();
  passed.push('changing a draft agent routes its tab into that agent workspace');

  // The approval entry remains discoverable when its owning task is outside the active scope.
  await chooseProject('beacon', 'Beacon');
  const approvalJump = page.getByRole('button', { name: /Approval .*Approval needed/ });
  await expect(approvalJump).toBeVisible();
  await approvalJump.click();
  await expectWorkspace('project:harbor');
  await expect(page.getByRole('heading', { name: /Approval needed/ })).toBeVisible();
  await expect(page.getByLabel('Pending approval requests', { exact: true })).toBeVisible();
  passed.push('a pending approval outside the current scope jumps to its owning workspace and chat');

  // Channels are global rather than pinned to the project/agent filter. The
  // approval shortcut remains available after collapsing the full sidebar.
  await chooseAgent('north', 'North');
  await page.locator('.channel-row').filter({ hasText: 'Global updates' }).click();
  await expectWorkspace('all');
  await expect(page.getByRole('heading', { name: /Global updates/ })).toBeVisible();
  await chooseProject('beacon', 'Beacon');
  const sidebarResize = page.getByRole('separator', { name: 'Resize main sidebar', exact: true });
  await sidebarResize.press('Enter');
  const railApproval = page.locator('[aria-label="Pending approvals across workspaces"] .workspace-approval[data-approval-task="approval-chat"]');
  await expect(railApproval).toBeVisible();
  await railApproval.click();
  await expectWorkspace('project:harbor');
  const resizeBox = await sidebarResize.boundingBox();
  if (!resizeBox) throw new Error('Sidebar resize handle is unavailable.');
  await page.mouse.move(resizeBox.x + resizeBox.width / 2, resizeBox.y + 80);
  await page.mouse.down();
  await page.mouse.move(resizeBox.x + 180, resizeBox.y + 80);
  await page.mouse.up();
  await expect(sidebarResize).toHaveAttribute('aria-valuetext', /pixels/);
  await chooseSidebarView('Activity');
  await expectWorkspace('all');
  passed.push('global channel opens All activity and approval routing remains visible while sidebar is collapsed');

  // Backend-style reassignment removes the task tab only from its old
  // right-hand workspace, without changing unrelated task metadata.
  const original = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(item => item.id === 'overlap'));
  await page.evaluate(() => { const qa = window.__MONITTER_QA__, state = qa.snapshot(); state.tasks.find(item => item.id === 'overlap').projectId = 'harbor'; qa.setSnapshot(state); });
  await chooseProject('beacon', 'Beacon');
  await expect(page.locator('[data-tab-kind="task"][data-tab-id="overlap"]')).toHaveCount(0);
  await chooseProject('harbor', 'Harbor');
  const reassigned = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(item => item.id === 'overlap'));
  expect({ ...reassigned, projectId: original.projectId }).toEqual(original);
  passed.push('project reassignment removes only an old scoped tab and preserves task metadata');

  // Inactive scope state is durable across a browser reload, including the old North layout and draft.
  await chooseAgent('north', 'North');
  await page.reload();
  await expectWorkspace('agent:north');
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  await expect(page.locator('.terminal-tab')).toHaveCount(1);
  await overlapTab().click();
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('North-only unsent draft');
  expect(await forbidden()).toEqual([]);
  passed.push('reload restores the active scope and independently persisted inactive workspace state');

  // A scope switch must not discard a composer while its send is still pending.
  // This deliberate fixture send is the sole harness-shaped call in this guard test.
  await page.evaluate(() => {
    const bridge = window.__MONITTER_BRIDGE__, original = bridge.sendMessage;
    let release;
    window.__MONITTER_QA__.releaseScopedSend = () => release();
    bridge.sendMessage = async (...args) => {
      bridge.sendMessage = original;
      await new Promise(resolve => { release = resolve; });
      return original(...args);
    };
  });
  await chooseSidebarView('Projects');
  await page.getByLabel('Task message', { exact: true }).fill('Keep this while send is pending');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  await page.getByRole('button', { name: 'Open project Beacon', exact: true }).click();
  await expectWorkspace('agent:north');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('monitter.workspaces.v2')).activeWorkspaceKey)).toBe('agent:north');
  await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('Keep this while send is pending');
  await page.evaluate(() => window.__MONITTER_QA__.releaseScopedSend());
  passed.push('a pending send blocks scope switching and retains its composer text');

  expect(pageErrors).toEqual([]);
  await page.screenshot({ path: 'verification/ui-scoped-workspaces-happy.png', fullPage: true });
  writeFileSync(output, `${JSON.stringify({ completedAt: new Date().toISOString(), passed, pageErrors }, null, 2)}\n`);
  await context.close();
  console.log(JSON.stringify({ passed, pageErrors }, null, 2));
} catch (error) {
  writeFileSync(output, `${JSON.stringify({ completedAt: new Date().toISOString(), passed, pageErrors, error: String(error) }, null, 2)}\n`);
  throw error;
} finally {
  await browser.close();
}
