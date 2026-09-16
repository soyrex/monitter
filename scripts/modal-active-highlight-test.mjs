import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'node:path';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';

const root = resolve('.');
const appSurfaceSource = readFileSync(resolve(root, 'src/lib/components/AppSurface.svelte'), 'utf8');
const appSurfaceStyle = appSurfaceSource.match(/<style>([\s\S]*?)<\/style>\s*$/)?.[1];
if (!appSurfaceStyle) throw new Error('Could not extract AppSurface styles for the focus-reset regression.');
const appSurfaceCss = compile(`<style>${appSurfaceStyle}</style>`, { generate: 'client', css: 'external' }).css.code;
const server = await createServer({
  root,
  configFile: false,
  cacheDir: '/tmp/monitter-modal-highlight-vite-cache',
  plugins: [svelte()],
  resolve: { alias: [{ find: '$lib', replacement: resolve(root, 'src/lib') }] },
  optimizeDeps: { entries: ['scripts/fixtures/modal-active-highlight-entry.js'] },
  appType: 'custom',
  server: { host: '127.0.0.1', port: 0 },
});
server.middlewares.stack.unshift({
  route: '',
  handle(request, response, next) {
    if (request.url === '/app-surface.css') {
      response.setHeader('Content-Type', 'text/css');
      response.end(appSurfaceCss);
    } else if (request.url === '/') {
      response.setHeader('Content-Type', 'text/html');
      response.end('<link rel="stylesheet" href="/app-surface.css"><div id="app"></div><script type="module" src="/scripts/fixtures/modal-active-highlight-entry.js"></script>');
    } else next();
  },
});
await server.listen();
const address = server.httpServer.address();
const base = `http://127.0.0.1:${address.port}`;

const pseudoContent = (page) => page.locator('.pane-leaf.active').evaluate((node) => getComputedStyle(node, '::after').content);
const outline = (locator) => locator.evaluate((node) => getComputedStyle(node).outline);
const focusedOutline = async (locator) => {
  return locator.evaluate((node) => {
    node.focus();
    const style = getComputedStyle(node);
    return { width: style.outlineWidth, style: style.outlineStyle, color: style.outlineColor };
  });
};
const accentOutline = { width: '2px', style: 'solid', color: 'rgb(0, 168, 240)' };

async function test(browserType) {
  const browser = await browserType.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  try {
    await page.goto(base);
    const pane = page.locator('.pane-leaf.active');
    await expect(pane).toBeVisible();
    await expect.poll(() => pseudoContent(page)).toBe('""');
    await expect(page.locator('.pane-grid-root')).toHaveAttribute('data-active-pane-border', 'true');

    await page.getByRole('button', { name: 'Open fixture modal' }).click();
    const modal = page.getByRole('dialog', { name: 'Fixture modal' });
    await expect(modal).toBeVisible();
    await expect(modal).toHaveAttribute('data-active-modal', '');
    await expect.poll(() => outline(modal)).toContain('2px');
    await expect.poll(() => focusedOutline(modal)).toEqual(accentOutline);
    await expect.poll(() => pseudoContent(page)).toBe('none');
    await page.waitForTimeout(220);
    await page.screenshot({ path: '/tmp/monitter-modal-highlight.png' });

    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(modal).toBeHidden();
    await expect.poll(() => pseudoContent(page)).toBe('""');

    await page.getByRole('button', { name: 'Open fixture modal' }).click();
    await expect(modal).toBeVisible();
    await modal.getByRole('button', { name: 'Open command palette' }).click();
    const palette = page.getByRole('dialog', { name: 'Fixture commands' });
    await expect(palette).toBeVisible();
    await expect(palette).toHaveAttribute('data-active-modal', '');
    await expect(modal).not.toHaveAttribute('data-active-modal', '');
    await expect.poll(() => outline(palette)).toContain('2px');
    await expect.poll(() => outline(modal)).not.toContain('2px');

    await palette.getByRole('button', { name: 'Preview fixture image' }).click();
    const lightbox = page.getByRole('dialog', { name: 'Fixture image preview' });
    await expect(lightbox).toBeVisible();
    await expect(lightbox).toHaveAttribute('data-active-modal', '');
    await expect(palette).not.toHaveAttribute('data-active-modal', '');
    await expect.poll(() => outline(lightbox)).toContain('2px');
    await expect.poll(() => focusedOutline(lightbox)).toEqual(accentOutline);
    await lightbox.getByRole('button', { name: 'Close image preview' }).click();
    await expect(lightbox).toBeHidden();
    await expect(palette).toHaveAttribute('data-active-modal', '');

    await palette.getByRole('button', { name: 'Close command palette' }).click();
    await expect(palette).toBeHidden();
    await expect(modal).toHaveAttribute('data-active-modal', '');
    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(modal).toBeHidden();

    await page.getByRole('button', { name: 'Open sharing' }).click();
    const sharing = page.getByRole('dialog', { name: 'Share workspace' });
    await expect(sharing).toBeVisible();
    await expect(sharing).toHaveAttribute('data-active-modal', '');
    await expect.poll(() => outline(sharing)).toContain('2px');
    await expect.poll(() => focusedOutline(sharing)).toEqual(accentOutline);
    await sharing.getByRole('button', { name: 'Close sharing' }).click();
    await expect(sharing).toBeHidden();
    await expect.poll(() => pseudoContent(page)).toBe('""');

    await page.getByLabel('Show active pane border').uncheck();
    await expect(page.locator('.pane-grid-root')).toHaveAttribute('data-active-pane-border', 'false');
    await expect.poll(() => pseudoContent(page)).toBe('none');
    await page.getByRole('button', { name: 'Open fixture modal' }).click();
    await expect(modal).toBeVisible();
    await expect(modal).toHaveAttribute('data-active-modal', '');
    await expect.poll(() => outline(modal)).not.toContain('2px');
    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(modal).toBeHidden();
    await page.getByRole('button', { name: 'Open sharing' }).click();
    await expect(sharing).toBeVisible();
    await expect.poll(() => outline(sharing)).not.toContain('2px');
  } finally {
    await browser.close();
  }
}

try {
  await test(chromium);
  await test(webkit);
  console.log('Modal active-highlight UI passed Chromium and WebKit.');
} finally {
  await server.close();
}
