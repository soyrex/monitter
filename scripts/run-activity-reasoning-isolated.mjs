import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

// Build the real RunActivity component in isolation, avoiding the full app server.
const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: {
    '$lib': `${process.cwd()}/src/lib`,
    '@lucide/svelte': `${process.cwd()}/scripts/fixtures/run-activity-lucide-stub.js`,
  } },
  root: process.cwd(),
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/run-activity-reasoning-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No isolated RunActivity entry was built');
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
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  await expect.poll(() => page.evaluate(() => Boolean(window.__REASONING_QA__))).toBe(true);
  const pending = page.locator('.reasoning-pending');
  await expect(pending).toBeVisible();
  await expect(pending.getByRole('img', { name: 'Agent avatar' })).toBeVisible();
  await expect(pending.locator('svg')).toHaveCount(0);
  await expect(page.locator('.waiting-spinner')).toHaveCount(0);
  expect(await pending.evaluate(node => {
    const style = getComputedStyle(node);
    return { border: style.borderTopWidth, background: style.backgroundColor, padding: style.paddingLeft };
  })).toEqual({ border: '0px', background: 'rgba(0, 0, 0, 0)', padding: '0px' });
  const compaction = page.locator('.activity.compaction');
  await expect(compaction).toHaveText('Compacting context...');
  await expect(compaction.locator('.animated-title')).toHaveAttribute('aria-busy', 'true');
  await expect(compaction.locator('.animated-title')).toHaveAttribute('title', 'Compacting context...');
  expect(await compaction.locator('.animated-title').evaluate(node => getComputedStyle(node.parentElement).overflow)).toBe('visible');
  await expect(page.locator('details.reasoning')).toHaveCount(0);
  const inactiveLabel = await pending.textContent();
  await page.waitForTimeout(5_200);
  await expect(pending).toHaveText(inactiveLabel || 'Thinking');
  await page.evaluate(() => window.__REASONING_QA__.activate());
  const activeLabel = await pending.textContent();
  await page.waitForTimeout(5_200);
  const rotatedLabel = await pending.textContent();
  expect(rotatedLabel).not.toBe(activeLabel);
  await page.evaluate(() => window.__REASONING_QA__.deactivate());
  const stoppedLabel = await pending.textContent();
  await page.waitForTimeout(5_200);
  await expect(pending).toHaveText(stoppedLabel || 'Thinking');
  await page.evaluate(() => window.__REASONING_QA__.emptyReply());
  await expect(pending).toBeVisible();
  await page.evaluate(() => window.__REASONING_QA__.reply());
  await expect(page.getByTestId('reply')).toHaveText('Here is the reply.');
  await expect(pending).toHaveCount(0);
  await page.evaluate(() => window.__REASONING_QA__.resetReply());
  await expect(pending).toBeVisible();
  await page.evaluate(() => window.__REASONING_QA__.summary());
  await expect(pending).toHaveCount(0);
  await expect(page.locator('details.reasoning')).toBeVisible();
  await expect(page.locator('.activity-body')).toContainText('I checked the source and found the relevant path.');
  await expect(page.locator('body')).not.toContainText('"summary"');
  await page.evaluate(() => window.__REASONING_QA__.reply());
  await expect(page.locator('details.reasoning')).toBeVisible();
  await expect(page.getByTestId('reply')).toBeVisible();
  await page.evaluate(() => window.__REASONING_QA__.completeCompaction());
  await expect(compaction).toHaveText('Context compacted in: 15.3s');
  await expect(compaction).not.toContainText('tool calls');
  await page.evaluate(() => window.__REASONING_QA__.interruptCompaction());
  await expect(compaction).toHaveText('Context compaction activity');
  await expect(compaction.locator('.animated-title')).toHaveAttribute('aria-busy', 'false');
  await page.evaluate(() => window.__REASONING_QA__.beginWaiting());
  await expect(pending).toHaveCount(1);
  await expect(pending.getByRole('img', { name: 'Agent avatar' })).toBeVisible();
  await page.evaluate(() => window.__REASONING_QA__.reportReasoning());
  await expect(pending).toHaveCount(1);
  await expect(pending.getByRole('img', { name: 'Agent avatar' })).toBeVisible();
  await page.evaluate(() => window.__REASONING_QA__.reply());
  await expect(pending).toHaveCount(0);
  expect(errors).toEqual([]);
  console.log('WebKit: one unboxed avatar/thinking row, no spinner, active label rotation, reply replacement and preserved real summaries.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
