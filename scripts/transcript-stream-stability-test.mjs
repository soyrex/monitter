import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chromium, webkit, expect } from '@playwright/test';

const port = 18435;
const useWebKit = process.argv.includes('--webkit');
const server = spawn(process.execPath, ['scripts/transcript-geometry-preview.mjs', `--port=${port}`], {
  cwd: process.cwd(), stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '';
const ready = new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(Error(`Fixture startup timed out: ${output}`)), 30_000);
  const receive = chunk => {
    output += chunk.toString();
    if (output.includes(`http://127.0.0.1:${port}/`)) { clearTimeout(timeout); resolve(); }
  };
  server.stdout.on('data', receive);
  server.stderr.on('data', receive);
  server.once('error', reject);
  server.once('exit', code => reject(Error(`Fixture exited ${code}: ${output}`)));
});

let browser;
try {
  await ready;
  browser = useWebKit
    ? await webkit.launch({ headless: true })
    : await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
  const page = await browser.newPage({ viewport: { width: 1120, height: 720 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${port}/`);
  await expect(page.locator('.fixture-pane .message').last()).toBeVisible();
  await page.getByRole('button', { name: 'Start 4 streams' }).click();
  const pane = page.locator('.fixture-pane').first();
  await expect(pane.locator('[data-stream-text]')).toContainText('token-1-1');
  const start = await pane.evaluate(node => {
    const viewport = node.querySelector('.messages');
    window.__streamNode = node.querySelector('[data-stream-text]');
    return { gap: viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop };
  });
  assert.ok(start.gap <= 3, `initial streaming gap: ${start.gap}px`);
  await expect(pane.locator('[data-stream-text]')).toContainText('token-1-12', { timeout: 10_000 });
  const follow = await pane.evaluate(node => {
    const viewport = node.querySelector('.messages');
    return {
      stableTextNode: window.__streamNode === node.querySelector('[data-stream-text]'),
      gap: viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop,
    };
  });
  assert.equal(follow.stableTextNode, true, 'stream ticks must not replace the live text container');
  assert.ok(follow.gap <= 3, `streaming must remain at bottom; gap ${follow.gap}px`);

  // Explicit upward intent freezes the visible history, even as the backing
  // messages continue to stream. A later jump releases that held snapshot.
  await pane.locator('.messages').evaluate(node => {
    node.scrollTop = Math.max(0, node.scrollHeight - node.clientHeight - 300);
    node.dispatchEvent(new WheelEvent('wheel', { deltaY: -80, bubbles: true }));
    node.dispatchEvent(new Event('scroll'));
  });
  await expect(pane.getByRole('button', { name: 'Jump to latest message' })).toBeVisible();
  const heldText = await pane.locator('[data-stream-text]').textContent();
  await page.waitForTimeout(350);
  assert.equal(await pane.locator('[data-stream-text]').textContent(), heldText, 'reader snapshot must remain unchanged while detached');
  await pane.getByRole('button', { name: 'Jump to latest message' }).click();
  await expect.poll(() => pane.locator('[data-stream-text]').textContent()).not.toBe(heldText);
  await expect.poll(() => pane.locator('.messages').evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop)).toBeLessThanOrEqual(3);

  await page.getByRole('button', { name: 'Use two panes' }).click();
  await expect(page.locator('.fixture-pane')).toHaveCount(2);
  const simultaneous = await page.evaluate(async () => {
    const panes = [...document.querySelectorAll('.fixture-pane')];
    const first = panes.map(node => node.querySelector('[data-stream-text]'));
    const maxGap = [0, 0];
    const started = performance.now();
    while (performance.now() - started < 1_500) {
      await new Promise(resolve => requestAnimationFrame(resolve));
      panes.forEach((pane, index) => {
        const viewport = pane.querySelector('.messages');
        maxGap[index] = Math.max(maxGap[index], viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop);
      });
    }
    return { maxGap, stable: panes.every((pane, index) => pane.querySelector('[data-stream-text]') === first[index]) };
  });
  assert.equal(simultaneous.stable, true, 'both live text containers must remain mounted');
  assert.ok(simultaneous.maxGap.every(gap => gap <= 4), `two-pane frame gaps: ${simultaneous.maxGap.join(', ')}px`);
  assert.deepEqual(errors, [], `browser errors: ${errors.join('; ')}`);
  console.log(`${useWebKit ? 'WebKit' : 'Chromium'} streaming text identity, two-pane bottom follow, held history, and catch-up passed.`);
} finally {
  await browser?.close();
  server.kill('SIGTERM');
}
