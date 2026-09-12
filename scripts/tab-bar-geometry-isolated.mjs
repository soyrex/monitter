import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { createServer } from 'node:http';
import { webkit, expect } from '@playwright/test';

// This deliberately small browser fixture mirrors the actual tab CSS contract
// so the close/status slot is checked geometrically without booting Tauri.
const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
for (const rule of [
  '.tab-picker-list .tab-status { position:absolute;',
  '.tab-picker-list .tab-entry:hover .tab-status, .tab-picker-list .tab-entry:focus-within .tab-status { opacity:0; }',
  '.tab-status, .tab-picker-list .tab-status { opacity:0; }',
  'border:1px solid var(--line); border-radius:4px; background:var(--panel);',
]) assert.ok(source.includes(rule), `Missing tab geometry rule: ${rule}`);

const html = `<!doctype html><style>
:root { --line:#b7c3c4; --panel:#fff; --soft:#e7f3f2; --accent-ink:#067b73; --muted:#566; --mono:ui-monospace,monospace; }
body { margin:20px; font:12px system-ui; }.tabs{counter-reset:tab-index;display:flex;gap:3px}.tab-entry{counter-increment:tab-index;position:relative;display:flex;align-items:stretch;border:1px solid transparent;border-radius:6px}.tab{display:flex;align-items:center;gap:7px;min-height:28px;padding:0 31px 0 9px;border:0;background:transparent}.tab-kind-icon{position:relative;display:grid;place-items:center;width:13px;height:13px}.bubble{width:13px;height:10px;border:1px solid currentColor;border-radius:3px}.tab-shortcut{display:none}.tab-status,.close-tab{position:absolute;z-index:2;right:3px;top:50%;display:grid;place-items:center;width:22px;height:24px;transform:translateY(-50%);transition:opacity .12s}.tab-status{pointer-events:none}.close-tab{opacity:0}.tab-entry:hover .tab-status,.tab-entry:focus-within .tab-status{opacity:0}.tab-entry:hover .close-tab,.tab-entry:focus-within .close-tab{opacity:1}.dot{width:7px;height:7px;border-radius:50%;background:#087d76}.show-tab-index .task-tab .bubble{opacity:0}.show-tab-index .task-tab .tab-shortcut{position:absolute;inset:-3px;display:grid;place-items:center;border:1px solid var(--line);border-radius:4px;background:var(--panel);color:var(--accent-ink);font:11px var(--mono)}.show-tab-index .task-tab .tab-shortcut::after{content:counter(tab-index)}
.compact .tabs{display:block;position:relative}.compact .tab-picker-list{display:grid;gap:3px;width:300px}.compact .tab-entry{min-height:44px}.compact .tab{flex:1;width:0;min-height:44px;padding-right:8px}.compact .close-tab{position:static;flex:none;width:44px;height:44px;transform:none;opacity:0;pointer-events:none}.compact .tab-status{position:absolute;z-index:3;right:0;top:0;width:44px;height:44px;transform:none;opacity:1;pointer-events:none}.compact .tab-entry:hover,.compact .tab-entry:focus-within{background:var(--soft)}.compact .tab-entry:hover .tab-status,.compact .tab-entry:focus-within .tab-status{opacity:0}.compact .tab-entry:hover .close-tab,.compact .tab-entry:focus-within .close-tab{opacity:1;pointer-events:auto}.edit-tab{width:44px;border:0;background:transparent}.compact .tab{background:transparent}
</style><section class="tabs show-tab-index" id="strip"><div class="tab-entry task-tab"><button class="tab"><span class="tab-kind-icon"><span class="bubble"></span><span class="tab-shortcut"></span></span><span>Chat</span></button><span class="tab-status"><span class="dot"></span></span><button class="close-tab">×</button></div></section><section class="compact"><div class="tabs show-tab-index"><div class="tab-picker-list"><div class="tab-entry task-tab"><button class="tab"><span class="tab-kind-icon"><span class="bubble"></span><span class="tab-shortcut"></span></span><span>Chat</span></button><button class="edit-tab">Edit</button><span class="tab-status"><span class="dot"></span></span><button class="close-tab">×</button></div></div></div></section>`;
const server = createServer((_request, response) => response.end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address(); if (!address || typeof address === 'string') throw new Error('No test server address');
const browser = await webkit.launch({ headless:true });
try {
  const page = await browser.newPage({ viewport:{width:600,height:300} }); await page.goto(`http://127.0.0.1:${address.port}`);
  for (const selector of ['#strip .tab-entry', '.compact .tab-entry']) {
    const entry = page.locator(selector); const status = entry.locator('.tab-status'); const close = entry.locator('.close-tab');
    const initial = await Promise.all([status.boundingBox(), close.boundingBox(), status.evaluate(node=>getComputedStyle(node).opacity), close.evaluate(node=>getComputedStyle(node).opacity)]);
    expect(initial[0].x).toBeCloseTo(initial[1].x, 0); expect(initial[0].y).toBeCloseTo(initial[1].y, 0); expect(initial[2]).toBe('1'); expect(initial[3]).toBe('0');
    await entry.hover(); await expect.poll(() => status.evaluate(node=>getComputedStyle(node).opacity)).toBe('0'); await expect.poll(() => close.evaluate(node=>getComputedStyle(node).opacity)).toBe('1');
  }
  const compactEntry = page.locator('.compact .tab-entry'); await compactEntry.hover(); expect(await compactEntry.evaluate(node=>getComputedStyle(node).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
  expect(await page.locator('#strip .tab-shortcut').evaluate(node=>getComputedStyle(node).display)).toBe('grid');
  console.log('tab bar geometry: status and Close share one slot; hover/focus swaps them; modifier keycap replaces chat icon; compact row hover spans actions');
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
