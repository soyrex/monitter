import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Builds only the real MessagePane and this tiny fixture. It avoids starting
// the full SvelteKit/Tauri application, whose development server is costly.
const messagePanePath = `${process.cwd()}/src/lib/components/MessagePane.svelte`;
const lucideStubPath = fileURLToPath(new URL('./fixtures/lucide-stub.js', import.meta.url));
const messagePaneRef = process.env.MESSAGE_PANE_REF;
const overrideSource = messagePaneRef
  ? execFileSync('git', ['show', `${messagePaneRef}:src/lib/components/MessagePane.svelte`], { encoding: 'utf8' })
  : null;
const result = await build({
  configFile: false,
  plugins: [{
    name: 'message-pane-git-ref-override',
    enforce: 'pre',
    load(id) {
      return id === messagePanePath && overrideSource !== null
        ? { code: overrideSource, map: null }
        : null;
    },
  }, svelte()],
  // The arrow is decorative; stubbing it avoids resolving Lucide's large
  // barrel while leaving the real MessagePane and all scroll logic intact.
  resolve: { alias: { '@lucide/svelte': lucideStubPath } },
  root: process.cwd(),
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/message-pane-scroll-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated MessagePane entry was built');
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
const assertions = [];
try {
  for (const profile of [
    { name: 'desktop WebKit', viewport: { width: 1120, height: 680 }, isMobile: false },
    { name: 'mobile WebKit', viewport: { width: 390, height: 844 }, isMobile: true },
  ]) {
    const context = await browser.newContext({ viewport: profile.viewport, isMobile: profile.isMobile, hasTouch: profile.isMobile });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await expect.poll(() => page.evaluate(() => Boolean(window.__PANE_QA__))).toBe(true);
    const metrics = () => page.locator('.messages').evaluate(node => ({ top: node.scrollTop, height: node.clientHeight, total: node.scrollHeight }));
    const atBottom = () => expect.poll(async () => { const m = await metrics(); return m.total - m.height - m.top; }).toBeLessThanOrEqual(2);
    const scrollUp = async () => page.locator('.messages').evaluate(node => { node.scrollTop = 0; node.dispatchEvent(new Event('scroll')); });
    const invoke = name => page.evaluate(name => window.__PANE_QA__[name](), name);
    const grow = async name => { const before = await metrics(); await invoke(name); await expect.poll(async () => (await metrics()).total).toBeGreaterThan(before.total + 8); };

    await atBottom();
    await grow('growExisting');
    await atBottom();
    await scrollUp();
    const readerBefore = await metrics();
    await grow('append');
    expect((await metrics()).top).toBeLessThanOrEqual(readerBefore.top + 2);
    await expect(page.getByRole('button', { name: 'Jump to latest message' })).toBeVisible();
    await page.getByRole('button', { name: 'Jump to latest message' }).click();
    await atBottom();

    // This is the exact old failure: content is taller, then scroll fires,
    // then ResizeObserver runs. HEAD leaves a bottom gap; the fix must not.
    await grow('growThenScrollBeforeObserver');
    await atBottom();
    await grow('shrinkThenGrow');
    await atBottom();
    await scrollUp();
    const preserved = await metrics();
    await grow('shrinkThenGrow');
    expect((await metrics()).top).toBeLessThanOrEqual(preserved.top + 2);
    await invoke('send');
    await atBottom();
    await page.setViewportSize({ width: profile.viewport.width, height: profile.viewport.height - 120 });
    await atBottom();
    await page.setViewportSize(profile.viewport);
    await atBottom();
    if (profile.isMobile) {
      const clearance = await page.locator('.message-content').evaluate(node => ({ padding: Number.parseFloat(getComputedStyle(node).paddingBottom), fade: Number.parseFloat(getComputedStyle(node.closest('.messages')).getPropertyValue('--scroll-fade')) }));
      expect(clearance.padding).toBeGreaterThanOrEqual(clearance.fade + 8);
    }
    expect(errors).toEqual([]);
    assertions.push(`${profile.name}: streaming existing reply, scroll-before-RO late layout, shrink/grow, reader preservation, resetKey send, resize${profile.isMobile ? ', final clearance' : ''}`);
    await context.close();
  }
  console.log(`${messagePaneRef ? `${messagePaneRef}: ` : 'working tree: '}${assertions.join('\n')}`);
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
