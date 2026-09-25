import { chromium, expect } from '@playwright/test';

// This deliberately uses the browser fixture only: start the UI separately,
// then point MONITTER_TEST_URL at that server (the default is an unused QA port).
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18447';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
const errors = [];
page.on('pageerror', error => {
  // The production shell probes Tauri before the injected browser bridge wins.
  if (!error.message.includes('window.__TAURI_INTERNALS__.transformCallback')) errors.push(error.message);
});

try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 180_000 });
  await expect(page.getByRole('button', { name: 'New chat with Atlas' })).toBeVisible({ timeout: 120_000 });

  await page.evaluate(theme => {
    const qa = window.__MONITTER_QA__;
    const snapshot = qa.snapshot();
    if (theme === 'dark') snapshot.settings.theme = 'dark';
    const now = Date.now();
    const task = (id, agentId, title, parentTaskId, status) => ({
      id, agentId, title, parentTaskId, status, nativeSessionId: null,
      archived: false, createdAt: now - 5_000, updatedAt: now,
      channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test',
      provider: 'codex', model: '', sandbox: 'read-only',
    });
    snapshot.agents.push({ ...snapshot.agents[0], id: 'reviewer', name: 'Reviewer' });
    snapshot.projects = [{ id: 'fixture-project', name: 'Fixture project', description: '', icon: 'folder', color: '#397e61', workspaces: [] }];
    snapshot.tasks = [
      { ...task('parent', 'atlas', 'Parent task', null, 'running'), projectId: 'fixture-project' },
      task('delegated', 'reviewer', 'Delegated review', 'parent', 'running'),
      task('nested', 'reviewer', 'Nested direct task', 'delegated', 'completed'),
    ];
    snapshot.collaborations = [{
      id: 'delegate-record', kind: 'delegation', fromAgentId: 'atlas', fromTaskId: 'parent',
      toAgentId: 'reviewer', toTaskId: 'delegated', text: 'Review the release notes.',
      requestId: 'delegate-request', status: 'running', result: null, error: null,
      createdAt: now - 2_000, updatedAt: now - 100,
    }];
    snapshot.subagentSessions = [
      // Native subagents do not have a separately navigable Task.
      { id: 'native-check', source: 'codex', parentTaskId: 'parent', parentThreadId: 'parent-thread',
        collaborationId: null, agentPath: '/root/native_check', agentThreadId: 'native-thread',
        prompt: 'Inspect the native task.', model: 'gpt-test', reasoningEffort: 'high',
        status: 'running', result: null, error: null, createdAt: now - 1_500, updatedAt: now - 50 },
      // This must represent the delegated Task once, rather than producing a second child row.
      { id: 'delegated-session', source: 'collaboration', parentTaskId: 'parent', parentThreadId: null,
        collaborationId: 'delegate-record', agentPath: null, agentThreadId: null,
        prompt: 'Review the release notes.', model: null, reasoningEffort: null,
        status: 'running', result: null, error: null, createdAt: now - 2_000, updatedAt: now - 100 },
    ];
    snapshot.messages = [
      { id: 'parent-message', taskId: 'parent', role: 'user', text: 'Coordinate the child work.', createdAt: now - 4_000 },
      { id: 'delegated-message', taskId: 'delegated', role: 'user', text: 'Review the release notes.', createdAt: now - 2_000 },
      { id: 'nested-message', taskId: 'nested', role: 'user', text: 'Check one direct follow-up.', createdAt: now - 1_000 },
    ];
    qa.setSnapshot(snapshot);
  }, process.env.MONITTER_ACTIVITY_THEME);

  const sidebar = page.locator('.sidebar');
  const parent = sidebar.locator('.sidebar-task-branch[data-sidebar-task-id="parent"]');
  const native = sidebar.locator('[data-sidebar-child-id="native-check"][data-sidebar-parent-task-id="parent"]');
  const delegated = sidebar.locator('[data-sidebar-child-id="delegated-session"][data-sidebar-parent-task-id="parent"]');
  const nested = sidebar.locator('[data-sidebar-child-id="task:nested"][data-sidebar-parent-task-id="delegated"]');

  await expect(parent).toBeVisible();
  await expect(parent.getByText('Parent task', { exact: true })).toBeVisible();
  await expect(native).toContainText('Inspect the native task.');
  await expect(native).toContainText('Native Check');
  await expect(native).toContainText('Working');
  await expect(delegated).toContainText('Delegated review');
  await expect(delegated).toContainText('Reviewer');
  await expect(delegated).toContainText('Working');
  await expect(nested).toHaveCount(0);
  const agentCard = sidebar.locator('.agent-group').filter({ has: page.getByRole('button', { name: 'Open agent Atlas' }) }).locator('.sidebar-parent-card');
  await expect(agentCard).toBeVisible();
  await expect(agentCard.locator('.activity-avatar')).toBeVisible();
  const agentChat = parent.locator(':scope > .grouped-task');
  await expect(agentChat).toBeVisible();
  await expect(agentChat.locator('.activity-task-title')).toHaveText('Parent task');
  const agentCardGeometry = await agentCard.evaluate(element => ({
    radius: getComputedStyle(element).borderRadius,
    avatar: getComputedStyle(element.querySelector('.activity-avatar')).width,
  }));
  expect(agentCardGeometry.radius).toBe('11px');
  expect(agentCardGeometry.avatar).toBe('32px');

  // Direct visual nesting is intentionally checked as geometry as well as ownership hooks.
  const boxes = await Promise.all([
    parent.locator(':scope > .task-row .task-select'),
    native.locator('.sidebar-child-select'),
    delegated.locator('.sidebar-child-select'),
  ].map(locator => locator.boundingBox()));
  const [parentBox, nativeBox, delegatedBox] = boxes;
  expect(nativeBox.x).toBeGreaterThan(parentBox.x);
  expect(delegatedBox.x).toBeGreaterThan(parentBox.x);

  // Every task-backed child belongs beneath its parent; it must not also be a root chat.
  await expect(sidebar.locator('.sidebar-task-branch[data-sidebar-task-id="delegated"]')).toHaveCount(0);
  await expect(sidebar.locator('.sidebar-task-branch[data-sidebar-task-id="nested"]')).toHaveCount(0);

  await delegated.click();
  await expect(page.getByRole('textbox', { name: 'Task message', exact: true })).toBeVisible();
  await expect(page.locator('.pane-leaf[data-pane-id="main"]')).toContainText('Delegated review');

  await native.click();
  const main = page.locator('.pane-leaf[data-pane-id="main"]');
  await expect(main).toContainText('Parent task');
  await expect(main.getByRole('region', { name: 'Subagents', exact: true })).toBeVisible();
  await expect(main.getByRole('region', { name: 'Subagents', exact: true })).toContainText('Inspect the native task.');

  await sidebar.getByRole('tab', { name: 'Projects view' }).click();
  const projectCard = sidebar.locator('.project-group[aria-label="Project Fixture project"] .sidebar-parent-card');
  await expect(projectCard).toBeVisible();
  await expect(projectCard.locator('.project-card-mark')).toBeVisible();
  await expect(projectCard.locator('.sidebar-card-copy small')).toHaveText('1 chat');
  await expect(sidebar.locator('.project-group[aria-label="Project Fixture project"] .grouped-task .activity-task-title')).toHaveText('Parent task');
  const projectCardGeometry = await projectCard.evaluate(element => ({
    radius: getComputedStyle(element).borderRadius,
    mark: getComputedStyle(element.querySelector('.project-card-mark')).width,
  }));
  expect(projectCardGeometry.radius).toBe('11px');
  expect(projectCardGeometry.mark).toBe('32px');

  await sidebar.getByRole('tab', { name: 'Activity view' }).click();
  await expect(sidebar.getByText('Running first', { exact: true })).toBeVisible();
  const activityParent = sidebar.locator('.activity-list .sidebar-task-branch[data-sidebar-task-id="parent"] > .activity-task');
  await expect(activityParent).toBeVisible();
  await expect(activityParent.locator('.activity-avatar')).toHaveText('AT');
  await expect(activityParent.locator('.activity-status-dot.running')).toBeVisible();
  await expect(activityParent.locator('.activity-task-title')).toHaveText('Parent task');
  await expect(activityParent.locator('.chat-meta')).toContainText('Atlas');
  const subagentsPill = activityParent.locator('.activity-subagents');
  await expect(subagentsPill).toHaveText('✦2');
  await expect(subagentsPill).toHaveAccessibleName('Collapse 2 active subagents for Parent task');
  await expect(activityParent.locator('.sidebar-child-toggle')).toHaveCount(0);
  const pillStyle = await subagentsPill.evaluate(element => ({
    radius: getComputedStyle(element).borderRadius,
    background: getComputedStyle(element).backgroundImage,
  }));
  expect(pillStyle.radius).toBe('999px');
  expect(pillStyle.background).toContain('gradient');
  const activityStyle = await activityParent.evaluate(element => ({
    height: element.getBoundingClientRect().height,
    avatar: parseFloat(getComputedStyle(element.querySelector('.activity-avatar')).width),
    background: getComputedStyle(element).backgroundColor,
    metaSize: getComputedStyle(element.querySelector('.chat-meta')).fontSize,
  }));
  expect(activityStyle.height).toBeGreaterThanOrEqual(58);
  expect(activityStyle.avatar).toBe(32);
  expect(parseFloat(activityStyle.metaSize)).toBeGreaterThanOrEqual(10);
  await expect(activityParent.locator('[data-sidebar-child-id="native-check"]')).toHaveCount(0);
  const activityNative = sidebar.locator('.activity-list [data-sidebar-child-id="native-check"]');
  await expect(activityNative).toBeVisible();
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="delegated-session"]')).toBeVisible();
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="task:nested"]')).toHaveCount(0);
  await expect(activityNative.locator('.activity-child-mark')).toHaveText('NC');
  await expect(activityNative.locator('.activity-child-copy small')).toHaveText('Native Check');
  await expect(activityNative.locator('.activity-child-status')).toHaveText('Working');
  const childStyle = await activityNative.locator('.activity-child-select').evaluate(element => ({
    radius: getComputedStyle(element).borderRadius,
    markWidth: getComputedStyle(element.querySelector('.activity-child-mark')).width,
    titleSize: getComputedStyle(element.querySelector('strong')).fontSize,
  }));
  expect(childStyle.radius).toBe('8px');
  expect(childStyle.markWidth).toBe('20px');
  expect(parseFloat(childStyle.titleSize)).toBeGreaterThanOrEqual(12);
  await subagentsPill.click();
  await expect(subagentsPill).toHaveAttribute('aria-expanded', 'false');
  await expect(subagentsPill).toHaveAccessibleName('Expand 2 active subagents for Parent task');
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="native-check"]')).toBeHidden();
  await subagentsPill.press('Enter');
  await expect(subagentsPill).toHaveAttribute('aria-expanded', 'true');
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="native-check"]')).toBeVisible();
  await activityParent.locator('.task-select').click();
  await expect(activityParent).toHaveClass(/current/);
  const selectedBackground = await activityParent.evaluate(element => getComputedStyle(element).backgroundColor);
  expect(selectedBackground).not.toBe('rgba(0, 0, 0, 0)');
  if (process.env.MONITTER_ACTIVITY_SCREENSHOT) {
    await page.waitForTimeout(350); // Let the sidebar-view transition settle for visual review.
    await page.screenshot({ path: process.env.MONITTER_ACTIVITY_SCREENSHOT });
  }
  // An active descendant remains visible even after its direct ancestor finishes.
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const snapshot = qa.snapshot();
    snapshot.subagentSessions.find(session => session.id === 'delegated-session').status = 'completed';
    snapshot.tasks.find(task => task.id === 'nested').status = 'running';
    qa.setSnapshot(snapshot);
  });
  await expect(subagentsPill).toHaveText('✦2');
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="delegated-session"]')).toHaveCount(0);
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id="task:nested"]')).toBeVisible();
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const snapshot = qa.snapshot();
    snapshot.subagentSessions.find(session => session.id === 'native-check').status = 'error';
    snapshot.tasks.find(task => task.id === 'nested').status = 'completed';
    qa.setSnapshot(snapshot);
  });
  await expect(activityParent.locator('.activity-subagents')).toHaveCount(0);
  await expect(sidebar.locator('.activity-list [data-sidebar-child-id]')).toHaveCount(0);
  expect(errors).toEqual([]);
  console.log('Sidebar child-task hierarchy, routing, and Activity row styling passed.');
} finally {
  await browser.close();
}
