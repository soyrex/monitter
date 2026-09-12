import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { createServer } from 'node:http';
import { webkit, expect } from '@playwright/test';

const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
for (const rule of [
  '.terminal-surface { position:relative; display:flex; flex-direction:column; flex:1; min-height:0;',
  '.terminal-pane-overlay { position:absolute; top:2px; right:10px; z-index:2; display:flex; pointer-events:none; }',
  '.terminal-pane-overlay > .pane-expand-control { pointer-events:auto;',
  '.terminal-surface :global(.terminal-pane) { flex:1; height:auto; }',
]) assert.ok(source.includes(rule), `Missing terminal overlay contract: ${rule}`);

const html = `<!doctype html><style>
body{margin:0}.terminal-surface{position:relative;display:flex;flex-direction:column;width:600px;height:300px;min-height:0;overflow:hidden;background:#090b0d}.terminal-pane-overlay{position:absolute;top:2px;right:10px;z-index:2;display:flex;pointer-events:none}.terminal-pane-overlay>.pane-expand-control{pointer-events:auto;background:#283036;color:white;width:30px;height:30px}.terminal-pane{flex:1;height:auto;background:#10171a;color:#fff}.terminal-pane button{margin:8px}
</style><div class="terminal-surface"><div class="terminal-pane-overlay"><button class="pane-expand-control" aria-label="Expand pane">↗</button></div><div class="terminal-pane"><button id="content-action">Terminal action</button></div></div>`;
const server = createServer((_request, response) => response.end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address(); if (!address || typeof address === 'string') throw new Error('No test server address');
const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 700, height: 400 } });
  await page.goto(`http://127.0.0.1:${address.port}`);
  const geometry = await page.evaluate(() => {
    const surface = document.querySelector('.terminal-surface').getBoundingClientRect();
    const pane = document.querySelector('.terminal-pane').getBoundingClientRect();
    const expand = document.querySelector('.pane-expand-control').getBoundingClientRect();
    return { surface, pane, expand, expandHit: document.elementFromPoint(expand.x + 5, expand.y + 5)?.className, contentHit: document.elementFromPoint(20, 80)?.className };
  });
  expect(geometry.pane.y).toBeCloseTo(geometry.surface.y, 0);
  expect(geometry.expandHit).toContain('pane-expand-control');
  expect(geometry.contentHit).toContain('terminal-pane');
  await page.getByRole('button', { name: 'Expand pane' }).click();
  console.log('terminal overlay geometry: terminal content starts at surface top; top-right expand control receives input; overlay does not block terminal content');
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
