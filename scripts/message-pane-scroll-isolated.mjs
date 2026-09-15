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
  resolve: { alias: { '$lib': `${process.cwd()}/src/lib`, '@lucide/svelte': lucideStubPath } },
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
    { name: 'narrow pane WebKit', viewport: { width: 260, height: 680 }, isMobile: false },
    { name: 'mobile WebKit', viewport: { width: 390, height: 844 }, isMobile: true },
  ]) {
    const context = await browser.newContext({ viewport: profile.viewport, isMobile: profile.isMobile, hasTouch: profile.isMobile });
    const page = await context.newPage();
    await page.addInitScript(() => { Math.random = () => 0.5; });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await expect.poll(() => page.evaluate(() => Boolean(window.__PANE_QA__))).toBe(true);
    const metrics = () => page.locator('.messages').evaluate(node => ({ top: node.scrollTop, height: node.clientHeight, total: node.scrollHeight }));
    const atBottom = () => expect.poll(async () => { const m = await metrics(); return m.total - m.height - m.top; }).toBeLessThanOrEqual(2);
    const setBottomGap = gap => page.locator('.messages').evaluate((node, value) => {
      node.scrollTop = node.scrollHeight - node.clientHeight - value;
      node.dispatchEvent(new Event('scroll'));
    }, gap);
    const scrollUp = async () => page.locator('.messages').evaluate(node => { node.scrollTop = 0; node.dispatchEvent(new Event('scroll')); });
    const invoke = name => page.evaluate(name => window.__PANE_QA__[name](), name);
    const grow = async name => { const before = await metrics(); await invoke(name); await expect.poll(async () => (await metrics()).total).toBeGreaterThan(before.total + 8); };

    await atBottom();
    await page.locator('.messages').evaluate(node => {
      node.dispatchEvent(new WheelEvent('wheel', { deltaY: 12, bubbles: true }));
      node.dispatchEvent(new WheelEvent('wheel', { deltaX: 18, deltaY: 4, bubbles: true }));
      node.focus();
      node.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
      const touchEvent = (type, clientY) => {
        const event = new Event(type, { bubbles: true });
        Object.defineProperty(event, 'touches', { value: [{ clientY }] });
        return event;
      };
      node.dispatchEvent(touchEvent('touchstart', 160));
      node.dispatchEvent(touchEvent('touchmove', 120));
    });
    await grow('growExisting');
    await atBottom();
    await grow('growExisting');
    await atBottom();
    const jumpButton = page.locator('.jump-latest');
    await page.locator('.messages').evaluate(node => {
      node.dispatchEvent(new WheelEvent('wheel', { deltaY: -8, bubbles: true }));
      node.scrollTop -= 8;
      node.dispatchEvent(new Event('scroll'));
    });
    await expect(page.getByRole('button', { name: 'Jump to latest message' })).toBeVisible();
    const firstIntent = await metrics();
    await invoke('tickTimer');
    await page.waitForTimeout(50);
    expect((await metrics()).top).toBeLessThanOrEqual(firstIntent.top + 2);
    await page.getByRole('button', { name: 'Jump to latest message' }).click();
    await atBottom();
    await page.waitForTimeout(50);
    await setBottomGap(49);
    await expect(jumpButton).toHaveAttribute('aria-hidden', 'true');
    await page.waitForTimeout(1_100);
    await atBottom();
    await setBottomGap(50);
    await expect(page.getByRole('button', { name: 'Jump to latest message' })).toBeVisible();
    await expect(jumpButton).toHaveCSS('opacity', '1');
    const restingBorder = await jumpButton.evaluate(node => getComputedStyle(node).borderColor);
    await invoke('beginThinking');
    await expect(jumpButton).toHaveCSS('border-color', 'rgb(0, 102, 102)');
    expect(await jumpButton.evaluate(node => getComputedStyle(node).animationName)).toContain('jump-attention-glow');
    expect(await jumpButton.evaluate(node => getComputedStyle(node, '::before').content)).not.toBe('none');
    expect(await jumpButton.evaluate(node => getComputedStyle(node, '::before').animationName)).toContain('jump-attention-twinkle');
    await invoke('endThinking');
    await expect(jumpButton).toHaveCSS('border-color', restingBorder);
    const manual = await metrics();
    const manualGap = manual.total - manual.height - manual.top;
    await page.waitForTimeout(1_100);
    const held = await metrics();
    expect(held.total - held.height - held.top).toBeGreaterThanOrEqual(manualGap - 2);
    await page.getByRole('button', { name: 'Jump to latest message' }).click();
    await atBottom();
    await invoke('beginThinking');
    await atBottom();
    const timerRowHeight = await page.locator('.reasoning-pending').evaluate(node => node.getBoundingClientRect().height);
    const initialTimerText = await page.getByLabel('Elapsed time').textContent();
    const timerTick = await page.evaluate(() => new Promise(resolve => {
      const viewport = document.querySelector('.messages');
      const row = document.querySelector('.reasoning-pending');
      const started = performance.now();
      let maximumGap = 0;
      let minimumTop = viewport.scrollTop;
      const sample = () => {
        maximumGap = Math.max(maximumGap, viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop);
        minimumTop = Math.min(minimumTop, viewport.scrollTop);
        // The shared clock ticks on wall-clock seconds, which can fall before
        // this run's next elapsed second. Allow two ticks plus render time.
        if (performance.now() - started < 2_300) requestAnimationFrame(sample);
        else resolve({ maximumGap, minimumTop, rowHeight: row.getBoundingClientRect().height, timerText: row.querySelector('time')?.textContent, top: viewport.scrollTop });
      };
      requestAnimationFrame(sample);
    }));
    expect(timerTick.maximumGap).toBeLessThanOrEqual(2);
    expect(timerTick.minimumTop).toBeGreaterThanOrEqual(timerTick.top - 2);
    expect(Math.abs(timerTick.rowHeight - timerRowHeight)).toBeLessThanOrEqual(0.5);
    expect(timerTick.timerText).not.toBe(initialTimerText);
    await invoke('endThinking');
    await atBottom();
    await page.waitForTimeout(50);
    await setBottomGap(10);
    await invoke('tickTimer');
    const afterTextTick = await metrics();
    expect(afterTextTick.total - afterTextTick.height - afterTextTick.top).toBeLessThanOrEqual(2);
    await scrollUp();
    const readerBefore = await metrics();
    await invoke('tickTimer');
    expect((await metrics()).top).toBeLessThanOrEqual(readerBefore.top + 2);
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
    {
      const clearance = await page.locator('.message-content').evaluate(node => ({ padding: Number.parseFloat(getComputedStyle(node).paddingBottom), fade: Number.parseFloat(getComputedStyle(node.closest('.messages')).getPropertyValue('--scroll-fade')) }));
      expect(clearance.padding).toBeGreaterThanOrEqual(clearance.fade + 8);
    }
    // Buffering uses the same production helper as direct and channel chats.
    await invoke('enableBuffer');
    await atBottom();
    // A gesture can detach before any physical movement; queued updates must
    // still expose a usable arrow rather than trapping the reader at old bottom.
    await page.locator('.messages').evaluate(node => node.dispatchEvent(new WheelEvent('wheel', { deltaY: -1, bubbles: true })));
    await invoke('append');
    await expect(jumpButton).toBeVisible();
    await jumpButton.click();
    await atBottom();
    await scrollUp();
    const frozenText = await page.locator('.message-content').textContent();
    const frozenMetrics = await metrics();
    const frozenCount = await page.locator('article').count();
    await invoke('append');
    await invoke('growExisting');
    await invoke('mutateFirst');
    await page.waitForTimeout(100);
    expect(await page.locator('.message-content').textContent()).toBe(frozenText);
    expect(await page.locator('article').count()).toBe(frozenCount);
    expect(await metrics()).toEqual(frozenMetrics);
    await expect(jumpButton).toHaveAttribute('data-pending-updates', 'true');
    await page.evaluate(async () => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
      await new Promise(resolve => setTimeout(resolve, 30));
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await page.waitForTimeout(50);
    expect(await page.locator('.message-content').textContent()).toBe(frozenText);
    await expect(jumpButton).toHaveAttribute('data-pending-updates', 'true');
    await jumpButton.click();
    await atBottom();
    await expect(page.locator('article')).toHaveCount(frozenCount + 1);
    await expect(page.locator('article').first()).toContainText('Changed while reading');
    await expect(jumpButton).toHaveAttribute('data-pending-updates', 'false');

    // Reaching the old bottom manually also flushes to the new bottom.
    await scrollUp();
    await invoke('append');
    await setBottomGap(0);
    await atBottom();
    await expect(page.locator('article')).toHaveCount(frozenCount + 2);
    await expect(jumpButton).toHaveAttribute('data-pending-updates', 'false');

    // Sending and switching chats must release the held view without leakage.
    await scrollUp();
    await invoke('append');
    await invoke('send');
    await atBottom();
    await expect(page.locator('article')).toHaveCount(frozenCount + 4);
    await scrollUp();
    await invoke('append');
    await invoke('switchChat');
    await atBottom();
    await expect(page.locator('article')).toHaveCount(1);
    await expect(page.locator('article')).toContainText('Other conversation');
    await expect(jumpButton).toHaveAttribute('data-pending-updates', 'false');
    expect(errors).toEqual([]);
    assertions.push(`${profile.name}: strict 50px intent threshold, stable elapsed timer, immediate live-text bottom correction, thinking-aware jump sparkle, periodic absolute-bottom correction, animated jump, streaming growth, reader preservation, send, resize, final clearance, frozen updates, manual catch-up, visibility preservation, conversation isolation`);
    await context.close();
  }
  console.log(`${messagePaneRef ? `${messagePaneRef}: ` : 'working tree: '}${assertions.join('\n')}`);
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
