import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';

const root = process.cwd();
const iconStub = fileURLToPath(new URL('./fixtures/lucide-stub.svelte', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], root,
  resolve: { alias: [...(process.env.MONITTER_HISTORY_TRANSCRIPT_FILE ? [{ find: '$lib/components/TaskTranscript.svelte', replacement: process.env.MONITTER_HISTORY_TRANSCRIPT_FILE }] : []), { find: '$lib', replacement: `${root}/src/lib` }, { find: /^@lucide\/svelte(?:\/.*)?$/, replacement: iconStub }] },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/owner-relay-history-entry.js' } },
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
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(`http://127.0.0.1:${address.port}/?monitter-owner-relay=1&initial-transient=1`);
  await expect.poll(() => page.evaluate(() => window.__ownerQA?.calls())).toBeGreaterThan(0);
  await page.waitForTimeout(100);
  assert.equal(await page.getByText('Reconnecting to your desktop. Try again once connected.', { exact: true }).count(), 0, 'initial transport interruption must not become a manual history error');
  assert.equal(await page.getByRole('button', { name: 'Retry loading messages' }).count(), 0);
  await expect.poll(() => page.evaluate(() => window.__ownerQA?.messages().length)).toBe(64);
  await expect(page.getByText('relay-message-100', { exact: true })).toBeVisible();
  // A loaded tail survives an offline interval; no history RPC is attempted
  // until connection restoration, which refreshes it without a button press.
  await page.evaluate(() => window.__ownerQA.disconnect());
  const offlineCalls = await page.evaluate(() => window.__ownerQA.calls());
  await page.waitForTimeout(2200);
  assert.equal(await page.evaluate(() => window.__ownerQA.calls()), offlineCalls, 'known-offline polling must pause');
  await expect(page.getByText('relay-message-100', { exact: true })).toBeVisible();
  await page.evaluate(() => { window.__ownerQA.transient(); window.__ownerQA.reconnect(); });
  await page.waitForTimeout(100);
  assert.equal(await page.getByRole('button', { name: 'Retry loading messages' }).count(), 0, 'loaded tail transport interruption must remain quiet');
  await expect(page.getByText('relay-message-100', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__ownerQA.calls()), { timeout: 6000 }).toBeGreaterThan(offlineCalls + 1);
  await page.locator('.messages').evaluate(node => { node.scrollTop = 0; node.dispatchEvent(new Event('scroll')); });
  // This fixture exercises transcript data flow without the enclosing workspace
  // layout. Dispatch the pager action independently of its scroll geometry.
  await page.evaluate(() => window.__ownerQA.delay());
  await page.getByRole('button', { name: 'Load earlier messages' }).dispatchEvent('click');
  await expect.poll(() => page.evaluate(() => window.__ownerQA.requests().some(request => request.before === 'row-37'))).toBe(true);
  await page.evaluate(() => { window.__ownerQA.disconnect(); window.__ownerQA.transient(); window.__ownerQA.release(); });
  await page.waitForTimeout(100);
  assert.equal(await page.getByText('Reconnecting to your desktop. Try again once connected.', { exact: true }).count(), 0, 'earlier-page transport interruption must remain quiet');
  assert.equal(await page.evaluate(() => window.__ownerQA.messages().length), 64, 'a failed earlier page must preserve the loaded tail');
  await page.evaluate(() => window.__ownerQA.reconnect());
  await expect.poll(() => page.evaluate(() => window.__ownerQA.messages().length)).toBe(100);
  assert.ok((await page.evaluate(() => window.__ownerQA.requests())).some(request => request.before === 'row-37'));
  await page.evaluate(() => window.__ownerQA.switch('empty'));
  await expect(page.getByText('No messages yet')).toBeVisible();
  const calls = await page.evaluate(() => window.__ownerQA.calls());
  await page.waitForTimeout(400);
  assert.equal(await page.evaluate(() => window.__ownerQA.calls()), calls, 'empty chat must not enter a tight request loop');
  await page.evaluate(() => { window.__ownerQA.fail(); window.__ownerQA.switch('failure'); });
  await expect(page.getByText('fixture page failed')).toBeVisible();
  assert.equal(await page.getByText('fixture page failed').evaluate(node => !!node.closest('.messages')), false, 'history errors must remain visible when the transcript is scrolled to the latest activity');
  await page.getByRole('button', { name: 'Retry loading messages' }).click();
  await expect(page.getByText('No messages yet')).toBeVisible();
  await page.evaluate(() => { window.__ownerQA.delay(); window.__ownerQA.switch('chat'); });
  await expect(page.getByText('Loading messages…')).toBeVisible();
  await page.evaluate(() => window.__ownerQA.switch('empty'));
  await expect(page.getByText('No messages yet')).toBeVisible();
  await page.evaluate(() => window.__ownerQA.release());
  await expect(page.getByText('No messages yet')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Load earlier messages' })).toHaveCount(0);
  assert.deepEqual(errors, []);
  console.log('Owner relay initial/tail/earlier reconnect recovery, real errors, loading and chat switching passed.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
