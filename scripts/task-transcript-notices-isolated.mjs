import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
const result = await build({ configFile: false, plugins: [svelte()], resolve: { alias: { '$lib': `${process.cwd()}/src/lib` } }, build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/task-transcript-notices-entry.js' } } });
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><div id="app"></div>${result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css')).map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => { const path = request.url.split('?')[0]; const body = files.get(path); response.writeHead(body === undefined ? 404 : 200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' }); response.end(body); });
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  try { browser = await webkit.launch({ headless: true }); } catch { browser = await chromium.launch({ headless: true }); }
  for (const width of [390, 1100]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => Boolean(window.__TRANSCRIPT_NOTICES__));
    const trees = page.locator('details.process-tree');
    await expect(trees).toHaveCount(2);
    await expect(trees.nth(0).locator(':scope > summary [data-activity-state]')).toHaveAttribute('data-activity-state', 'completed');
    await expect(trees.nth(1).locator(':scope > summary [data-activity-state]')).toHaveAttribute('data-activity-state', 'running');
    await page.evaluate(() => window.__TRANSCRIPT_NOTICES__.finish());
    await expect(trees.nth(1).locator(':scope > summary [data-activity-state]')).toHaveAttribute('data-activity-state', 'completed');
    await page.evaluate(() => window.__TRANSCRIPT_NOTICES__.exchange());
    const outgoing = page.locator('[data-exchange-direction="outgoing"]');
    const incoming = page.locator('[data-exchange-direction="incoming"]');
    await expect(outgoing).toHaveCount(1); await expect(incoming).toHaveCount(1);
    await expect(outgoing.locator('.state')).toHaveText('completed');
    for (const row of [outgoing, incoming]) {
      const trigger = row.locator('.exchange-trigger');
      await expect(trigger).toHaveAttribute('aria-expanded', 'false');
      if (width <= 640) expect((await trigger.boundingBox()).height).toBeGreaterThanOrEqual(44);
      await trigger.click();
      await expect(trigger).toHaveAttribute('aria-expanded', 'true');
      await expect(row.getByLabel('Attached files')).toBeVisible();
      await expect(row.getByLabel('Exchange request')).toHaveText('Recorded request');
      expect(await row.locator('.exchange-body').evaluate(element => { const style = getComputedStyle(element); return style.getPropertyValue('user-select') || style.getPropertyValue('-webkit-user-select'); })).toBe('text');
    }
    await outgoing.getByRole('button', { name: 'Open Scoped child' }).click();
    expect(await page.evaluate(() => window.__TRANSCRIPT_NOTICES__.opened())).toBe('child');
    await expect(incoming.getByRole('button', { name: /Open/ })).toHaveCount(0);
    expect(errors).toEqual([]);
    await page.close();
  }
  console.log('Mounted TaskTranscript: real boundary events, steering, exchange direction/state, dedup attachments and scoped child navigation');
} finally { await browser?.close(); await new Promise(resolve => server.close(resolve)); }
