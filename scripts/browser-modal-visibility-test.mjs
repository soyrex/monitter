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
  cacheDir: '/tmp/monitter-browser-modal-visibility-cache',
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

try {
  for (const browserType of [chromium, webkit]) {
    const browser=await browserType.launch();
    try {
      const page=await browser.newPage();await page.goto(base);
      const shell=page.locator('.fixture-shell');
      await expect(shell).toHaveAttribute('data-modal-count','0');
      for(let repeat=0;repeat<2;repeat++) {
        await page.getByRole('button',{name:'Open fixture modal'}).click();
        await expect(page.getByRole('dialog',{name:'Fixture modal'})).toBeVisible();
        await expect(shell).toHaveAttribute('data-modal-count','1');
        await page.keyboard.press('Escape');
        await expect(shell).toHaveAttribute('data-modal-count','0');
      }
      console.log(`${browserType.name()}: mounted modal suspension and exit cleanup passed`);
    } finally {await browser.close();}
  }
} finally {await server.close();}
