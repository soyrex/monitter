import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { chromium, expect } from '@playwright/test';

const root = process.cwd();
const [messagePane, thinkingStatus, virtualList, terminalRuntime, appSurface] = await Promise.all([
  readFile(`${root}/src/lib/components/MessagePane.svelte`, 'utf8'),
  readFile(`${root}/src/lib/components/ThinkingStatus.svelte`, 'utf8'),
  readFile(`${root}/src/lib/components/TranscriptVirtualList.svelte`, 'utf8'),
  readFile(`${root}/src/lib/terminal-runtime.ts`, 'utf8'),
  readFile(`${root}/src/lib/components/AppSurface.svelte`, 'utf8'),
]);

assert.doesNotMatch(messagePane, /setInterval\s*\(/, 'MessagePane must not retain a layout-repair poll');
assert.doesNotMatch(thinkingStatus, /setInterval\s*\(/, 'thinking timers must share the activity clock');
assert.match(messagePane, /active = true/);
assert.match(messagePane, /visibilitychange/);
assert.match(virtualList, /generics="T"/);
assert.match(virtualList, /estimateHeight/);
assert.match(virtualList, /stickyKey/);
assert.match(virtualList, /const renderedRows = \$derived\(rows\.filter\(row =>[\s\S]{0,180}row\.key === getKey\(items\[row\.index\], row\.index\)/,
  'stale virtualizer indexes and identities must be excluded before a row is rendered');
assert.match(virtualList, /item === undefined \? `stale-row:\$\{index\}` : getKey\(item, index\)/,
  'stale virtualizer indexes must never reach caller key generation');
assert.match(virtualList, /\{#each renderedRows as row, index \(row\.key\)\}/,
  'the row snippet must consume only bounds-checked virtual rows');
assert.match(virtualList, /let transcriptAnchorKey: string \| null = null/,
  'the mounted virtualizer must track the current transcript identity');
assert.match(virtualList, /!items\.some\(\(item, index\) => getKey\(item, index\) === transcriptAnchorKey\)/,
  'a replacement transcript must be distinguished from a normal append or prepend');
assert.match(virtualList, /if \(transcriptChanged\) instance\(\)\.measure\(\)/,
  'a replacement transcript must clear cached measurements before rendering its range');
assert.match(virtualList, /role="feed"/);
assert.match(virtualList, /ResizeObserver/);
assert.match(appSurface, /items=\{displayedChannelTranscript\.messages\}[\s\S]{0,180}active=\{embedded \? active : activePaneId === 'main'\}/,
  'visible channel panes must render transcript rows even when another split has focus');
const taskTranscript = await readFile(`${root}/src/lib/components/TaskTranscript.svelte`, 'utf8');
assert.match(taskTranscript, /<TranscriptVirtualList[\s\S]{0,260}\{active\}/,
  'visible task panes must render transcript rows even when another split has focus');
assert.doesNotMatch(terminalRuntime, /setInterval\s*\(/, 'terminal runtime must not keep a global interval');
assert.match(terminalRuntime, /runtimeCanPoll/);
assert.match(terminalRuntime, /documentVisibilityChanged/);
assert.match(terminalRuntime, /}, 5_000\)/, 'active title refresh should follow the backend five-second settle window');
assert.match(thinkingStatus, /label = labels\[0\];\s*labelBucket = 0;/,
  'new status must keep the initial Thinking label');
assert.match(thinkingStatus, /nextBucket > labelBucket/,
  'label rotation must wait for a later five-second bucket');
assert.match(appSurface, /<TranscriptVirtualList\s+items=\{displayedChannelTranscript\.messages\}/,
  'channel histories must use the same bounded transcript renderer');

function startVite() {
  const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--mode', 'monitter-app-ui', '--host', '127.0.0.1', '--port', '18433'], {
    cwd: root, env: { ...process.env, NO_COLOR: '1', MONITTER_APP_UI: '1' }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const ready = new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output.slice(-1000)}`)), 60_000);
    const receive = chunk => {
      output += chunk.toString();
      const match = output.match(/Local:\s+(http:\/\/[^\s]+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', receive);
    child.stderr.on('data', receive);
    child.once('error', reason => { clearTimeout(timer); reject(reason); });
    child.once('exit', code => { clearTimeout(timer); reject(Error(`Vite exited ${code}: ${output.slice(-1000)}`)); });
  });
  return { child, ready };
}

let vite;
let browser;
try {
  vite = startVite();
  const origin = await vite.ready;
  const url = new URL('/monitter-app-ui/', origin).href;
  browser = await chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const qa = window.__MONITTER_QA__;
    const state = qa.snapshot();
    const now = Date.now();
    state.agents.push({
      id: 'minimax-live', avatar: null, name: 'MiniMax live', description: 'Live MiniMax fixture', instructions: '',
      provider: 'acp', model: '', hostId: 'local', cwd: '/tmp/monitter-ui-test', color: '#a13c72', sandbox: 'read-only',
      expertise: [], responsibilities: [], skills: [], collaborationEnabled: true,
    });
    state.tasks = [
      {
        id: 'minimax-chat', agentId: 'minimax-live', title: 'MiniMax running chat', nativeSessionId: null, status: 'running', archived: false,
        createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null,
        hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'acp', model: '', sandbox: 'read-only',
      },
      {
        id: 'codex-chat', agentId: 'atlas', title: 'Codex running chat', nativeSessionId: null, status: 'running', archived: false,
        createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null,
        hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
      },
    ];
    state.messages = [
      ...Array.from({ length: 48 }, (_, index) => ({
        id: `minimax-history-${index}`, taskId: 'minimax-chat', role: index % 2 ? 'assistant' : 'user',
        text: `MiniMax history ${index}: ${'virtualized transcript row '.repeat(14)}`, createdAt: now - 10_000 + index,
      })),
      { id: 'minimax-last-user', taskId: 'minimax-chat', role: 'user', text: 'Keep thinking about this.', createdAt: now - 100 },
      ...Array.from({ length: 48 }, (_, index) => ({
        id: `codex-history-${index}`, taskId: 'codex-chat', role: index % 2 ? 'assistant' : 'user',
        text: `Codex history ${index}: ${'a distinct virtualized transcript row '.repeat(14)}`, createdAt: now - 9_000 + index,
      })),
      { id: 'codex-final-answer', taskId: 'codex-chat', role: 'assistant', text: 'Codex final answer is already visible.', createdAt: now },
    ];
    state.events = [{ id: 'minimax-live-thinking', taskId: 'minimax-chat', kind: 'reasoning', title: 'MiniMax thinking', detail: '', createdAt: now }];
    qa.setSnapshot(state);
  });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 90_000 });
  await page.waitForTimeout(1_000);
  await expect(page.getByRole('button', { name: 'MiniMax running chat', exact: true })).toBeVisible({ timeout: 90_000 });

  const main = page.locator('.pane-leaf[data-pane-id="main"]');
  await page.locator('.sidebar .task-select').filter({ hasText: 'MiniMax running chat' }).click();
  const minimaxThinking = main.locator('.reasoning-pending');
  await expect(minimaxThinking).toBeVisible();
  await expect(minimaxThinking.locator('.message-avatar[title="MiniMax live"]')).toBeVisible();

  await page.locator('.sidebar .task-select').filter({ hasText: 'Codex running chat' }).click();
  await expect(main.locator('.task-title')).toContainText('Codex running chat');
  await expect(main.getByText('Codex final answer is already visible.', { exact: true })).toBeVisible();
  await expect(main.locator('.reasoning-pending')).toHaveCount(0);
  await expect(main.locator('.message-avatar[title="MiniMax live"]')).toHaveCount(0);
  assert.deepEqual(pageErrors, [], `browser errors: ${pageErrors.join(' | ')}`);
} finally {
  await browser?.close();
  vite?.child.kill('SIGTERM');
}

console.log('transcript performance regression: stale virtual keys are rejected, transcript switches reset measurements, and MiniMax Thinking/avatar never survive into a running Codex chat.');
