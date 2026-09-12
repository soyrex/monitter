import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

// Render the real component and icons, including its real keyboard handlers.
const result = await build({ configFile: false, plugins: [svelte()],
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/command-palette-entry.js' } },
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
  for (const width of [390, 1280]) {
    const page = await browser.newPage({ viewport: { width, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await expect(page.locator('.backdrop')).toHaveCSS('background-color', 'rgba(0, 0, 0, 0.3)');
    await expect(page.locator('.backdrop')).toHaveCSS('backdrop-filter', 'blur(3px)');
    for (const [label, icon] of Object.entries({ Settings: 'settings-2', 'New chat': 'message-square', 'Existing chat': 'message-square', 'Terminal: zsh': 'square-terminal', Everyone: 'radio', Agent: 'bot', Project: 'folder' })) {
      const row = page.getByRole('button', { name: label, exact: true });
      await expect(row.locator(`.result-icon svg.lucide-${icon}`)).toHaveCount(1);
      await expect(row.locator('.result-icon')).toHaveAttribute('aria-hidden', 'true');
    }
    const input = page.getByRole('textbox');
    await expect(input).toBeFocused();
    await input.fill('terminal');
    await expect(page.locator('.result')).toHaveCount(1);
    await input.press('Enter');
    expect(await page.evaluate(() => window.__PALETTE_QA__.selected)).toBe('terminal:1');
    await input.press('Escape');
    expect(await page.evaluate(() => window.__PALETTE_QA__.closed)).toBe(true);
    expect(errors).toEqual([]);
    const bounds = await page.locator('.palette').boundingBox();
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    await page.close();
  }
  console.log('WebKit: 30% black blurred backdrop, relevant real icons, filtering, keyboard selection and mobile bounds pass.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
