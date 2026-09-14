import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

const lucideStubPath = fileURLToPath(new URL('./fixtures/lucide-stub.js', import.meta.url));
const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: { '$lib': `${process.cwd()}/src/lib`, '@lucide/svelte': lucideStubPath } },
  root: process.cwd(),
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/sticky-user-request-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No sticky-request browser entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);

const server = createServer((request, response) => {
  const path = request.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated sticky-request server address');

const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 980, height: 800 } });
  await page.goto(`http://127.0.0.1:${address.port}/`);
  const viewport = page.locator('.messages');
  const request = page.getByLabel('Latest user request');

  await viewport.evaluate(element => { element.scrollTop = 0; element.dispatchEvent(new Event('scroll')); });
  await expect(request).toBeVisible();
  await expect(request).toHaveCount(1);
  const flowing = await request.evaluate(element => ({
    background: getComputedStyle(element).backgroundColor,
    top: element.getBoundingClientRect().top,
  }));
  await viewport.evaluate(element => { element.scrollTop = element.scrollHeight; element.dispatchEvent(new Event('scroll')); });
  await expect(request).toBeVisible();
  await expect(request).toHaveCount(1);
  await expect(request).toContainText('Please review the complete shipment-tracking workflow');
  const stuck = await request.evaluate(element => ({
    background: getComputedStyle(element).backgroundColor,
    top: element.getBoundingClientRect().top,
    fadeBottom: getComputedStyle(element, '::before').bottom,
    fadeBackground: getComputedStyle(element, '::before').backgroundImage,
  }));
  expect(stuck.background).toBe(flowing.background);
  expect(flowing.top).toBeGreaterThan(stuck.top + 1);
  expect(stuck.top).toBeGreaterThanOrEqual(8);
  expect(stuck.top).toBeLessThanOrEqual(10);
  expect(stuck.fadeBottom).toBe('-20px');
  expect(stuck.fadeBackground).toContain('linear-gradient');
  if (process.env.MONITTER_STICKY_SCREENSHOT) { await page.waitForTimeout(250); await page.screenshot({ path: process.env.MONITTER_STICKY_SCREENSHOT }); }
  console.log('The latest user bubble itself sticks in place with an identical background and a 20px page-colour fade.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
