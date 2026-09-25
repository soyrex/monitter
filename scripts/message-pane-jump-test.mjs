import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

const root = process.cwd();
const lucideStubPath = fileURLToPath(new URL('./fixtures/lucide-stub.js', import.meta.url));
const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: { '$lib': `${root}/src/lib`, '@lucide/svelte': lucideStubPath } },
  root,
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/message-pane-jump-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated jump-to-latest entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);

const server = createServer((request, response) => {
  const body = files.get(request.url?.split('?')[0] || '/');
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': request.url === '/' ? 'text/html' : request.url?.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated server address');

const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 620 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  const viewport = page.locator('.messages');
  await page.waitForFunction(() => {
    const node = document.querySelector('.messages');
    return !!node && node.scrollHeight > node.clientHeight;
  });
  await viewport.evaluate(node => { node.scrollTop = node.scrollHeight - node.clientHeight; node.dispatchEvent(new Event('scroll')); });
  await viewport.evaluate(node => {
    node.dispatchEvent(new WheelEvent('wheel', { deltaY: -80, bubbles: true }));
    node.scrollTop = 0;
    node.dispatchEvent(new Event('scroll'));
    window.__scrollBehaviors = [];
    const nativeScrollTo = node.scrollTo.bind(node);
    node.scrollTo = options => { window.__scrollBehaviors.push(options?.behavior ?? 'instant'); nativeScrollTo(options); };
  });
  const jump = page.getByRole('button', { name: 'Jump to latest message' });
  await expect(jump).toBeVisible();
  await jump.click();
  await expect.poll(() => page.evaluate(() => window.__scrollBehaviors.includes('smooth'))).toBe(true);
  await expect.poll(() => viewport.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop)).toBeLessThanOrEqual(3);
  expect(errors).toEqual([]);
  console.log('Jump to latest: detached reader returns to the transcript end with smooth native scrolling.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
