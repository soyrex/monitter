import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: { '$lib': new URL('../src/lib', import.meta.url).pathname } },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/command-palette-jev-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
const css = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width"><style>*{box-sizing:border-box}body{margin:0;--line:#555;--ink:#eee;--muted:#aaa;--panel:#222;--soft:#333;--accent:#398;--accent-ink:#6ca;--mono:monospace}button{border:0;background:transparent;color:inherit}</style><div id="app"></div>${css.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((req, res) => {
  const path = req.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return res.writeHead(404).end();
  res.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' });
  res.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));

const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const input = page.getByRole('textbox');
  await input.fill('make everything a little less huge');
  const interpret = page.getByRole('button', { name: /^Interpret with Jev/ });
  await expect(interpret).toBeVisible();
  await interpret.click();
  expect(await page.evaluate(() => window.__PALETTE_JEV_QA__.interpreted)).toBe('make everything a little less huge');
  expect(await page.evaluate(() => window.__PALETTE_JEV_QA__.executions)).toBe(0);
  await expect(page.getByRole('button', { name: /^Decrease interface scale/ })).toBeVisible();
  await expect(page.getByText('Jev · 91% · Matches the request to make text smaller')).toBeVisible();
  expect(await page.evaluate(() => window.__PALETTE_JEV_QA__.executions)).toBe(0);
  await page.getByRole('button', { name: /^Decrease interface scale/ }).click();
  expect(await page.evaluate(() => window.__PALETTE_JEV_QA__.selected)).toBe('__jev-suggestion:scale-down');
  expect(await page.evaluate(() => window.__PALETTE_JEV_QA__.executions)).toBe(1);
  expect(errors).toEqual([]);
  console.log('WebKit: Jev interpretation proposes a control without executing it, then explicit selection confirms it.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
