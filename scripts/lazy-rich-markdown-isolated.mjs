import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

const result = await build({
  configFile: false,
  plugins: [svelte()],
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/lazy-rich-markdown-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
const composerChunk = result.output.find(file => file.type === 'chunk' && file.fileName.includes('RichMarkdownComposer'));
if (!entry || !composerChunk) throw new Error('Expected a separate lazy RichMarkdownComposer chunk');
if (entry.code.includes('contentType: "markdown"')) throw new Error('TipTap editor implementation leaked into the initial entry chunk');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width"><style>*{box-sizing:border-box}body{margin:0;--line:#555;--ink:#eee;--muted:#aaa;--panel:#222;--soft:#333;--accent:#398;--accent-ink:#6ca;--danger:#d55;--chat-font:system-ui;--chat-font-size:14px;--chat-line-height:1.5}</style><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const path = request.url?.split('?')[0] || '/';
  const body = files.get(path);
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated server address');
const browser = await webkit.launch({ headless: true });
const lazyChunkUrl = `**/${composerChunk.fileName}`;

try {
  {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    let requested = false;
    let releaseChunk;
    const chunkGate = new Promise(resolve => { releaseChunk = resolve; });
    await page.route(lazyChunkUrl, async route => { requested = true; await chunkGate; await route.continue(); });
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await expect(page.getByRole('button', { name: 'Mount editor' })).toBeVisible();
    expect(requested).toBe(false);

    await page.getByRole('button', { name: 'Mount editor' }).click();
    await expect(page.locator('.editor-loading')).toHaveText('Loading rich editor…');
    await expect.poll(() => requested).toBe(true);
    const fallback = page.getByRole('textbox', { name: 'Markdown message' });
    await expect(fallback).toBeFocused();
    await fallback.fill('typed before lazy editor finishes');
    await fallback.press('Enter');
    await expect(page.getByTestId('bound-value')).toContainText('typed before lazy editor finishes');
    await expect(page.getByTestId('input-calls')).toContainText('typed before lazy editor finishes');
    await expect(page.getByTestId('key-calls')).toContainText('Enter');

    releaseChunk();
    const editor = page.locator('.tiptap');
    await expect(editor).toHaveText('typed before lazy editor finishes');
    await expect(editor).toBeFocused();
    await editor.pressSequentially(' updated');
    await expect(page.getByTestId('bound-value')).toContainText('typed before lazy editor finishes updated');
    await expect(page.getByTestId('input-calls')).toContainText('typed before lazy editor finishes updated');
    expect(errors).toEqual([]);
    console.log('WebKit: editor chunk is absent until mount; deferred typing, key/input forwarding, bind propagation and focus survive lazy load.');
    await page.close();
  }

  {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route(lazyChunkUrl, route => route.fulfill({ status: 500, body: 'simulated lazy chunk failure' }));
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await page.getByRole('button', { name: 'Mount editor' }).click();
    await expect(page.getByRole('alert')).toContainText('Rich editor failed to load');
    const fallback = page.getByRole('textbox', { name: 'Markdown message' });
    await fallback.fill('still editable after failure');
    await expect(page.getByTestId('bound-value')).toContainText('still editable after failure');
    await expect(page.getByTestId('input-calls')).toContainText('still editable after failure');
    expect(errors).toEqual([]);
    console.log('WebKit: failed lazy import reports an accessible error and leaves the plain-text fallback editable.');
    await page.close();
  }
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
