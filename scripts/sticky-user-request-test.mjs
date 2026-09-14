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
  const context = page.getByLabel('Current user request');

  await viewport.evaluate(element => { element.scrollTop = 0; element.dispatchEvent(new Event('scroll')); });
  await expect(context).toBeHidden();
  await viewport.evaluate(element => { element.scrollTop = element.scrollHeight; element.dispatchEvent(new Event('scroll')); });
  await expect(context).toBeVisible();
  await expect(context).toContainText('Please review the complete shipment-tracking workflow');

  const collapsed = await context.locator('p').evaluate(element => ({ height: element.clientHeight, scrollHeight: element.scrollHeight, clamp: getComputedStyle(element).webkitLineClamp }));
  expect(collapsed.clamp).toBe('2');
  expect(collapsed.scrollHeight).toBeGreaterThan(collapsed.height + 1);
  if (process.env.MONITTER_STICKY_SCREENSHOT) { await page.waitForTimeout(250); await page.screenshot({ path: process.env.MONITTER_STICKY_SCREENSHOT }); }

  const expand = page.getByRole('button', { name: 'Expand current request', exact: true });
  await expand.click();
  await expect(page.getByRole('button', { name: 'Collapse current request', exact: true })).toHaveAttribute('aria-expanded', 'true');
  expect(await context.locator('p').evaluate(element => element.clientHeight)).toBeGreaterThan(collapsed.height);

  await page.setViewportSize({ width: 980, height: 500 });
  await expect(context).toBeHidden();
  await page.setViewportSize({ width: 980, height: 650 });
  await expect(context).toBeVisible();
  console.log('Sticky user request hides before its anchor, floats over long output, expands, and respects the 500px viewport threshold.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
