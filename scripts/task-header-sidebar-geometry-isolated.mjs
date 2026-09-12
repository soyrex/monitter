import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';

// Keep this fixture deliberately small, but always import the current AppSurface
// stylesheet. This catches geometry regressions without maintaining a CSS mirror.
const source = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const stylesheet = source.slice(source.lastIndexOf('<style>') + 7, source.lastIndexOf('</style>'))
  .replace(/:global\(([^)]+)\)/g, '$1');
const html = `<!doctype html><meta name="viewport" content="width=device-width"><style>
  :root { --paper:#fafafa; --sidebar:#f1f1ef; --panel:#fff; --line:#aaa; --soft:#eee; --ink:#111; --muted:#555; --accent:#36795a; --accent-ink:#175b35; --mono:monospace; --interface-font-ratio:1; }
  * { box-sizing:border-box; }
  html, body { margin:0; width:100%; height:100%; }
  body { font:14px system-ui,sans-serif; overflow:hidden; }
  .app-shell { height:100%; display:grid; grid-template-columns:252px minmax(0,1fr); }
  .sidebar { min-width:0; background:var(--sidebar); }
  .task-layout { min-width:0; min-height:0; height:100%; }
  .conversation { min-width:0; min-height:0; }
  .run-detail { min-width:0; min-height:0; }
  .task-heading-identity { min-width:0; display:flex; align-items:center; gap:10px; }
  .task-heading-sidebar { min-width:0; display:flex; align-items:stretch; }
  .task-heading-sidebar .detail-tabs { min-width:0; }
  .task-actions button { width:30px; height:30px; flex:none; }
  .task-header-avatar { display:block; background:var(--accent); }
  .task-title-edit { display:none; }
  ${stylesheet}
</style>
<div class="app-shell"><aside class="sidebar"></aside><section class="task-layout">
  <header class="conversation-head task-heading pane-task-header has-detail-tabs">
    <div class="task-heading-identity"><span class="avatar task-header-avatar"></span><h1 class="task-title">A deliberately long task title that should not displace sidebar controls</h1></div>
    <div class="task-heading-sidebar"><div class="detail-tabs" role="tablist" aria-label="Run detail views">
      <div class="detail-tab-entry active"><button class="detail-tab" role="tab" aria-selected="true">Run detail</button></div>
      <div class="detail-tab-entry"><button class="detail-tab" role="tab" aria-selected="false">Timeline</button></div>
      <div class="detail-tab-entry"><button class="detail-tab" role="tab" aria-selected="false">Approvals</button></div>
      <div class="detail-tab-entry"><button class="detail-tab" role="tab" aria-selected="false">Git changes</button></div>
    </div><div class="task-actions"><button aria-label="Expand">E</button><button aria-label="Close">X</button></div></div>
  </header><section class="conversation"></section><aside class="run-detail"></aside>
</section></div>`;

const server = createServer((_, response) => response.writeHead(200, {'content-type':'text/html'}).end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No server');
const browser = await webkit.launch({headless:true});

try {
  for (const modern of [false, true]) for (const viewportWidth of [1000, 1280, 1600]) {
    for (const sidebarWidth of [260, 292, 360]) for (const ratio of [1, 1.25]) {
      const context = await browser.newContext({viewport:{width:viewportWidth, height:520}});
      const page = await context.newPage();
      await page.goto(`http://127.0.0.1:${address.port}`);
      await page.locator('.app-shell').evaluate((node, value) => node.style.setProperty('--right-sidebar-width', `${value}px`), sidebarWidth);
      await page.locator('.app-shell').evaluate((node, value) => node.style.setProperty('--interface-font-ratio', value), ratio);
      await page.locator('.app-shell').evaluate((node, value) => node.classList.toggle('modern-tabs', value), modern);
      const metrics = await page.evaluate(() => {
        const box = selector => document.querySelector(selector).getBoundingClientRect();
        const header = box('.pane-task-header');
        const group = box('.task-heading-sidebar');
        const tabs = box('.detail-tabs');
        const active = box('.detail-tab-entry.active');
        const actions = box('.task-actions');
        const tabsNode = document.querySelector('.detail-tabs');
        const style = selector => getComputedStyle(document.querySelector(selector));
        return { header, group, tabs, active, actions,
          tabScrollWidth: tabsNode.scrollWidth, tabClientWidth: tabsNode.clientWidth,
          headerBorder: parseFloat(style('.pane-task-header').borderBottomWidth),
          activeRadius: style('.detail-tab-entry.active').borderTopLeftRadius,
          activeButtonRadius: style('.detail-tab').borderTopLeftRadius,
          headerHeight: header.height, groupHeight: group.height };
      });
      const sidebarLeft = metrics.header.right - sidebarWidth;
      expect(Math.abs(metrics.group.left - sidebarLeft), 'right group starts at the configured sidebar edge').toBeLessThanOrEqual(1);
      expect(Math.abs(metrics.actions.right - (metrics.header.right - 15)), 'controls keep a 15px right edge').toBeLessThanOrEqual(1);
      expect(metrics.actions.left).toBeGreaterThan(metrics.tabs.left);
      expect(metrics.tabs.right).toBeLessThanOrEqual(metrics.actions.left);
      expect(metrics.tabScrollWidth).toBeGreaterThan(metrics.tabClientWidth);
      if (modern) {
        expect(Math.abs(metrics.active.height - metrics.groupHeight)).toBeLessThanOrEqual(1);
        expect(metrics.activeRadius).toBe('0px');
        expect(metrics.activeButtonRadius).toBe('0px');
      } else {
        // The active classic tab owns the one-pixel divider overlap.
        expect(Math.abs(metrics.active.bottom - (metrics.header.bottom - metrics.headerBorder))).toBeLessThanOrEqual(1);
      }
      await context.close();
    }
    console.log(`${modern ? 'modern' : 'classic'} header: ${viewportWidth}px × 3 sidebar widths × 2 font scales OK`);
  }
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
