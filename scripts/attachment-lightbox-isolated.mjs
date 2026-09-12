import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: {
    '$lib': `${process.cwd()}/src/lib`,
    '@lucide/svelte': `${process.cwd()}/scripts/fixtures/attachment-lightbox-lucide-stub.js`,
  } },
  root: process.cwd(),
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/attachment-lightbox-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No attachment lightbox entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const url = request.url?.split('?')[0] || '/';
  const body = files.get(url);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': url === '/' ? 'text/html' : url.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated server address');

const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 500, height: 420 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  const trigger = page.getByRole('button', { name: 'View full-size Screenshot.png' });
  await expect(trigger).toBeVisible();
  const preview = page.getByAltText('Preview of Screenshot.png');
  await expect.poll(() => preview.evaluate(node => node.naturalWidth)).toBe(1600);
  expect((await preview.boundingBox()).width).toBeLessThanOrEqual(300);
  await expect(page.getByAltText('Preview of Pixel.png')).toHaveCSS('width', '1px');
  await trigger.click();
  const dialog = page.getByRole('dialog', { name: 'Screenshot.png' });
  await expect(dialog).toBeVisible();
  await expect(page.getByRole('button', { name: 'View actual size' })).toBeFocused();
  await page.getByRole('button', { name: 'View actual size' }).click();
  await expect(dialog.locator('img')).toHaveClass(/actual-size/);
  const frame = dialog.locator('.image-frame');
  const scroll = await frame.evaluate(node => {
    node.scrollLeft = node.scrollWidth;
    node.scrollTop = node.scrollHeight;
    return { left: node.scrollLeft, top: node.scrollTop, maxLeft: node.scrollWidth - node.clientWidth, maxTop: node.scrollHeight - node.clientHeight };
  });
  expect(scroll.maxLeft).toBeGreaterThan(0);
  expect(scroll.maxTop).toBeGreaterThan(0);
  expect(scroll.left).toBeGreaterThanOrEqual(scroll.maxLeft - 1);
  expect(scroll.top).toBeGreaterThanOrEqual(scroll.maxTop - 1);
  await page.getByRole('button', { name: 'Fit image to window' }).click();
  await expect(dialog.locator('img')).not.toHaveClass(/actual-size/);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.locator('dialog.image-lightbox').evaluate(node => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await expect(dialog).toHaveCount(0);
  const markdownImage = page.getByRole('button', { name: 'View full-size Tool screenshot' });
  await expect(markdownImage).toBeVisible();
  expect((await markdownImage.boundingBox()).width).toBeLessThanOrEqual(300);
  await markdownImage.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Tool screenshot' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(markdownImage).toBeFocused();
  expect(errors).toEqual([]);
  console.log('WebKit: attachment image stays intrinsic, opens an accessible lightbox, and closes via Escape/backdrop with focus restore.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
