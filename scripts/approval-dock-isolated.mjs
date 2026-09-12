import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

const lucideStubPath = fileURLToPath(new URL('./fixtures/approval-dock-lucide-stub.js', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], resolve: { alias: { '@lucide/svelte': lucideStubPath } }, root: process.cwd(), build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/approval-dock-entry.js' } } });
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated ApprovalDock entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => { const path = request.url?.split('?')[0] || '/'; const body = files.get(path); if (body === undefined) return response.writeHead(404).end(); response.writeHead(200, { 'content-type': path === '/' ? 'text/html' : path.endsWith('.css') ? 'text/css' : 'text/javascript' }); response.end(body); });
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address(); if (!address || typeof address === 'string') throw new Error('No isolated server address');
const browser = await webkit.launch({ headless: true });
try {
  for (const profile of [{ name: 'desktop WebKit', viewport: { width: 1120, height: 680 }, isMobile: false }, { name: 'mobile WebKit', viewport: { width: 390, height: 844 }, isMobile: true }]) {
    const context = await browser.newContext({ viewport: profile.viewport, isMobile: profile.isMobile, hasTouch: profile.isMobile });
    const page = await context.newPage(); const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await expect.poll(() => page.evaluate(() => Boolean(window.__APPROVAL_DOCK_QA__))).toBe(true);
    await expect(page.getByLabel('Pending approvals')).toBeVisible();
    await expect(page.getByText('Review expired before continuing')).toHaveCount(0);
    const dock = page.locator('.approval-dock'); const composer = page.locator('.composer');
    expect((await dock.boundingBox()).y + (await dock.boundingBox()).height).toBeLessThanOrEqual((await composer.boundingBox()).y + 1);
    await page.getByRole('button', { name: 'Next approval' }).click();
    await page.getByRole('button', { name: 'Previous approval' }).click();
    await page.getByRole('button', { name: 'Approve once' }).click();
    await page.getByRole('button', { name: 'Deny' }).click();
    await page.getByLabel('Which environment?').fill('Staging');
    await page.getByRole('button', { name: 'Submit response' }).click();
    const calls = await page.evaluate(() => window.__APPROVAL_DOCK_QA__.calls());
    expect(calls).toEqual([{ id: 'approve', value: 'approve_once' }, { id: 'deny', value: 'deny' }, { id: 'input', value: { answers: { environment: { answers: ['Staging'] } } } }]);
    expect(errors).toEqual([]);
    console.log(`${profile.name}: pending-only dock stays above composer; approve, deny, and structured input wired`);
    await context.close();
  }
  const context = await browser.newContext({ viewport: { width: 390, height: 320 }, isMobile: true, hasTouch: true });
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${address.port}/`);
  await expect(page.getByLabel('Pending approvals')).toBeVisible();
  const geometry = await page.evaluate(() => {
    const body = document.querySelector('.dock-body'); const actions = document.querySelector('.dock-actions'); const composer = document.querySelector('.composer');
    if (!body || !actions || !composer) throw new Error('Dock geometry missing');
    const actionBox = actions.getBoundingClientRect(); const composerBox = composer.getBoundingClientRect();
    return { bodyScrolls: body.scrollHeight > body.clientHeight, actionBottom: actionBox.bottom, composerBottom: composerBox.bottom, viewport: window.innerHeight };
  });
  expect(geometry.bodyScrolls).toBe(true);
  expect(geometry.actionBottom).toBeLessThanOrEqual(geometry.viewport);
  expect(geometry.composerBottom).toBeLessThanOrEqual(geometry.viewport);
  console.log('narrow mobile WebKit: scrolling request body retains response controls and composer');
  await context.close();
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
