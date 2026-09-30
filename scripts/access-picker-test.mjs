import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { chromium, expect } from '@playwright/test';

const root = process.cwd();
const iconStub = fileURLToPath(new URL('./fixtures/lucide-stub.svelte', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], root,
  resolve: { alias: [
    { find: '$lib', replacement: `${root}/src/lib` },
    { find: /^@lucide\/svelte(?:\/.*)?$/, replacement: iconStub },
  ] },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/access-picker-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated AccessPicker entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const path = request.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated AccessPicker server address');
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  const trigger = page.getByRole('button', { name: 'Permissions: Read only', exact: true });
  await expect(trigger).toBeVisible();

  await trigger.click();
  let menu = page.getByRole('dialog', { name: 'Permissions', exact: true });
  await menu.getByRole('radio', { name: /Auto-approve edits/ }).click();
  await expect(menu).toHaveCount(0);
  await expect(trigger).toBeFocused();
  assert.deepEqual(await page.evaluate(() => window.__ACCESS_PICKER_QA__.calls), ['workspace-write']);
  await page.evaluate(() => window.__ACCESS_PICKER_QA__.resolve());
  await expect(page.getByRole('button', { name: 'Permissions: Auto-approve edits', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Permissions: Auto-approve edits', exact: true }).click();
  menu = page.getByRole('dialog', { name: 'Permissions', exact: true });
  await menu.getByRole('radio', { name: 'Full access' }).click();
  await expect(menu).toHaveCount(0);
  const otherControl = page.getByRole('button', { name: 'Other control', exact: true });
  await otherControl.focus();
  await page.evaluate(() => window.__ACCESS_PICKER_QA__.reject());
  await expect(page.getByRole('alert')).toHaveText('Settings save failed');
  await expect(page.getByRole('dialog', { name: 'Permissions', exact: true })).toHaveCount(0);
  await expect(otherControl).toBeFocused();
  expect(errors).toEqual([]);
  console.log('AccessPicker closes while permission saves are pending; rejected saves remain visible without reopening or stealing focus.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
