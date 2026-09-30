import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true, executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
const page = await browser.newPage({ viewport: { width: 1240, height: 800 } });
page.on('pageerror', error => console.error('Browser error:', error.message));
const styles = locator => locator.evaluate(element => {
  const style = getComputedStyle(element);
  return { opacity: style.opacity, background: style.backgroundColor, border: style.boxShadow !== 'none' ? style.boxShadow : style.borderTopColor };
});

async function checkRow(row, focusTarget) {
  await page.mouse.move(1000, 700);
  await page.waitForTimeout(180);
  assert.equal((await styles(row)).opacity, '0.75');

  await row.hover();
  await page.waitForTimeout(180);
  const hover = await styles(row);
  assert.equal(hover.opacity, '1');
  assert.notEqual(hover.background, 'rgba(0, 0, 0, 0)');
  assert.notEqual(hover.border, 'none');

  await page.mouse.move(1000, 700);
  await page.keyboard.press('Tab');
  await focusTarget.focus();
  await expect(focusTarget).toBeFocused();
  await page.waitForTimeout(180);
  assert.deepEqual(await styles(row), hover, 'keyboard focus must match hover on the whole row');
}

try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18424/monitter-app-ui/', { waitUntil: 'domcontentloaded', timeout: 180000 });
  try {
    await page.getByRole('tab', { name: 'Agents view' }).waitFor({ timeout: 180000 });
  } catch (error) {
    console.error('Initial page:', (await page.locator('body').innerText()).slice(0, 400));
    throw error;
  }
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.projects.push({ id: 'sample-project', name: 'Sample project', icon: 'folder', color: '#3f9d6a', workspaces: [] });
    snapshot.tasks.push({ id: 'sample-chat', agentId: 'atlas', title: 'Sample chat', nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp', provider: 'codex', model: '', sandbox: 'read-only' });
    qa.setSnapshot(snapshot);
  });

  const agentRow = page.locator('.side-scroll .agent-row').first();
  await checkRow(agentRow, agentRow.locator('.agent-name'));
  const taskRow = page.locator('.side-scroll .task-row').first();
  await checkRow(taskRow, taskRow.locator('.task-select'));

  await page.getByRole('tab', { name: 'Projects view' }).click();
  const projectRow = page.locator('.side-scroll .project-row').first();
  await checkRow(projectRow, projectRow.locator('.project-name'));

  await page.getByRole('separator', { name: 'Resize main sidebar' }).focus();
  await page.keyboard.press('Enter');
  const railProject = page.locator('.project-rail .rail-project').first();
  await checkRow(railProject, railProject);
  console.log('Main sidebar list and condensed rail use 75% rest opacity and matching hover/keyboard-focus treatments.');
} finally {
  await browser.close();
}
