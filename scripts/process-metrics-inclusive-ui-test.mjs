import { chromium, expect as baseExpect } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'process-metrics-harness-'));
writeFileSync(join(harness, 'App.svelte'), `<script>
  import ProcessMetricsModal from ${JSON.stringify(join(root, 'src/lib/components/ProcessMetricsModal.svelte'))};
  const mib = 1024 * 1024;
  const process = (pid, parentPid, name, memory, cpuTimeMs) => ({ pid, parentPid, name, commandLine: name, startedAt: pid, residentMemoryBytes: memory * mib, cpuTimeMs });
  const initial = [process(1, 0, 'Monitter', 25, 0), process(2, 1, 'codex', 1, 0), process(3, 2, 'node', 250, 0), process(4, 1, 'mona', 1, 0), process(5, 4, 'python', 500, 0), process(6, 1, 'shell', 1, 0), process(7, 3, 'sleep', 1, 0)];
  const current = [process(1, 0, 'Monitter', 25, 100), process(2, 1, 'codex', 1, 100), process(3, 2, 'node', 250, 200), process(4, 1, 'mona', 1, 50), process(5, 4, 'python', 500, 250), process(6, 1, 'shell', 1, 50), process(7, 3, 'sleep', 1, 50)];
  const samples = [
    { sampledAt: 1000, rootPid: 1, cpuTimeMs: 0, residentMemoryBytes: 779 * mib, processes: initial },
    { sampledAt: 2000, rootPid: 1, cpuTimeMs: 800, residentMemoryBytes: 779 * mib, processes: current },
  ];
</script>
<ProcessMetricsModal open {samples} onclose={() => {}}/>
<style>:global(:root){color-scheme:light;--line:#d9d3c7;--panel:#fff;--soft:#eeeae2;--ink:#27231e;--muted:#776f63;--accent:#3f9d6a;--danger:#b82f3e;--mono:monospace;--interface-font-ratio:1}:global(body){font-family:system-ui}</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte';import App from './App.svelte';mount(App,{target:document.querySelector('#app')});`);
const entry = relative(root, join(harness, 'main.js'));
const server = await createServer({
  root, configFile: false, plugins: [svelte()],
  resolve: { alias: [{ find: '$lib', replacement: join(root, 'src/lib') }] },
  optimizeDeps: { noDiscovery: true, include: [] }, appType: 'custom',
  server: { host: '127.0.0.1', port: 0 },
});
server.middlewares.stack.unshift({
  route: '',
  handle(request, response, next) {
    if (request.url !== '/') return next();
    response.setHeader('Content-Type', 'text/html');
    response.end(`<div id="app"></div><script type="module" src="/${entry}"></script>`);
  },
});
let browser;
try {
  await server.listen();
  const url = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
  const page = await browser.newPage({ viewport: { width: 1100, height: 850 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url, { timeout: 60000 });
  const breakdown = page.getByRole('region', { name: 'Live process breakdown' });
  const groups = breakdown.locator('.process-group-row');
  await expect(groups).toHaveCount(3);
  await expect(groups.nth(0)).toContainText('80.0%');
  await expect(groups.nth(0)).toContainText('779 MB');
  await expect(groups.nth(1)).toContainText('35.0%');
  await expect(groups.nth(1)).toContainText('252 MB');
  await expect(groups.nth(2)).toContainText('30.0%');
  await expect(groups.nth(2)).toContainText('501 MB');
  await expect(groups.nth(1).locator('.memory-value')).toHaveAttribute('data-memory-level', 'warning');
  await expect(groups.nth(2).locator('.memory-value')).toHaveAttribute('data-memory-level', 'critical');
  const colours = await groups.evaluateAll(rows => rows.map(row => getComputedStyle(row.querySelector('.memory-value')).color));
  expect(colours).toEqual(['rgb(184, 47, 62)', 'rgb(169, 90, 13)', 'rgb(184, 47, 62)']);
  await page.evaluate(() => document.documentElement.style.colorScheme = 'dark');
  const darkColours = await groups.evaluateAll(rows => rows.map(row => getComputedStyle(row.querySelector('.memory-value')).color));
  expect(darkColours).toEqual(['rgb(255, 114, 123)', 'rgb(255, 177, 92)', 'rgb(255, 114, 123)']);

  await groups.nth(1).click();
  await expect(groups.nth(1)).toContainText('10.0%');
  await expect(groups.nth(1)).toContainText('1 MB');
  const codexChild = breakdown.locator('.process-child-row').filter({ hasText: 'node' });
  await expect(codexChild).toContainText('20.0%');
  await expect(codexChild.locator('.memory-value')).toHaveAttribute('data-memory-level', 'normal');
  await expect(breakdown.locator('.process-child-row').filter({ hasText: 'sleep' })).toContainText('5.0%');
  await groups.nth(2).click();
  await expect(groups.nth(2)).toContainText('5.0%');
  await expect(groups.nth(2)).toContainText('1 MB');
  const monaChild = breakdown.locator('.process-child-row').filter({ hasText: 'python' });
  await expect(monaChild).toContainText('25.0%');
  await expect(monaChild.locator('.memory-value')).toHaveAttribute('data-memory-level', 'warning');
  await groups.nth(0).click();
  await expect(groups.nth(0)).toContainText('10.0%');
  await expect(groups.nth(0)).toContainText('25 MB');
  await expect(breakdown.locator('.process-child-row').filter({ hasText: 'shell' })).toHaveCount(1);
  expect(errors).toEqual([]);
  console.log('Inclusive process metrics and memory text-colour assertions passed.');
} finally {
  await browser?.close();
  await server.close();
  rmSync(harness, { recursive: true, force: true });
}
