import { readFileSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:http';
import assert from 'node:assert/strict';
import { webkit, expect } from '@playwright/test';

const source = readFileSync('src/lib/components/AppSurface.svelte', 'utf8');
const nav = source.split('\n').find(line => line.includes('<nav class="tabs tab-picker"'));
assert.match(nav, /data-tauri-drag-region(?:\s|>)/);
assert.match(nav, /ondragover=\{tabBarOver\} ondrop=\{tabBarDrop\}/);
assert.ok(!/<button[^>]*data-tauri-drag-region/.test(source), 'Tab/buttons must not become window handles');
const css = source.slice(source.lastIndexOf('<style>') + 7, source.lastIndexOf('</style>'))
  .replace(/:global\(([^)]+)\)/g, '$1');

// Exercise the pinned Tauri runtime's real drag dispatch, not a reproduction.
const version = readFileSync('src-tauri/Cargo.lock', 'utf8').match(/name = "tauri"\nversion = "([^"]+)"/)[1];
const registry = join(homedir(), '.cargo/registry/src');
const candidates = readdirSync(registry).map(dir => join(registry, dir, `tauri-${version}/src/window/scripts/drag.js`));
let dragScript;
for (const path of candidates) {
  try { dragScript = readFileSync(path, 'utf8'); break; } catch { /* Next registry. */ }
}
assert.ok(dragScript, `Fetch pinned Tauri ${version} sources before running this test`);
dragScript = dragScript.replace('__TEMPLATE_os_name__', JSON.stringify('macos'));
const html = `<!doctype html><style>
*{box-sizing:border-box}body{margin:20px;--line:#777;--sidebar:#222;--paper:#333;--muted:#bbb;--ink:#fff;--soft:#444}button{font:inherit}
${css}
.workspace{width:900px;height:180px;margin:0 0 20px;flex:none}
</style>
${['classic','modern'].map(style => `<section class="workspace ${style === 'modern' ? 'modern-tabs' : ''}" id="${style}"><header class="topbar" data-tauri-drag-region><nav class="tabs tab-picker" data-tauri-drag-region><div class="tab-picker-list"><div class="tab-entry task-tab"><button class="tab" draggable="false"><span>Chat tab</span></button><button class="close-tab" aria-label="Close chat">×</button></div></div></nav><div class="top-actions" data-tauri-drag-region><button class="icon" aria-label="Open terminal">T</button></div></header></section>`).join('')}
<script>
window.calls=[];window.tabPresses=0;window.clicks=0;window.drops=0;
window.__TAURI_INTERNALS__={invoke:cmd=>{window.calls.push(cmd);return Promise.resolve()}};
document.querySelectorAll('.tab').forEach(tab=>tab.addEventListener('pointerdown',()=>window.tabPresses++));
document.querySelectorAll('button').forEach(button=>button.addEventListener('click',()=>window.clicks++));
document.querySelectorAll('.tabs').forEach(nav=>{
  nav.addEventListener('dragover',e=>{if(e.dataTransfer.types.includes('application/x-monitter-tab'))e.preventDefault()});
  nav.addEventListener('drop',e=>{if(e.dataTransfer.types.includes('application/x-monitter-tab')){e.preventDefault();window.drops++}});
});
</script><script>${dragScript}</script>`;
const server = createServer((_, res) => res.writeHead(200, {'content-type':'text/html'}).end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await webkit.launch({headless:true});
try {
  const page = await browser.newPage({viewport:{width:1000,height:600}});
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  for (const style of ['classic','modern']) {
    await page.evaluate(()=>{window.calls=[];window.tabPresses=0;window.clicks=0});
    const bar = page.locator(`#${style} .tabs`);
    const box = await bar.boundingBox();
    // Actual empty nav hit area, not the tiny header padding behind it.
    const point = {x:box.x+box.width-20,y:box.y+box.height/2};
    expect(await page.evaluate(({x,y})=>document.elementFromPoint(x,y).classList.contains('tabs'),point)).toBe(true);
    await page.mouse.move(point.x,point.y); await page.mouse.down();
    await page.mouse.move(point.x+12,point.y+8); await page.mouse.up();
    expect(await page.evaluate(()=>window.calls)).toEqual(['plugin:window|start_dragging']);
    await page.evaluate(()=>window.calls=[]);
    await page.locator(`#${style} .tab span`).click();
    await page.getByRole('button',{name:'Open terminal'}).nth(style==='classic'?0:1).click();
    await page.locator(`#${style} .tab-entry`).hover();
    await page.locator(`#${style} .close-tab`).click();
    expect(await page.evaluate(()=>window.calls)).toEqual([]);
    expect(await page.evaluate(()=>window.tabPresses)).toBe(1);
    expect(await page.evaluate(()=>window.clicks)).toBe(3);
    // Native HTML tab drops still reach the same nav target without invoking window movement.
    const dropAccepted = await bar.evaluate(node=>{
      const dataTransfer = new DataTransfer();dataTransfer.setData('application/x-monitter-tab','{}');
      const over = new DragEvent('dragover',{bubbles:true,cancelable:true,dataTransfer});
      node.dispatchEvent(over);node.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer}));
      return over.defaultPrevented;
    });
    expect(dropAccepted).toBe(true);
    expect(await page.evaluate(()=>window.calls)).toEqual([]);
  }
  expect(await page.evaluate(()=>window.drops)).toBe(2);
  expect(errors).toEqual([]);
  console.log('WebKit + pinned Tauri: empty Classic/Modern tab strips move the window; tabs, buttons and drops stay interactive.');
} finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
