import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'node:path';

const root = resolve('.');
const server = await createServer({ root, configFile: false, plugins: [svelte()], resolve: { alias: [
  { find: '$lib/controller/remote-client', replacement: resolve(root, 'scripts/share-visitor-session-fixture.mjs') },
  { find: '$lib', replacement: resolve(root, 'src/lib') }
] }, appType: 'custom', server: { host: '127.0.0.1', port: 0 } });
const entry = { route: '', handle: (request, response, next) => {
  if (request.url === '/' || request.url?.startsWith('/share')) {
    response.setHeader('Content-Type', 'text/html');
    response.end('<div id="app"></div><script type="module" src="/scripts/share-visitor-ui-entry.js"></script>');
  } else next();
} };
server.middlewares.stack.unshift(entry);
await server.listen();
const address = server.httpServer.address();
const base = `http://127.0.0.1:${address.port}`;
const task = async (browserType) => {
  const browser = await browserType.launch();
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await page.addInitScript(() => { window.__shareTest = { visitorName: 'Riley', sendStarted: false, snapshotCalls: 0 }; });
  await page.goto(`${base}/share#invite=test-invite`);
  await expect(page.getByRole('heading', { name: 'Join the conversation' })).toBeVisible();
  await page.getByLabel('Your display name').fill('Riley');
  await page.getByRole('button', { name: 'Request access' }).click();
  await expect(page.getByRole('heading', { name: 'Shared design chat' })).toBeVisible();
  await expect(page.getByText('Atlas', { exact: true })).toBeVisible();
  await expect(page.getByText('Same text', { exact: true })).toBeVisible();
  await page.getByLabel('Message').fill('Same text');
  await page.evaluate(() => { window.__shareTest.releaseSend = undefined; });
  await page.getByRole('button', { name: 'Send' }).click();
  await expect.poll(() => page.evaluate(() => window.__shareTest.sendStarted)).toBe(true);
  await expect(page.locator('small', { hasText: 'Sending…' })).toBeVisible();
  await page.getByLabel('Message').fill('Follow-up draft');
  await expect(page.getByLabel('Message')).toHaveValue('Follow-up draft');
  await page.evaluate(() => window.__shareTest.releaseSend());
  await expect(page.getByText('Same text', { exact: true })).toHaveCount(2);
  await expect(page.getByText('Sent', { exact: true })).toHaveCount(0);
  await page.evaluate(() => { window.__shareTest.failNext = true; window.__shareTest.sendStarted = false; });
  await page.getByLabel('Message').fill('Same text');
  await page.getByRole('button', { name: 'Send' }).click();
  await expect.poll(() => page.evaluate(() => window.__shareTest.sendStarted)).toBe(true);
  await page.evaluate(() => window.__shareTest.releaseSend());
  await expect(page.getByText('Not confirmed', { exact: true })).toBeVisible();
  await page.waitForTimeout(2700);
  await expect(page.getByText('Not confirmed', { exact: true })).toBeVisible();
  await page.getByLabel('Message').fill('Another text');
  await page.evaluate(() => { window.__shareTest.releaseSend = undefined; });
  await page.getByRole('button', { name: 'Send' }).click();
  await expect.poll(() => page.evaluate(() => window.__shareTest.sendStarted)).toBe(true);
  await page.evaluate(() => window.__shareTest.releaseSend());
  await expect(page.getByText('Another text', { exact: true })).toBeVisible();
  await browser.close();
};
try { await task(chromium); await task(webkit); console.log('Share visitor UI passed Chromium and WebKit mobile checks.'); }
finally { await server.close(); }
