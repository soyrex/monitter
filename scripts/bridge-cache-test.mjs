import { chromium, expect } from '@playwright/test';

// Requires `npm run dev -- --port 18435` (or MONITTER_TEST_URL). This is a
// browser-level contract test for the real LAN bridge, not a harness test.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18435';
const snapshot = {
  hosts: [{ id: 'local', name: 'This Mac', kind: 'local', address: '', user: '', port: 0, identityFile: '', defaultCwd: '/tmp', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '' }],
  agents: [{ id: 'atlas', name: 'Atlas', description: '', instructions: '', avatar: null, provider: 'codex', model: '', hostId: 'local', cwd: '/tmp', color: '#397e61', sandbox: 'read-only', expertise: [], responsibilities: [], skills: [], collaborationEnabled: true }],
  tasks: [{ id: 'task-1', agentId: 'atlas', title: 'Cache task', nativeSessionId: null, archived: false, status: 'idle', createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp', provider: 'codex', model: '', sandbox: 'read-only' }],
  messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [],
  settings: { accent: '#3f9d6a', theme: 'light', interfaceScale: 125, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard', busyMessageMode: 'queue' },
};
const createdTask = { ...snapshot.tasks[0], id: 'draft-task', title: 'Draft receipt task', createdAt: 2, updatedAt: 2 };
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const page = await context.newPage();
page.setDefaultTimeout(10000);
let snapshotReads = 0, fastSends = 0, legacySends = 0, legacyReads = 0, failFast = false, legacyMode = false;
let slowSnapshot = false, activeReads = 0, maxActiveReads = 0;
const eventCursors = [];
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await context.route('**/api/access', route => route.fulfill({ json: { required: false } }));
await context.route('**/api/invoke', async route => {
  const { command, args } = route.request().postDataJSON();
  if (command === 'get_model_catalog') return route.fulfill({ json: { ok: true, result: {
    models: [], current: { model: '', reasoningEffort: null, fastMode: null }, source: 'test fixture', warning: null,
  } } });
  if (command === 'get_task_goal') return route.fulfill({ json: { ok: true, result: null } });
  if (command === 'get_task_git_status') return route.fulfill({ json: { ok: true, result: { repository: false } } });
  if (command === 'get_task_events') {
    eventCursors.push(args.before ?? null);
    const label = args.before === 1 ? 'Older diagnostic' : args.before === 0 ? 'Oldest diagnostic' : 'Recent diagnostic';
    return route.fulfill({ json: { ok: true, result: {
      events: [{ id: label, taskId: args.taskId, kind: 'error', title: label, detail: 'Actionable detail', createdAt: args.before ?? 2 }],
      nextBefore: args.before === 1 ? 0 : args.before === 0 ? null : 1,
    } } });
  }
  if (command === 'get_ui_snapshot') {
    if (legacyMode) return route.fulfill({ json: { ok: false, error: "LAN command 'get_ui_snapshot' is not available." } });
    snapshotReads++;
    activeReads++;
    maxActiveReads = Math.max(maxActiveReads, activeReads);
    try {
      if (slowSnapshot) await new Promise(resolve => setTimeout(resolve, 2300));
      return await route.fulfill({ json: { ok: true, result: { revision: 'epoch:1', snapshot: snapshotReads === 1 ? snapshot : null } } });
    } finally { activeReads--; }
  }
  if (command === 'get_snapshot') { legacyReads++; return route.fulfill({ json: { ok: true, result: snapshot } }); }
  if (command === 'create_task') return route.fulfill({ json: { ok: true, result: createdTask } });
  if (command === 'send_message_fast') {
    fastSends++;
    if (failFast) return route.fulfill({ json: { ok: false, error: 'unknown command after mutation attempt' } });
    return route.fulfill({ json: { ok: true, result: { accepted: true } } });
  }
  if (command === 'send_message') { legacySends++; return route.fulfill({ json: { ok: true, result: snapshot } }); }
  return route.fulfill({ json: { ok: true, result: [] } });
});
await context.route(url + '/', async route => {
  const response = await route.fetch();
  await route.fulfill({ response, body: (await response.text()).replace('<html ', '<html data-monitter-lan="1" ').replace('<head>', '<head><meta name="monitter-lan" content="1">') });
});
try {
  await page.goto(url + '/');
  await expect(page.getByText('Cache task', { exact: true }).first()).toBeVisible({ timeout: 30000 });
  await page.getByText('Cache task', { exact: true }).first().click();
  const taskComposer = page.getByRole('textbox', { name: 'Task message', exact: true });
  await expect(taskComposer).toBeVisible();
  const initialReads = snapshotReads;
  await taskComposer.fill('first fast send');
  await page.getByRole('button', { name: 'Send task message' }).click();
  await expect.poll(() => fastSends).toBe(1);
  expect(snapshotReads).toBe(initialReads); // no per-send capability preflight
  await expect(page.getByText('first fast send', { exact: true })).toBeVisible();
  await page.waitForTimeout(320);
  expect(snapshotReads).toBe(initialReads + 1); // one scheduled refresh

  failFast = true;
  await taskComposer.fill('must not duplicate');
  await page.getByRole('button', { name: 'Send task message' }).click();
  await expect.poll(() => fastSends).toBe(2);
  await expect.poll(() => legacySends).toBe(0);
  await expect(page.getByText('Not confirmed', { exact: false })).toBeVisible();

  expect(eventCursors).toEqual([]);
  await page.getByRole('tab', { name: 'Timeline', exact: true }).click();
  await expect(page.getByText('Recent diagnostic', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Load older activity', exact: true }).click();
  await expect(page.getByText('Older diagnostic', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Load older activity', exact: true }).click();
  await expect(page.getByText('Oldest diagnostic', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Load older activity', exact: true })).toHaveCount(0);
  expect(eventCursors).toContain(1);
  expect(eventCursors).toContain(0);
  await page.getByRole('button', { name: 'Close run detail', exact: true }).click();
  await page.waitForTimeout(300);
  const closedEventReads = eventCursors.length;

  slowSnapshot = true;
  const beforeSlow = snapshotReads;
  await expect.poll(() => snapshotReads, { timeout: 7000 }).toBeGreaterThan(beforeSlow + 1);
  expect(maxActiveReads).toBe(1);
  slowSnapshot = false;
  await expect.poll(() => activeReads, { timeout: 5000 }).toBe(0);
  expect(eventCursors.length).toBe(closedEventReads);

  // create_task inserts this task locally. A tiny send receipt must never
  // replace that local snapshot with the older cached projection.
  failFast = false;
  await page.getByRole('button', { name: 'New chat', exact: true }).first().click();
  await page.getByRole('textbox', { name: 'Task message', exact: true }).fill('draft must survive receipt');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect.poll(() => fastSends).toBe(3);
  await expect(page.locator('[data-task-id="draft-task"]')).toBeVisible();
  await expect(page.getByText('draft must survive receipt', { exact: true })).toBeVisible();

  // Old installed LAN servers use this exact read-probe error. It is safe to
  // fall back before any send is attempted.
  legacyMode = true;
  const oldPage = await context.newPage();
  await oldPage.goto(url + '/');
  await expect(oldPage.getByText('Cache task', { exact: true }).first()).toBeVisible({ timeout: 30000 });
  expect(legacyReads).toBeGreaterThan(0);
  await oldPage.close();
  expect(errors).toEqual([]);
  console.log('Revision cache: no-preflight, coalesced slow reads, tiny receipts, draft preservation and safe legacy fallback pass.');
} finally { await browser.close(); }
