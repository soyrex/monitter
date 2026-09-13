import { chromium, webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464/';
const fixture = readFileSync('scripts/ui-fixture.js', 'utf8');

function seededFixture(theme) {
  return `${fixture}
    const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();
    s.settings.theme=${JSON.stringify(theme)};
    s.tasks=[{id:'blade-task',agentId:'atlas',title:'Blade test',nativeSessionId:null,status:'idle',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];
    q.setSnapshot(s);`;
}

async function openCompactBlade(page) {
  await page.locator('[data-task-id="blade-task"] .task-select').first().click();
  await page.keyboard.press('Meta+p');
  const controls = page.getByRole('dialog', { name: 'Controls' });
  await expect(controls).toBeVisible();
  await controls.getByRole('button', { name: 'Two columns', exact: true }).click();
  await expect(controls).toHaveCount(0);
  await expect(page.locator('.pane-leaf[data-pane-id="main"]')).toBeVisible();
  const detailToggle = page.locator('.pane-leaf[data-pane-id="main"] button[aria-label="Show right sidebar"]');
  await expect(detailToggle).toBeVisible();
  await detailToggle.click();
  const backdrop = page.locator('.detail-backdrop');
  await expect(backdrop).toBeVisible();
  return backdrop;
}

async function check(browserType, name, theme) {
  const browser = await browserType.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript({ content: seededFixture(theme) });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
  await expect(page.locator('.sidebar')).toBeVisible({ timeout: 120_000 });
  const backdrop = await openCompactBlade(page);
  const content = await page.locator('.pane-leaf[data-pane-id="main"] .conversation').boundingBox();
  const box = await backdrop.boundingBox();
  expect(box).not.toBeNull();
  for (const axis of ['x', 'y', 'width', 'height']) expect(Math.abs(box[axis] - content[axis])).toBeLessThan(2);
  const blade = await page.locator('.run-detail:not(.closed)').boundingBox();
  expect(Math.abs(blade.y - content.y)).toBeLessThan(2);
  const header = page.locator('.pane-leaf[data-pane-id="main"] .pane-task-header');
  const headerBox = await header.boundingBox();
  expect(headerBox.y + headerBox.height).toBeLessThanOrEqual(box.y + 1);
  expect(await header.evaluate(node => {
    const r = node.getBoundingClientRect();
    return node.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  const expectedBackdrop = theme === 'dark' ? 'rgba(0, 0, 0, 0.5)' : 'rgba(255, 255, 255, 0.5)';
  await expect.poll(() => backdrop.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(expectedBackdrop);
  await expect.poll(() => backdrop.evaluate((node) => getComputedStyle(node).backdropFilter || getComputedStyle(node).webkitBackdropFilter)).toContain('blur');
  expect(await page.evaluate(() => document.elementFromPoint(12, 300)?.classList.contains('detail-backdrop'))).toBe(false);

  const detailTab = page.locator('.run-detail .detail-tab', { hasText: 'Timeline' });
  await expect(detailTab).toBeVisible();
  const detailTabBox = await detailTab.boundingBox();
  expect(detailTabBox).not.toBeNull();
  expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest('.detail-tab')?.textContent?.trim(), {
    x: detailTabBox.x + detailTabBox.width / 2,
    y: detailTabBox.y + detailTabBox.height / 2,
  })).toBe('Timeline');
  await detailTab.click();
  await expect(page.locator('.run-detail .detail-tab[aria-selected="true"]', { hasText: 'Timeline' })).toBeVisible();

  await backdrop.click({ position: { x: 12, y: 300 } });
  await expect(page.locator('.detail-backdrop')).toHaveCount(0);
  await expect(page.locator('.pane-leaf[data-pane-id="main"] .run-detail.closed')).toHaveCount(1);
  if (errors.length) throw new Error(`${name} ${theme}: ${errors.join('\n')}`);
  await browser.close();
  console.log(`${name} ${theme}: compact blade backdrop ok`);
}

async function checkMobileTransform(browserType, name) {
  const browser = await browserType.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.addInitScript({ content: seededFixture('light') });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
  await expect(page.locator('.sidebar')).toBeVisible({ timeout: 120_000 });
  await page.locator('[data-task-id="blade-task"] .task-select').first().click();
  await expect(page.locator('.mobile-navigation.mobile-main')).toBeVisible();
  const detailToggle = page.locator('button[aria-label="Show right sidebar"]:visible');
  await expect(detailToggle).toBeVisible();
  await detailToggle.click();
  const backdrop = page.locator('.detail-backdrop');
  await expect(backdrop).toBeVisible();
  const box = await backdrop.boundingBox();
  const content = await page.locator('.conversation:visible').boundingBox();
  expect(box).not.toBeNull();
  for (const axis of ['x', 'y', 'width', 'height']) expect(Math.abs(box[axis] - content[axis])).toBeLessThan(2);
  const blade = await page.locator('.run-detail:not(.closed)').boundingBox();
  expect(Math.abs(blade.y - content.y)).toBeLessThan(2);
  expect(await page.locator('.mobile-navigation > .pane-grid').evaluate((node) => getComputedStyle(node).transform)).not.toBe('none');
  await backdrop.click({ position: { x: 12, y: 300 } });
  await expect(backdrop).toHaveCount(0);
  await browser.close();
  console.log(`${name} mobile transformed container: compact blade backdrop ok`);
}

for (const [browserType, name] of [[chromium, 'chromium'], [webkit, 'webkit']]) {
  await check(browserType, name, 'light');
  await check(browserType, name, 'dark');
  await checkMobileTransform(browserType, name);
}
