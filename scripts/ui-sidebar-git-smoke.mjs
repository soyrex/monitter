import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Synthetic browser fixture only: no repository or SSH command is executed.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18421';
const output = 'verification/ui-sidebar-git-results.json';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const passed = [], pageErrors = [];
let page;

try {
  page = await browser.newPage({ viewport: { width: 1240, height: 760 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'Monitter menu', exact: true })).toBeVisible({ timeout: 30000 });

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.hosts.push({ id: 'git-ssh', name: 'Git SSH', kind: 'ssh', address: 'git.example.test', user: 'qa', port: 22, identityFile: '', defaultCwd: '/srv/git', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '' });
    state.agents.push({ id: 'remote-git', avatar: null, name: 'Remote Git', description: 'SSH repository agent', instructions: '', provider: 'codex', model: '', hostId: 'git-ssh', cwd: '/srv/git', color: '#8755c7', sandbox: 'read-only', expertise: [], responsibilities: [], skills: [], collaborationEnabled: true });
    state.tasks.push(
      { id: 'local-git-task', agentId: 'atlas', title: 'Local Git chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/local-git', provider: 'codex', model: '', sandbox: 'read-only' },
      { id: 'remote-git-task', agentId: 'remote-git', title: 'Remote Git chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: now + 1, updatedAt: now + 1, parentTaskId: null, channelId: null, projectId: null, hostId: 'git-ssh', cwd: '/srv/git', provider: 'codex', model: '', sandbox: 'read-only' },
    );
    const files = [
      { path: 'src/staged.rs', originalPath: null, indexStatus: 'M', worktreeStatus: '', untracked: false },
      { path: 'src/changed.rs', originalPath: null, indexStatus: '', worktreeStatus: 'M', untracked: false },
      { path: 'notes/new.txt', originalPath: null, indexStatus: '', worktreeStatus: '?', untracked: true },
    ];
    state.gitStatus = {
      'local-git-task': { repository: true, root: '/tmp/local-git', branch: 'feature/local', files, truncated: false },
      'remote-git-task': { repository: true, root: '/srv/git', branch: 'feature/remote', files, truncated: false },
    };
    state.gitDiffs = {
      'local-git-task': {
        'staged:src/staged.rs': { repository: true, path: 'src/staged.rs', scope: 'staged', text: 'diff --git a/src/staged.rs b/src/staged.rs\n+staged local change', truncated: false, binary: false },
        'unstaged:src/changed.rs': { repository: true, path: 'src/changed.rs', scope: 'unstaged', text: 'diff --git a/src/changed.rs b/src/changed.rs\n-previous\n+working tree change', truncated: false, binary: false },
        'untracked:notes/new.txt': { repository: true, path: 'notes/new.txt', scope: 'untracked', text: '+untracked local file', truncated: false, binary: false },
      },
      'remote-git-task': {
        'staged:src/staged.rs': { repository: true, path: 'src/staged.rs', scope: 'staged', text: '+staged remote change', truncated: false, binary: false },
        'unstaged:src/changed.rs': { repository: true, path: 'src/changed.rs', scope: 'unstaged', text: '+remote working tree change', truncated: false, binary: false },
        'untracked:notes/new.txt': { repository: true, path: 'notes/new.txt', scope: 'untracked', text: '+remote untracked file', truncated: false, binary: false },
      },
    };
    qa.setSnapshot(state);
  });
  const calls = method => page.evaluate(method => window.__MONITTER_QA__.calls.filter(call => call.method === method), method);
  const openTask = title => page.getByRole('button', { name: title, exact: true }).first().click();
  const showGit = async () => {
    const git = page.getByRole('button', { name: 'Git changes', exact: true });
    await git.click();
    await expect(git).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('region', { name: 'Git changes' })).toBeVisible();
  };

  // The left rail collapses independently of the right detail pane and remains keyboard-accessible.
  const mainSidebar = page.getByRole('button', { name: 'Collapse main sidebar', exact: true });
  await mainSidebar.click();
  await expect(page.getByRole('button', { name: 'Expand main sidebar', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Expand main sidebar', exact: true }).click();
  await expect(mainSidebar).toHaveAttribute('aria-pressed', 'false');
  const rightSidebar = page.getByRole('button', { name: 'Hide right sidebar', exact: true });
  await rightSidebar.click();
  await expect(page.getByRole('button', { name: 'Show right sidebar', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('button', { name: 'Show right sidebar', exact: true }).click();
  passed.push('main and right sidebar toggles are independent and expose state through aria-pressed');

  // The agent avatar rail is available only in the collapsed main-sidebar state.
  await mainSidebar.click();
  // Avatar rail opens a scoped floating chat list with its own new-chat entry.
  await page.getByRole('button', { name: 'Chats with Atlas', exact: true }).click();
  const atlasChats = page.getByRole('dialog', { name: 'Atlas chats', exact: true });
  await expect(atlasChats).toBeVisible();
  await expect(atlasChats.getByRole('button', { name: 'New chat with Atlas', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(atlasChats).toHaveCount(0);
  passed.push('agent avatar opens a scoped floating chat list with New chat');

  // The full sidebar owns the view selector.
  await page.getByRole('button', { name: 'Expand main sidebar', exact: true }).click();
  const activityView = page.getByRole('tab', { name: 'Activity view', exact: true });
  await activityView.click();
  await expect(activityView).toHaveAttribute('aria-selected', 'true');
  await expect.poll(() => page.evaluate(() => localStorage.getItem('monitter.sidebar-view.v2:web'))).toBe('activity');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'saveSettings' && call.args.sidebarView))).toEqual([]);
  passed.push('sidebar views switch locally without a backend settings write');

  // Cmd-K/P still open their palettes after the footer launchers were removed.
  await expect(page.locator('.palette-launchers')).toHaveCount(0);
  await page.keyboard.press('Meta+k');
  await expect(page.getByRole('dialog', { name: 'Switch to', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Meta+p');
  await expect(page.getByRole('dialog', { name: 'Controls', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  passed.push('Cmd-K and Cmd-P remain available without footer launchers');

  // Both local and SSH task panes ask the bridge only for their own task and show all three scopes.
  await page.getByRole('tab', { name: 'Agents view', exact: true }).click();
  for (const [title, taskId, root, stagedText] of [
    ['Local Git chat', 'local-git-task', '/tmp/local-git', 'staged local change'],
    ['Remote Git chat', 'remote-git-task', '/srv/git', 'staged remote change'],
  ]) {
    await openTask(title);
    await showGit();
    const pane = page.getByRole('region', { name: 'Git changes' });
    await expect(pane).toContainText(root);
    await expect(pane.getByRole('heading', { name: /^Staged/ })).toBeVisible();
    await expect(pane.getByRole('heading', { name: /^Changes/ })).toBeVisible();
    await expect(pane.getByRole('heading', { name: /^Untracked/ })).toBeVisible();
    await pane.getByRole('button', { name: 'src/staged.rs', exact: true }).click();
    await expect(pane).toContainText(stagedText);
    await pane.getByRole('button', { name: 'src/changed.rs', exact: true }).click();
    await expect(pane).toContainText('working tree change');
    await pane.getByRole('button', { name: 'notes/new.txt', exact: true }).click();
    await expect(pane).toContainText('untracked');
    expect((await calls('getTaskGitStatus')).some(call => call.args.taskId === taskId)).toBe(true);
    expect((await calls('getTaskGitDiff')).filter(call => call.args.taskId === taskId).map(call => call.args.scope)).toEqual(expect.arrayContaining(['staged', 'unstaged', 'untracked']));
  }
  passed.push('local and SSH tasks show staged, unstaged, and untracked diffs scoped to the active task');

  expect(pageErrors).toEqual([]);
  writeFileSync(output, `${JSON.stringify({ completedAt: new Date().toISOString(), passed, pageErrors }, null, 2)}\n`);
  console.log(JSON.stringify({ passed }));
} catch (error) {
  writeFileSync(output, `${JSON.stringify({ completedAt: new Date().toISOString(), passed, pageErrors, error: String(error) }, null, 2)}\n`);
  if (page) await page.screenshot({ path: 'verification/ui-sidebar-git-failure.png' });
  throw error;
} finally { await browser.close(); }
