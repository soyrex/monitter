import { chromium, expect } from '@playwright/test';

// Browser-only coverage: the fixture supplies every bridge method, so this
// proves workspace routing without starting or changing a real harness.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18465';
const browser = await chromium.launch({ headless: true });

try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const qa = window.__MONITTER_QA__;
    const state = qa.snapshot();
    const now = Date.now();
    const task = (id, agentId, title, offset) => ({
      id, agentId, projectId: null, title, nativeSessionId: null,
      status: 'idle', archived: false, createdAt: now + offset, updatedAt: now + offset,
      parentTaskId: null, channelId: null, hostId: 'local', cwd: `/tmp/${id}`,
      provider: 'codex', model: '', sandbox: 'read-only',
    });
    state.agents = [
      { ...state.agents[0], id: 'north', name: 'North' },
      { ...state.agents[0], id: 'south', name: 'South' },
    ];
    state.tasks = [task('north-task', 'north', 'North task', 2), task('south-task', 'south', 'South task', 1)];
    qa.setSnapshot(state);
    localStorage.removeItem('monitter.workspace.v1');
    localStorage.removeItem('monitter.workspaces.v2');
  });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });

  const activeWorkspace = () => page.evaluate(() => {
    const stored = localStorage.getItem('monitter.workspaces.v2');
    return stored ? JSON.parse(stored).activeWorkspaceKey : null;
  });
  await expect(page.getByRole('button', { name: 'Open global overview', exact: true })).toBeVisible({ timeout: 60000 });
  await expect(page.locator('.dashboard-overview')).toBeVisible({ timeout: 60000 });
  await expect(page.locator('.workspace > .topbar')).toHaveCount(0);
  await page.getByRole('button', { name: 'Open agent North', exact: true }).click();
  await expect.poll(activeWorkspace).toBe('agent:north');
  await expect(page.locator('.workspace > .topbar')).toBeVisible();

  await page.getByRole('button', { name: 'Open global overview', exact: true }).click();
  await expect.poll(activeWorkspace).toBe('all');
  await expect(page.locator('.dashboard-overview')).toBeVisible();
  await expect(page.locator('.workspace > .topbar')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'Open terminal'})).toHaveCount(0);
  await expect(page.locator('.dashboard-overview')).toContainText('North task');
  await expect(page.locator('.dashboard-overview')).toContainText('South task');

  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('monitter.workspaces.v2')));
  expect(stored.activeWorkspaceKey).toBe('all');
  expect(stored.workspaces['agent:north']).toBeTruthy();

  const mobile = await browser.newPage({ viewport: { width: 390, height: 760 } });
  await mobile.addInitScript({ path: 'scripts/ui-fixture.js' });
  await mobile.addInitScript(() => {
    localStorage.removeItem('monitter.workspace.v1');
    localStorage.removeItem('monitter.workspaces.v2');
  });
  await mobile.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await expect(mobile.locator('.dashboard-overview')).toBeVisible({ timeout:60000 });
  await expect(mobile.locator('.workspace > .topbar.overview-nav-only')).toBeVisible();
  await expect(mobile.getByRole('button',{name:'Back to chats',exact:true})).toBeVisible();
  await expect(mobile.locator('.workspace > .topbar .tabs')).toHaveCount(0);
  await expect(mobile.getByRole('button',{name:'Open terminal'})).toHaveCount(0);
  expect(pageErrors).toEqual([]);
  console.log('Global overview owns the desktop workspace and keeps only mobile back navigation.');
} finally {
  await browser.close();
}
