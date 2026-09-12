import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
  const snapshot = window.__MONITTER_QA__.snapshot();
  snapshot.tasks = [{id:'touch-chat',agentId:'atlas',title:'Touch chat',status:'completed',archived:false,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only',nativeSessionId:null,createdAt:1,updatedAt:1}];
  window.__MONITTER_QA__.setSnapshot(snapshot);
`);

try {
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18438/');
  const chat = page.locator('[data-task-id="touch-chat"] .task-select');
  await expect(chat).toBeVisible({ timeout: 60000 });
  const box = await chat.boundingBox();
  if (!box) throw new Error('Touch chat has no tappable bounds.');
  await page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.locator('.tab-picker-trigger')).toHaveText('Touch chat');
  await expect(page.locator('h1').first()).toContainText('Touch chat');
  await expect(page.locator('.pane-grid').first()).toBeVisible();
  expect(errors).toEqual([]);
  console.log('WebKit touchscreen single-tap selects a sidebar chat.');
} finally {
  await browser.close();
}
