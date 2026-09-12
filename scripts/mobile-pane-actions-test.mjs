import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const testUrl = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18449/';
const browser = await webkit.launch({ headless: true });

const task = (id, title, status, nativeSessionId = null) => ({
  id, agentId: 'atlas', title, status, nativeSessionId, archived: false,
  createdAt: 1, updatedAt: 2, parentTaskId: null, channelId: null,
  projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test',
  provider: 'codex', model: '', modelSettings: null, sandbox: 'read-only',
});

function fixture() {
  return {
    hosts: [{ id: 'local', name: 'This Mac', kind: 'local', address: '', user: '', port: 0,
      identityFile: '', defaultCwd: '/tmp/monitter-ui-test', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '' }],
    agents: [{ id: 'atlas', avatar: null, name: 'Atlas', description: 'Coding partner', instructions: 'Work carefully.',
      provider: 'codex', model: '', hostId: 'local', cwd: '/tmp/monitter-ui-test', color: '#397e61', sandbox: 'read-only',
      expertise: [], responsibilities: [], skills: [], collaborationEnabled: true }],
    tasks: [
      task('interrupted-task', 'Interrupted task', 'interrupted', 'native-session-1'),
      task('running-task', 'Running task', 'running'),
      task('close-me', 'Close me', 'idle'),
    ],
    messages: [], events: [], channels: [{ id: 'qa-channel', name: 'QA channel', description: 'Channel fixture', agentIds: ['atlas'], messages: [] }],
    projects: [], collaborations: [], queuedMessages: [],
    settings: { accent: '#3f9d6a', theme: 'light', interfaceScale: 100, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
  };
}

async function openPage(width) {
  const page = await browser.newPage({ viewport: { width, height: 844 }, isMobile: true, hasTouch: true });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  // Exercise the actual mobile LAN shell: desktop sharing launchers are absent
  // there, while all task operations still use the isolated test bridge.
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.route(testUrl, async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace('<html', '<html data-monitter-lan="1"') });
  });
  await page.goto(testUrl, { timeout: 90000 });
  await expect(page.locator('.app-shell').first()).toHaveClass(/mobile-navigation/, { timeout: 60000 });
  const initial = fixture();
  if (width === 390) initial.settings.theme = 'dark';
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), initial);
  await expect(page.getByRole('button', { name: 'Interrupted task', exact: true })).toBeVisible();
  return { page, errors };
}

async function editInterruptedFromPicker(page) {
  await page.locator('.tab-picker-trigger').click();
  const picker = page.locator('.tab-picker-list');
  const pickerBox = await picker.boundingBox();
  expect(pickerBox).not.toBeNull();
  expect(pickerBox.width).toBeGreaterThanOrEqual((await page.evaluate(() => innerWidth)) - 2);
  expect(pickerBox.x).toBeGreaterThanOrEqual(-1);
  expect(pickerBox.x + pickerBox.width).toBeLessThanOrEqual((await page.evaluate(() => innerWidth)) + 1);
  const row = page.locator('[data-tab-id="interrupted-task"]');
  await expect(row).toBeVisible();
  const edit = row.getByRole('button', { name: 'Edit name for Interrupted task', exact: true });
  await expect(edit).toHaveText('');
  const editBox = await edit.boundingBox();
  expect(editBox.height).toBeCloseTo(44, 1);
  expect(editBox.width).toBeCloseTo(44, 1);
  const closeBox = await row.getByRole('button', { name: 'Close tab Interrupted task', exact: true }).boundingBox();
  expect(closeBox.height).toBeCloseTo(44, 1);
  expect(closeBox.width).toBeCloseTo(44, 1);
  if (page.viewportSize().width === 390) await page.screenshot({ path: 'verification/mobile-tab-picker-icons-dark-390.png' });
  await edit.click();
}

async function mobileChecks(width) {
  const { page, errors } = await openPage(width);
  try {
    await page.getByRole('button', { name: 'Interrupted task', exact: true }).click();
    await expect(page.locator('.pane-task-header')).toBeHidden();
    await expect(page.getByRole('button', { name: 'Expand pane', exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Resume session', exact: true })).toHaveCount(0);
    const originalNative = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(t => t.id === 'interrupted-task')?.nativeSessionId);

    // Editing and sidebar toggles must not implicitly resume the interrupted session.
    await editInterruptedFromPicker(page);
    const settings = page.getByRole('dialog', { name: 'Task settings' });
    await expect(settings).toBeVisible();
    await settings.getByLabel('Task title', { exact: true }).fill('Renamed interrupted task');
    await settings.getByRole('button', { name: 'Save title', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(t => t.id === 'interrupted-task')?.title)).toBe('Renamed interrupted task');
    if (width === 390) await page.screenshot({ path: 'verification/mobile-pane-actions-dark-390.png' });

    await page.getByRole('button', { name: 'Show right sidebar', exact: true }).click();
    await expect(page.getByRole('complementary', { name: 'Right sidebar', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Hide right sidebar', exact: true }).click();
    await expect(page.getByRole('complementary', { name: 'Right sidebar', exact: true })).toBeHidden();
    await expect(page.getByRole('button', { name: 'Resume session', exact: true })).toHaveCount(0);
    expect(await page.evaluate(native => { const t = window.__MONITTER_QA__.snapshot().tasks.find(t => t.id === 'interrupted-task'); return { status: t?.status, native: t?.nativeSessionId }; }, originalNative)).toEqual({ status: 'interrupted', native: originalNative });
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c => c.method === 'resumeTask' || c.method === 'sendMessage'))).toEqual([]);

    await page.getByLabel('Task message', { exact: true }).fill('/');
    await expect(page.getByRole('button', { name: /^\/resume/ })).toHaveCount(0);
    await page.getByLabel('Task message', { exact: true }).fill('Continue this interrupted session');
    await page.getByRole('button', { name: 'Send task message', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(c => c.method === 'sendMessage').length)).toBe(1);
    expect(await page.evaluate(native => { const t = window.__MONITTER_QA__.snapshot().tasks.find(t => t.id === 'interrupted-task'); const call = window.__MONITTER_QA__.calls.find(c => c.method === 'sendMessage'); return { taskId: call?.args?.taskId ?? call?.args?.[0], native: t?.nativeSessionId }; }, originalNative)).toEqual({ taskId: 'interrupted-task', native: originalNative });
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.some(c => c.method === 'resumeTask'))).toBe(false);

    // Closing a tab is a UI-only operation and must not archive, delete, cancel, or touch task state.
    await page.getByRole('button', { name: 'Back to chats', exact: true }).click();
    await page.getByRole('button', { name: 'Close me', exact: true }).first().click();
    await page.locator('.tab-picker-trigger').click();
    await page.locator('[data-tab-id="interrupted-task"] .tab').click();
    await page.locator('.tab-picker-trigger').click();
    const closeRow = page.locator('[data-tab-id="close-me"]');
    await closeRow.getByRole('button', { name: 'Edit name for Close me', exact: true }).click();
    const closeSettings = page.getByRole('dialog', { name: 'Task settings' });
    await closeSettings.getByLabel('Task title', { exact: true }).fill('Renamed close me');
    await closeSettings.getByRole('button', { name: 'Save title', exact: true }).click();
    await page.locator('.tab-picker-trigger').click();
    const renamedCloseRow = page.locator('[data-tab-id="close-me"]');
    const before = await page.evaluate(() => window.__MONITTER_QA__.calls.length);
    await renamedCloseRow.getByRole('button', { name: 'Close tab Renamed close me', exact: true }).click();
    await expect(page.locator('[data-tab-id="close-me"]')).toHaveCount(0);
    const afterCalls = await page.evaluate(before => window.__MONITTER_QA__.calls.slice(before).map(c => c.method), before);
    expect(afterCalls.filter(method => ['setTaskArchived', 'deleteTask', 'deleteArchivedTask', 'cancelTask'].includes(method))).toEqual([]);
    expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.some(t => t.id === 'close-me' && !t.archived))).toBe(true);

    await page.getByRole('button', { name: 'Back to chats', exact: true }).click();
    await page.getByRole('button', { name: 'Running task', exact: true }).first().click();
    await expect(page.locator('.pane-task-header')).toBeHidden();
    await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
    await page.getByRole('button', { name: 'Stop current task', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(t => t.id === 'running-task')?.status)).toBe('interrupted');
    expect(errors).toEqual([]);
    console.log(`WebKit ${width}px mobile pane action checks passed.`);
  } finally { await page.close(); }
}

async function channelCheck() {
  const { page, errors } = await openPage(390);
  try {
    await page.locator('.sidebar .channel-row').filter({ hasText: 'QA channel' }).click();
    await expect(page.locator('.pane-task-header')).toHaveCount(0);
    await expect(page.locator('.conversation-head')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Expand pane', exact: true })).toHaveCount(0);
    await page.locator('.tab-picker-trigger').click();
    await page.locator('[data-tab-id="qa-channel"]').getByRole('button', { name: 'Edit name for QA channel', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Edit channel' });
    await expect(dialog).toBeVisible();
    await dialog.getByLabel('Name', { exact: true }).fill('Renamed channel');
    await dialog.getByRole('button', { name: 'Save channel', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().channels.find(c => c.id === 'qa-channel')?.name)).toBe('Renamed channel');
    expect(errors).toEqual([]);
  } finally { await page.close(); }
}

try {
  await mobileChecks(390);
  await mobileChecks(320);
  await channelCheck();
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  try {
    await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
    await page.goto(testUrl, { timeout: 90000 });
    await expect(page.locator('.app-shell').first()).not.toHaveClass(/mobile-navigation/, { timeout: 60000 });
    await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), fixture());
    await page.getByRole('button', { name: 'Running task', exact: true }).first().click();
    await expect(page.locator('.pane-task-header')).toBeVisible();
    await expect(page.locator('.pane-task-header .task-title')).toBeVisible();
    await expect(page.locator('.pane-task-header .task-title')).toContainText('Running task');
    await expect(page.locator('.pane-task-header').getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Resume session', exact: true })).toHaveCount(0);
    await expect(page.locator('.pane-task-header').getByRole('button', { name: 'Expand pane', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Stop current task', exact: true })).toBeVisible();
    console.log('WebKit desktop pane header checks passed.');
  } finally { await page.close(); }
} finally { await browser.close(); }
