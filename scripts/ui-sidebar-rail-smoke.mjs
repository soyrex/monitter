import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1240, height: 760 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433/');
  await expect(page.getByRole('tab', { name: 'Agents view' })).toBeVisible();
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.tasks.push({ id: 'rail-chat', agentId: 'atlas', title: 'Rail chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp', provider: 'codex', model: '', sandbox: 'read-only' });
    qa.setSnapshot(snapshot);
  });
  await page.getByRole('tab', { name: 'Activity view' }).click();
  const resize = page.getByRole('separator', { name: 'Resize main sidebar' });
  await resize.focus();
  await resize.press('Enter');
  await expect(page.getByRole('navigation', { name: 'Activity' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open activity chat Rail chat' })).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Activity view' })).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(page.getByRole('navigation', { name: 'Projects' })).toBeVisible();
  await page.getByRole('button', { name: 'Chats with no project' }).click();
  await expect(page.getByRole('dialog', { name: 'No project chats' })).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'No project chats' })).toContainText('Rail chat');
  console.log('Collapsed sidebar follows Activity and Projects, including keyboard view switching and project chats.');
} finally {
  await browser.close();
}
