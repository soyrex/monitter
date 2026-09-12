import { webkit, expect } from '@playwright/test';
import { readFileSync, mkdirSync } from 'node:fs';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
await page.emulateMedia({ reducedMotion: 'reduce' });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(`
  class MockVisualViewport extends EventTarget {
    width = 390;
    height = 844;
    offsetLeft = 0;
    offsetTop = 0;
    scale = 1;
  }
  const viewport = new MockVisualViewport();
  Object.defineProperty(window, 'visualViewport', { configurable: true, value: viewport });
  window.__setKeyboardViewport = (height, offsetTop = 0) => {
    viewport.height = height;
    viewport.offsetTop = offsetTop;
    viewport.dispatchEvent(new Event('resize'));
    viewport.dispatchEvent(new Event('scroll'));
  };
`);
await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
await page.addInitScript(`
  const snapshot = window.__MONITTER_QA__.snapshot();
  snapshot.tasks = [{id:'existing-chat',agentId:'atlas',title:'Existing chat',status:'completed',archived:false,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only',nativeSessionId:null,createdAt:1,updatedAt:1}];
  window.__MONITTER_QA__.setSnapshot(snapshot);
`);

try {
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18438/');
  await page.getByRole('button', { name: 'New chat with Atlas', exact: true }).click();
  const shell = page.locator('.app-shell').first();
  const composer = page.getByRole('textbox', { name: 'Task message' });
  const send = page.getByRole('button', { name: 'Send task message' });
  await composer.fill('Keep this draft while the keyboard moves.');
  await composer.focus();
  await page.evaluate(() => window.__setKeyboardViewport(300, 0));
  await expect.poll(() => shell.evaluate(node => node.style.getPropertyValue('--mobile-viewport-height'))).toBe('300px');
  await expect.poll(() => shell.evaluate(node => node.style.getPropertyValue('--mobile-viewport-top'))).toBe('0px');
  mkdirSync('verification', { recursive: true });
  await page.screenshot({ path: 'verification/mobile-keyboard.png' });
  const keyboardBounds = await send.boundingBox();
  expect(keyboardBounds).not.toBeNull();
  expect(keyboardBounds.y + keyboardBounds.height).toBeLessThanOrEqual(300);
  await expect(composer).toHaveValue('Keep this draft while the keyboard moves.');
  await expect(composer).toBeFocused();
  await page.evaluate(() => window.__setKeyboardViewport(844, 0));
  await expect.poll(() => shell.evaluate(node => node.style.getPropertyValue('--mobile-viewport-height'))).toBe('844px');
  const restoredBounds = await send.boundingBox();
  expect(restoredBounds).not.toBeNull();
  expect(restoredBounds.y + restoredBounds.height).toBeLessThanOrEqual(844);
  await expect(composer).toHaveValue('Keep this draft while the keyboard moves.');
  await expect(composer).toBeFocused();
  await page.getByRole('button', { name: 'Back to chats' }).click();
  await expect(page.getByRole('complementary', { name: 'Agents and tasks' })).toBeVisible();
  await page.getByRole('button', { name: 'Existing chat', exact: true }).click();
  const existingComposer = page.getByRole('textbox', { name: 'Task message' });
  await existingComposer.fill('Existing chat draft stays visible too.');
  await existingComposer.focus();
  await page.evaluate(() => window.__setKeyboardViewport(300, 24));
  await expect.poll(() => shell.evaluate(node => node.style.getPropertyValue('--mobile-viewport-height'))).toBe('300px');
  await expect.poll(() => shell.evaluate(node => node.style.getPropertyValue('--mobile-viewport-top'))).toBe('24px');
  const existingSendBounds = await send.boundingBox();
  expect(existingSendBounds).not.toBeNull();
  expect(existingSendBounds.y + existingSendBounds.height).toBeLessThanOrEqual(324);
  await expect(existingComposer).toHaveValue('Existing chat draft stays visible too.');
  await expect(existingComposer).toBeFocused();
  await page.screenshot({ path: 'verification/mobile-keyboard.png' });
  expect(errors).toEqual([]);
  console.log('WebKit visualViewport keyboard resize preserves a visible composer and draft.');
} finally {
  await browser.close();
}
