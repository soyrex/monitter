import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { chromium, expect, webkit } from '@playwright/test';
import { createServer } from 'node:http';

const result = await build({
  configFile: false,
  plugins: [svelte()],
  resolve: { alias: {
    '$lib': `${process.cwd()}/src/lib`,
    '@lucide/svelte': `${process.cwd()}/scripts/fixtures/run-activity-process-history-lucide-stub.js`,
  } },
  root: process.cwd(),
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/run-activity-process-history-entry.js' } },
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

let browser;
try {
  try { browser = await webkit.launch({ headless: true }); } catch (webkitError) {
    console.warn(`WebKit unavailable (${webkitError.message}); trying Chromium.`);
    browser = await chromium.launch({ headless: true, channel: 'chrome' }).catch(() => chromium.launch({ headless: true }));
  }
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  await expect.poll(() => page.evaluate(() => Boolean(window.__PROCESS_HISTORY_QA__))).toBe(true);

  const qa = action => page.evaluate(actionName => window.__PROCESS_HISTORY_QA__[actionName](), action);
  await qa('startFirstTurn');
  await expect(page.locator('[data-message-id="user-1"]')).toHaveText('Inspect the first command.');
  await qa('startFirstCommand');
  await qa('completeFirstCommand');
  const firstTree = page.locator('details.process-tree');
  await expect(firstTree).toHaveCount(1);
  await firstTree.locator(':scope > summary').click();
  // Blank reasoning is transient and must not become an invented thought branch;
  // the completed command history is still rendered as a process tree.
  await expect(firstTree.locator('.process-branch')).toHaveCount(1);
  await expect(firstTree.locator('.process-branch')).not.toContainText('Thought process');
  await expect(firstTree.locator('.process-branch')).toContainText('Ran 2 commands');
  await expect(firstTree.locator('.process-step')).toHaveCount(2);
  await expect(firstTree.locator('[aria-label="Tool details: command_execution"]').nth(0)).toContainText('printf first');
  await expect(firstTree.locator('[aria-label="Tool details: command_execution"]').nth(1)).toContainText('output for printf first');

  await qa('firstReplyAndSecondTurn');
  await expect(page.locator('[data-message-id="reply-1"]')).toHaveText('First command finished.');
  await expect(page.locator('[data-message-id="user-2"]')).toHaveText('Run the second command.');
  await expect(page.locator('details.process-tree')).toHaveCount(2);
  const retainedTree = page.locator('details.process-tree').filter({ hasText: 'printf first' });
  await expect(retainedTree).toHaveCount(1);
  await expect(retainedTree.locator('.process-branch')).toHaveCount(1);
  await expect(retainedTree.locator('.process-branch')).not.toContainText('Thought process');
  await expect(retainedTree).toContainText('output for printf first');
  await expect(page.locator('details.process-tree').filter({ hasText: 'printf second' })).toHaveCount(1);

  await qa('summaryScenario');
  const summaryTree = page.locator('details.process-tree');
  await expect(summaryTree).toHaveCount(1);
  await summaryTree.locator(':scope > summary').click();
  await expect(summaryTree.locator('.process-thought')).toContainText('I checked the command output before replying.');
  await expect(summaryTree.locator('.process-step')).toHaveCount(3);
  expect(errors).toEqual([]);
  console.log('Process history: reasoning-first tree, nested command output, completed blank-reasoning retention, and separate summary scenario passed.');
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
