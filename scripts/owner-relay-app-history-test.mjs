import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';

const root = process.cwd();
const iconStub = fileURLToPath(new URL('./fixtures/lucide-stub.svelte', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], root,
  resolve: { alias: [{ find: '$lib', replacement: `${root}/src/lib` }, { find: /^@lucide\/svelte(?:\/.*)?$/, replacement: iconStub }] },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/owner-relay-app-history-entry.ts' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No owner relay fixture entry');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const path = request.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' }).end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No fixture server address');
const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 720 } });
  const errors = [];
  page.on('pageerror', error => {
    // WebKit reports its deferred-layout notification when a whole virtualized
    // page disappears. Keep application exceptions as failures.
    if (String(error) !== 'ResizeObserver loop completed with undelivered notifications.') errors.push(String(error));
  });
  await page.goto(`http://127.0.0.1:${address.port}/?monitter-owner-relay=1`);
  await expect.poll(() => page.evaluate(() => window.__appHistory?.messages().length)).toBe(64);
  await expect(page.getByText('app-history-100', { exact: true })).toBeVisible();
  await expect(page.getByText('Transcript history changed while loading.', { exact: false })).toHaveCount(0);
  await page.evaluate(() => window.__appHistory.refresh());
  assert.equal(await page.evaluate(() => window.__appHistory.messages().length), 64, 'metadata refresh retains hydrated history');
  const tailPage = await page.evaluate(async () => {
    window.__historyServer.mode('edit-remove');
    return window.__appHistory.page();
  });
  assert.equal(tailPage.messages.length, 63);
  assert.equal(tailPage.nextBeforeId, tailPage.messages[0].id, 'a removed cursor advances to the first surviving row');
  const editedPage = await page.evaluate(async () => {
    window.__historyServer.mode('edit-remove');
    return window.__appHistory.page(window.__historyServer.messageId(38));
  });
  assert.equal(editedPage.messages.length, 35);
  assert.equal(editedPage.messages[0].text, 'app-history-edited-after-page');
  assert.equal(editedPage.nextBeforeId, null);
  assert.equal(await page.evaluate(() => window.__appHistory.messages().some(row => row.id === window.__historyServer.messageId(1))), false, 'following delta must not resurrect a deleted page row');
  const survivingPage = await page.evaluate(async () => {
    window.__historyServer.mode('remove-page');
    return window.__appHistory.page();
  });
  assert.ok(survivingPage.messages.length > 0, 'deleting a whole page must not hide surviving older messages');
  assert.equal(survivingPage.messages.length, 34);
  assert.equal(survivingPage.messages.at(-1).id, await page.evaluate(() => window.__historyServer.messageId(35)));
  assert.equal(survivingPage.nextBeforeId, null);
  await expect(page.getByText('app-history-35', { exact: true })).toBeVisible();
  const resetPage = await page.evaluate(async () => {
    window.__historyServer.mode('reset');
    return window.__appHistory.page();
  });
  assert.ok(resetPage.revision.startsWith('epoch-b:'));
  assert.equal(resetPage.messages.at(-1).text, 'app-history-after-reset');
  assert.ok(await page.evaluate(() => window.__historyServer.counts().resetCount) >= 2, 'epoch reset requires a fresh page, not a stale merge');
  const wrongTask = await page.evaluate(async () => {
    window.__historyServer.mode('wrong-task');
    try { await window.__appHistory.page(); return ''; } catch (error) { return String(error); }
  });
  assert.match(wrongTask, /different task/);
  assert.equal(await page.evaluate(() => window.__appHistory.messages().some(row => row.taskId !== window.__historyServer.taskId)), false);
  assert.deepEqual(errors, []);
  console.log('Real bridge + AppSurface history hydration passed under continuously advancing revisions, edits, removals, epoch reset, and scope rejection.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
