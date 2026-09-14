import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
  await page.addInitScript({
    content: readFileSync('scripts/ui-fixture.js', 'utf8') + `
      const q = window.__MONITTER_QA__, s = q.snapshot(), now = Date.now();
      s.tasks = ['First chat', 'Split chat'].map((title, index) => ({
        id: index ? 'split-chat' : 'first-chat', agentId: 'atlas', title,
        nativeSessionId: null, status: 'completed', archived: false,
        createdAt: now + index, updatedAt: now + index, parentTaskId: null,
        channelId: null, projectId: null, hostId: 'local', cwd: '/tmp',
        provider: 'codex', model: '', sandbox: 'read-only'
      }));
      q.setSnapshot(s);
    `,
  });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433');
  const sidebarChat = title => page.locator(`.sidebar .task-select[title="${title}"]`);
  const panes = page.locator('.pane-leaf');
  const activePane = page.locator('.pane-leaf.active');

  await sidebarChat('First chat').click({ timeout: 60_000 });
  await expect(panes).toHaveCount(1);
  await expect(activePane.locator('[data-tab-id="first-chat"]')).toBeVisible();

  await sidebarChat('Split chat').click({ modifiers: ['Meta'] });
  await expect(panes).toHaveCount(2);
  await expect(activePane.locator('[data-tab-id="split-chat"]')).toBeVisible();
  await expect(page.locator('.pane-leaf[data-pane-id="main"] [data-tab-id="split-chat"]')).toHaveCount(0);

  await sidebarChat('First chat').click();
  await expect(panes).toHaveCount(2);
  await expect(activePane).toHaveAttribute('data-pane-id', 'main');

  // macOS converts a physical Control-click into a context-menu gesture, so
  // dispatch the cross-platform primary-click shape used by Windows/Linux.
  await sidebarChat('First chat').dispatchEvent('click', { ctrlKey: true, metaKey: false, button: 0 });
  await expect(panes).toHaveCount(3);
  await expect(page.locator('[data-tab-kind="task"][data-tab-id="first-chat"]')).toHaveCount(2);
  await expect(activePane.locator('[data-tab-id="first-chat"]')).toBeVisible();

  await sidebarChat('Split chat').dispatchEvent('contextmenu', { ctrlKey: true, button: 2 });
  await expect(panes).toHaveCount(4);
  await expect(page.locator('[data-tab-kind="task"][data-tab-id="split-chat"]')).toHaveCount(2);
  await expect(activePane.locator('[data-tab-id="split-chat"]')).toBeVisible();

  console.log('Normal sidebar click reuses a pane; Command-click, Control-click and macOS Control-context-click each open the chat in a fresh split pane.');
} finally {
  await browser.close();
}
