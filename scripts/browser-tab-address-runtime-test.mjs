import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { chromium, webkit, expect } from '@playwright/test';

const root = process.cwd();
const iconStub = fileURLToPath(new URL('./fixtures/lucide-stub.svelte', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], root,
  resolve: { alias: [
    { find: '$lib', replacement: `${root}/src/lib` },
    { find: /^@lucide\/svelte(?:\/.*)?$/, replacement: iconStub },
  ] },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/browser-address-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated browser address entry was built');
files.set('/', `<!doctype html><div id="app"></div><script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const path = request.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated browser address server');
for (const engine of [chromium, webkit]) {
const browser = await engine.launch({ headless: true });
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  const input = page.getByRole('textbox', { name: 'Browser address' });
  await expect(input).toHaveValue('https://one.example/start');

  await input.focus();
  await page.evaluate(() => window.setNativeUrlForTest('https://one.example/focus-poll'));
  await page.waitForTimeout(100);
  await expect(input).toHaveValue('https://one.example/start');
  await page.getByRole('button', { name: 'Poll native URL' }).focus();
  await expect(input).toHaveValue('https://one.example/focus-poll');

  await input.fill('https://draft.example/path');
  await page.getByRole('button', { name: 'Poll native URL' }).click();
  await page.waitForTimeout(150);
  await expect(input).toHaveValue('https://draft.example/path');
  await page.getByRole('button', { name: 'Poll native URL' }).focus();
  await page.waitForTimeout(100);
  await expect(input).toHaveValue('https://draft.example/path');

  await input.focus();
  await input.press('Enter');
  await expect(page.getByLabel('Submitted address')).toHaveText('https://draft.example/path');
  await page.keyboard.type('?after-submit=1');
  await page.waitForTimeout(120);
  await expect(input).toHaveValue('https://draft.example/path?after-submit=1');

  await input.press('Escape');
  await expect(input).toHaveValue('https://draft.example/path');
  await expect(input).not.toBeFocused();
  await input.fill('https://discard.example/');
  await input.press('Escape');
  await page.getByRole('button', { name: 'Poll native URL' }).click();
  await expect(input).toHaveValue('https://one.example/polled');

  await input.fill('https://draft-before-switch.example/');
  await page.getByRole('button', { name: 'Switch browser tab' }).click();
  await expect(input).toHaveValue('https://two.example/current');
  assert.deepEqual(errors, [], 'browser address harness should have no page errors');
  console.log(`${engine.name()}: browser address draft survives native polling and blur; submit, continued editing, Escape, and tab switching behave correctly.`);
} finally {
  await browser.close();
}
}
await new Promise(resolve => server.close(resolve));
