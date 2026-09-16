import { chromium, expect } from '@playwright/test';
import { createServer } from 'vite';
import { realpathSync } from 'node:fs';

const server = await createServer({
  server: {
    host: '127.0.0.1',
    port: 0,
    strictPort: false,
    // The parity worktree's node_modules is a symlink. Vite resolves the
    // imported font files to the target checkout, so explicitly allow the
    // real path rather than producing noisy 403s during cold compilation.
    fs: { allow: [process.cwd(), realpathSync('node_modules')] },
  },
});
await server.listen();
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.setDefaultTimeout(120000);
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    let callbackId = 0;
    window.__TAURI_INTERNALS__ = {
      transformCallback: () => ++callbackId,
      invoke: async () => 0,
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    const snapshot = window.__MONITTER_QA__.snapshot();
    snapshot.settings.userName = 'Alex';
    snapshot.settings.interfaceScale = 100;
    snapshot.settings.tintUserMessages = false;
    snapshot.projects = [{ id: 'shared-project', name: 'Shared project', description: '', icon: 'folder', color: '#397e61', workspaces: [] }];
    snapshot.tasks = [{ id: 'shared-owner-chat', agentId: 'atlas', title: 'Shared owner chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: 'shared-project', hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' }];
    snapshot.messages = [
      { id: 'owner-before-sharing', taskId: 'shared-owner-chat', role: 'user', text: 'Earlier owner message', createdAt: 1 },
      { id: 'visitor-message', taskId: 'shared-owner-chat', role: 'user', text: '@(Riley): Visitor message', createdAt: 2 },
    ];
    window.__MONITTER_QA__.setSnapshot(snapshot);
  });
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}`);
  await page.waitForFunction(() => document.body.innerText.length > 30);
  await page.getByRole('tab', { name: 'Projects view', exact: true }).click();
  await page.getByRole('button', { name: 'Open project Shared project', exact: true }).click();
  await page.locator('.overview-task').filter({ hasText: 'Shared owner chat' }).click();
  await page.evaluate(async () => {
    const { activeOperatorShare } = await import('/src/lib/operator-sharing.ts');
    activeOperatorShare.set({ primary: { name: 'Alex', role: 'primary user' }, visitor: { name: 'Riley', role: 'visitor' }, taskIds: [], projectIds: ['shared-project'] });
  });
  const owner = page.locator('article.message[data-participant="Alex"]').filter({ hasText: 'Earlier owner message' });
  const visitor = page.locator('article.message[data-participant="Riley"]');
  await expect(owner).toBeVisible();
  await expect(visitor).toBeVisible();
  await expect(owner.locator('.message-author')).toHaveText('Alex');
  const ownerTint = await owner.evaluate(node => getComputedStyle(node).backgroundColor);
  const visitorTint = await visitor.evaluate(node => getComputedStyle(node).backgroundColor);
  expect(ownerTint).not.toBe(visitorTint);
  await page.getByLabel('Task message', { exact: true }).fill('Owner follow-up');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'sendMessage').length)).toBe(1);
  const sent = await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'sendMessage').args.text);
  expect(sent).toContain('Alex (primary user), Riley (visitor)');
  expect(sent).toContain('@(Alex): Owner follow-up');
  console.log('Shared owner UI: project-only prompt attribution and distinct participant tints passed.');
} finally {
  await browser.close();
  await server.close();
}
