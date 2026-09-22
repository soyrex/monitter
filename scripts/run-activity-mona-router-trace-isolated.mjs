import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

// Render the production RunActivity component directly so router-trace display
// is tested independently of the full desktop shell and its bridge.
const temporary = await mkdtemp(join(tmpdir(), 'monitter-mona-router-trace-'));
const harness = join(temporary, 'router-trace-harness.svelte');
const entry = join(temporary, 'router-trace-entry.js');
const iconStub = join(temporary, 'router-trace-lucide-stub.js');
const component = resolve('src/lib/components/RunActivity.svelte');
const iconStubSource = await readFile('scripts/fixtures/run-activity-lucide-stub.js', 'utf8');
await writeFile(iconStub, `${iconStubSource.replaceAll("'./lucide-stub.svelte'", JSON.stringify(resolve('scripts/fixtures/lucide-stub.svelte')))}\nexport { default as GitBranch } from ${JSON.stringify(resolve('scripts/fixtures/lucide-stub.svelte'))};\nexport { default as ShieldCheck } from ${JSON.stringify(resolve('scripts/fixtures/lucide-stub.svelte'))};\n`);
await writeFile(harness, `<script lang="ts">
  import RunActivity from ${JSON.stringify(component)};
  import type { RunEvent } from ${JSON.stringify(resolve('src/lib/types.ts'))};
  const event: RunEvent = {
    id: 'mona-router-rollback', taskId: 'task-mona', kind: 'status',
    title: 'Jev route rolled back · gpt-5.5 · high',
    detail: JSON.stringify({
      sessionId: 'mona-session',
      trace: {
        traceId: 'trace-fixture', trigger: 'initial_prompt', applied: false,
        requestedModel: 'gpt-6-astra', requestedEffort: 'max',
        newModel: 'gpt-5.5', newEffort: 'high',
        applicationError: 'model unavailable', rationale: 'fixture rollback',
        prompt: 'DO NOT RENDER THIS RAW PROMPT', apiKey: 'DO NOT RENDER THIS API KEY'
      }
    }),
    createdAt: Date.now(),
  };
</script>
<main><RunActivity {event}/></main>
<style>
  :global(html, body, #app) { margin: 0; }
  main { padding: 20px; --panel: #fff; --line: #ddd; --muted: #667; --ink: #171717; --accent-ink: #056; --interface-font-ratio: 1; --mono: ui-monospace, monospace; }
</style>`);
await writeFile(entry, `import { mount } from 'svelte';\nimport Harness from ${JSON.stringify(harness)};\nmount(Harness, { target: document.getElementById('app') });\n`);

try {
  const result = await build({
    configFile: false,
    plugins: [svelte()],
    resolve: { alias: {
      '$lib': `${process.cwd()}/src/lib`,
      '@lucide/svelte': iconStub,
    } },
    root: process.cwd(),
    build: { write: false, minify: false, rollupOptions: { input: entry } },
  });
  const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
  const builtEntry = result.output.find(file => file.type === 'chunk' && file.isEntry);
  if (!builtEntry) throw new Error('No isolated RunActivity entry was built');
  const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
  files.set('/', `<!doctype html><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${builtEntry.fileName}"></script>`);

  const server = createServer((request, response) => {
    const url = request.url?.split('?')[0] || '/';
    const body = files.get(url);
    if (body === undefined) return response.writeHead(404).end();
    response.writeHead(200, { 'content-type': url === '/' ? 'text/html' : url.endsWith('.css') ? 'text/css' : 'text/javascript' });
    response.end(body);
  });
  await new Promise(resolveServer => server.listen(0, '127.0.0.1', resolveServer));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('No isolated server address');

  const browser = await webkit.launch({ headless: true });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${address.port}/`);
    const row = page.locator('[data-router-trace]');
    const compact = row.locator('summary');
    await expect(compact).toBeVisible();
    await expect(compact).toContainText('gpt-5.5');
    await expect(compact).toContainText('high');
    await compact.click();
    const popup = row.locator('.routing-body');
    await expect(popup).toBeVisible();
    await expect(popup).toContainText('gpt-6-astra');
    await expect(popup).toContainText('max');
    await expect(popup).toContainText('gpt-5.5');
    await expect(popup).toContainText('high');
    await expect(popup).toContainText('model unavailable');
    await expect(page.locator('body')).not.toContainText('DO NOT RENDER THIS RAW PROMPT');
    await expect(page.locator('body')).not.toContainText('DO NOT RENDER THIS API KEY');
    expect(errors).toEqual([]);
    console.log('WebKit: Mona rollback route shows actual model/effort compactly, requested/actual/error details on expansion, and omits raw prompt/apiKey fields.');
  } finally {
    await browser.close();
    await new Promise(resolveServer => server.close(resolveServer));
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
