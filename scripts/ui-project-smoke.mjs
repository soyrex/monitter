import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Start the development server first. The injected bridge is browser-QA-only.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const output = 'verification/ui-project-results.json';
mkdirSync('verification', { recursive: true });

const browser = await chromium.launch({ headless: true });
const passed = [];
const pageErrors = [];
let page;

try {
  page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Projects', exact: true })).toBeVisible({ timeout: 30000 });
  const openDraftOptions = async () => {
    const options = page.locator('details.draft-advanced');
    await expect(options).toBeVisible();
    await options.evaluate(element => { element.open = true; });
    await expect.poll(() => options.evaluate(element => element.open)).toBe(true);
  };

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    state.hosts.push({
      id: 'qa-ssh', name: 'QA SSH', kind: 'ssh', address: 'qa.example.test', user: 'qa', port: 22,
      identityFile: '', defaultCwd: '/srv/qa-default', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '',
    });
    state.agents.push({
      id: 'remote-scout', name: 'Remote Scout', description: 'QA remote agent', instructions: 'Keep this fixture local.',
      provider: 'codex', model: '', hostId: 'qa-ssh', cwd: '/srv/remote-workspace', color: '#8755c7', sandbox: 'read-only',
    });
    state.projects = [];
    state.settings = { ...state.settings, sidebarView: 'standard' };
    qa.setSnapshot(state);
  });

  const snapshot = () => page.evaluate(() => window.__MONITTER_QA__.snapshot());
  const project = () => page.evaluate(() => window.__MONITTER_QA__.snapshot().projects[0]);
  const projectId = () => page.evaluate(() => window.__MONITTER_QA__.snapshot().projects[0]?.id);
  const taskByTitle = title => page.evaluate(title => window.__MONITTER_QA__.snapshot().tasks.find(task => task.title === title), title);

  // Project creation records an explicit workspace for each available host.
  await page.getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('button', { name: 'New project', exact: true }).click();
  let dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'New project', exact: true })).toBeVisible();
  await dialog.getByLabel('Name', { exact: true }).fill('Release QA');
  await dialog.getByLabel('Description', { exact: true }).fill('Cross-host release verification.');
  await dialog.getByLabel('Folder on This Mac', { exact: true }).fill('/tmp/release-qa');
  await dialog.getByLabel('Folder on QA SSH', { exact: true }).fill('/srv/release-qa');
  await dialog.getByRole('button', { name: 'Save project', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().projects.length)).toBe(1);
  expect(await project()).toMatchObject({
    name: 'Release QA', description: 'Cross-host release verification.',
    workspaces: expect.arrayContaining([
      expect.objectContaining({ hostId: 'local', cwd: '/tmp/release-qa' }),
      expect.objectContaining({ hostId: 'qa-ssh', cwd: '/srv/release-qa' }),
    ]),
  });
  passed.push('create project with local and SSH workspaces');

  const createProjectTask = async ({ title, agentId, expectedHost, expectedCwd }) => {
    await page.getByRole('button', { name: 'New task', exact: true }).first().click();
    await openDraftOptions();
    await page.getByLabel('Task title', { exact: true }).fill(title);
    await page.getByLabel('Agent', { exact: true }).selectOption(agentId);
    await page.getByLabel('Project', { exact: true }).selectOption(await projectId());
    await page.getByLabel('Task message', { exact: true }).fill(`Create ${title} through the draft-first composer.`);
    const createsBefore = await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length);
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'createTask').length)).toBe(createsBefore + 1);
    await expect.poll(() => taskByTitle(title)).toMatchObject({ projectId: await projectId(), hostId: expectedHost, cwd: expectedCwd });
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
  };
  await createProjectTask({ title: 'Local project chat', agentId: 'atlas', expectedHost: 'local', expectedCwd: '/tmp/release-qa' });
  await createProjectTask({ title: 'Remote project chat', agentId: 'remote-scout', expectedHost: 'qa-ssh', expectedCwd: '/srv/release-qa' });
  passed.push('project chats inherit each selected agent host and project workspace');

  // Existing chats retain their own history and working directory when linked later.
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.tasks.push({
      id: 'existing-unassigned', agentId: 'atlas', title: 'Existing unassigned chat', nativeSessionId: null,
      archived: false, status: 'idle', createdAt: now, updatedAt: now, parentTaskId: null, channelId: null,
      projectId: null, hostId: 'local', cwd: '/tmp/keep-this-cwd', provider: 'codex', model: '', sandbox: 'read-only',
    });
    state.messages.push({ id: 'existing-history', taskId: 'existing-unassigned', role: 'assistant', text: 'Preserved chat history.', createdAt: now });
    qa.setSnapshot(state);
  });
  await page.getByRole('button', { name: /Existing unassigned chat/ }).first().click();
  await page.getByRole('button', { name: 'Task actions', exact: true }).click();
  await page.getByRole('button', { name: 'Task settings', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Project', { exact: true }).selectOption(await projectId());
  await expect.poll(() => taskByTitle('Existing unassigned chat')).toMatchObject({ projectId: await projectId(), cwd: '/tmp/keep-this-cwd' });
  expect((await snapshot()).messages.find(message => message.id === 'existing-history').text).toBe('Preserved chat history.');
  await dialog.getByRole('button', { name: /close|cancel|save/i }).first().click().catch(() => {});
  passed.push('assign existing chat without changing its workspace or history');

  // Sidebar views show the same unarchived chats and do not discard the active composer draft.
  await page.getByRole('button', { name: /Local project chat/ }).first().click();
  await page.getByLabel('Task message', { exact: true }).fill('Draft survives sidebar views');
  for (const view of ['Standard', 'Activity', 'Projects']) {
    await page.getByRole('button', { name: view, exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.sidebarView)).toBe(view.toLowerCase());
    await expect(page.getByLabel('Task message', { exact: true })).toHaveValue('Draft survives sidebar views');
  }
  await page.getByRole('button', { name: 'Activity', exact: true }).click();
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    const local = state.tasks.find(task => task.title === 'Local project chat');
    const remote = state.tasks.find(task => task.title === 'Remote project chat');
    const existing = state.tasks.find(task => task.title === 'Existing unassigned chat');
    local.status = 'running'; local.updatedAt = 10;
    remote.status = 'idle'; remote.updatedAt = Date.now();
    existing.status = 'idle'; existing.updatedAt = 20;
    qa.setSnapshot(state);
  });
  await expect.poll(async () => (await page.locator('.activity-list .task-select .chat-copy > span:first-child').allTextContents())).toEqual(['Local project chat', 'Remote project chat', 'Existing unassigned chat']);
  const activityRows = await page.locator('.task-row').allTextContents();
  expect(activityRows.findIndex(text => text.includes('Local project chat'))).toBeLessThan(activityRows.findIndex(text => text.includes('Remote project chat')));
  expect(activityRows.findIndex(text => text.includes('Remote project chat'))).toBeLessThan(activityRows.findIndex(text => text.includes('Existing unassigned chat')));
  passed.push('standard/activity/projects views preserve drafts and activity puts running chats first');

  // Project navigation participates in both keyboard palettes.
  await page.keyboard.press('Meta+k');
  dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Release QA', { exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Meta+p');
  dialog = page.getByRole('dialog');
  await expect(dialog.getByText('New project', { exact: true })).toBeVisible();
  await expect(dialog.getByText(/Projects|Sidebar/i).first()).toBeVisible();
  await page.keyboard.press('Escape');
  passed.push('projects appear in Cmd-K and project/sidebar commands appear in Cmd-P');

  // Rename through the UI, then deleting the project unassigns chats without deleting their records.
  await page.getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('button', { name: 'Open project Release QA', exact: true }).click();
  await page.getByRole('button', { name: 'Edit project Release QA', exact: true }).first().click();
  dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'Edit project', exact: true })).toBeVisible();
  await dialog.getByLabel('Name', { exact: true }).fill('Release QA renamed');
  await dialog.getByRole('button', { name: 'Save project', exact: true }).click();
  await expect.poll(() => project()).toMatchObject({ name: 'Release QA renamed' });
  await expect(page.getByRole('button', { name: 'Open project Release QA renamed', exact: true })).toBeVisible();
  passed.push('rename project updates its sidebar entry');

  await page.getByRole('button', { name: 'Edit project Release QA renamed', exact: true }).first().click();
  dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Delete project', exact: true }).click();
  const confirm = page.getByRole('dialog');
  await confirm.getByRole('button', { name: /delete project|delete/i }).last().click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().projects.length)).toBe(0);
  for (const title of ['Local project chat', 'Remote project chat', 'Existing unassigned chat']) {
    expect(await taskByTitle(title)).toMatchObject({ projectId: null });
  }
  expect((await snapshot()).messages.find(message => message.id === 'existing-history').text).toBe('Preserved chat history.');
  passed.push('delete project keeps chats and history while unassigning every project chat');

  expect(pageErrors).toEqual([]);
  const report = { completedAt: new Date().toISOString(), passed, pageErrors, output };
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  const failure = { completedAt: new Date().toISOString(), passed, pageErrors, error: String(error) };
  writeFileSync(output, `${JSON.stringify(failure, null, 2)}\n`);
  if (page) await page.screenshot({ path: 'verification/ui-project-failure.png' });
  throw error;
} finally {
  await browser.close();
}
