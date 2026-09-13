import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const stylesheet = source.slice(source.lastIndexOf('<style>') + 7, source.lastIndexOf('</style>'));
function rules(selector) {
  const results = [];
  let cursor = 0;
  while ((cursor = stylesheet.indexOf(`\n  ${selector} {`, cursor)) >= 0) {
    const start = cursor;
    cursor = stylesheet.indexOf('{', cursor) + 1;
    let depth = 1;
    while (depth && cursor < stylesheet.length) {
      if (stylesheet[cursor] === '{') depth++;
      if (stylesheet[cursor] === '}') depth--;
      cursor++;
    }
    results.push(stylesheet.slice(start, cursor).replace(/:global\(([^)]+)\)/g, '$1'));
  }
  if (!results.length) throw new Error(`Actual AppSurface rule missing: ${selector}`);
  return results.join('\n');
}
const css = [
  '.tabs', '.tab-picker-list', '.tab', '.tab-entry', '.tab-entry .tab',
  '.tab-kind-icon', '.tab-shortcut', '.close-tab', '.tab-entry:focus-within .close-tab',
  ':global(.compact-tabs)', '.tabs > .tab-picker-list > .tab-entry',
  '.topbar', '.workspace.modern-tabs:not(.compact-tabs) > .topbar',
  '.modern-tabs:not(.compact-tabs) .tabs', '.modern-tabs:not(.compact-tabs) .tab-entry',
  '.modern-tabs:not(.compact-tabs) .tab',
  '.detail-tabs', '.detail-tab-entry', '.detail-tab', '.detail-tab-entry.active',
  '.modern-tabs .detail-tabs', '.modern-tabs .detail-tab-entry', '.modern-tabs .detail-tab',
  '.conversation-head', '.task-heading', '.conversation-head h1',
  '.conversation-head.pane-task-header', '.pane-task-header > h1',
  '.pane-task-header > .task-header-avatar',
  ".tabs.show-tab-index > .tab-picker-list > .tab-entry:not([data-tab-kind='task']):not([data-tab-kind='draft']):not([data-tab-kind='terminal']):not([data-tab-kind='settings'])::after",
  '.tabs.show-tab-index > .tab-picker-list > .tab-entry .tab-kind-icon :global(svg)',
  '.tabs.show-tab-index > .tab-picker-list > .tab-entry .tab-shortcut',
  '.tabs.show-tab-index > .tab-picker-list > .tab-entry .tab-shortcut::after',
  '@media (hover:hover) and (pointer:fine)', '@media (hover:none), (pointer:coarse)',
].map(rules).join('\n');
const kinds = ['task', 'draft', 'terminal', 'settings'];
const rows = kinds.map(kind => `<div class="tab-entry ${kind}-tab" data-tab-kind="${kind}"><button class="tab"><span class="tab-kind-icon"><svg aria-hidden="true"></svg><span class="tab-shortcut"></span></span><span>${kind}</span></button><button class="close-tab" aria-label="Close ${kind}">×</button></div>`).join('');
const html = `<!doctype html><style>
:root{--line:#aaa;--panel:#fff;--paper:#fafafa;--soft:#eee;--muted:#555;--accent-ink:#175b35;--mono:monospace;--sidebar:#fff;--pane-tabbar-height:46px;--density-tabbar-inset:6px;--density-tab-min-height:28px;--density-detail-tabs-height:32px;--density-detail-tab-height:24px;--density-pane-header-y:8px;--density-header-avatar-size:30px}body{font:12px system-ui;margin:20px}button{font:inherit}.surface{margin-bottom:25px}.tab-kind-icon svg{width:13px;height:13px;background:currentColor}.compact-tabs{width:320px;height:210px}
${css}
.compact-tabs .tab-picker-list{top:0}
</style><div class="surface" id="strip"><nav class="tabs tab-picker"><div class="tab-picker-list">${rows}</div></nav></div><div class="surface workspace modern-tabs" id="modern"><header class="topbar"><nav class="tabs tab-picker"><div class="tab-picker-list">${rows}</div></nav></header></div><div class="surface compact-tabs" id="picker"><nav class="tabs tab-picker tab-picker-open"><div class="tab-picker-list">${rows}</div></nav></div><div class="conversation-head task-heading pane-task-header" id="chat-header"><span class="avatar task-header-avatar"></span><h1>Chat title</h1><button>Sidebar</button></div>`;
const server = createServer((_, response) => response.writeHead(200, { 'content-type': 'text/html' }).end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
const browser = await webkit.launch({ headless: true });
try {
  for (const touch of [false, true]) {
    const context = await browser.newContext({ viewport: { width: 700, height: 700 }, hasTouch: touch, isMobile: touch });
    const page = await context.newPage();
    await page.goto(`http://127.0.0.1:${address.port}`);
    for (const surface of ['strip', 'modern', 'picker']) {
      for (const kind of kinds) {
        const entry = page.locator(`#${surface} [data-tab-kind="${kind}"]`);
        const icon = entry.locator('.tab-kind-icon');
        const keycap = entry.locator('.tab-shortcut');
        const close = entry.locator('.close-tab');
        await expect(keycap).toHaveCSS('display', 'none');
        await expect(icon.locator('svg')).toHaveCSS('opacity', '1');
        await page.locator(`#${surface} .tabs`).evaluate(node => node.classList.add('show-tab-index'));
        await expect(keycap).toHaveCSS('display', 'grid');
        await expect(icon.locator('svg')).toHaveCSS('opacity', '0');
        expect(await keycap.evaluate(node => getComputedStyle(node, '::after').content)).toBe('counter(tab-index)');
        expect(await entry.evaluate(node => getComputedStyle(node, '::after').content)).toBe('none');
        const [i, k, c] = await Promise.all([icon.boundingBox(), keycap.boundingBox(), close.boundingBox()]);
        expect(Math.abs((i.x + i.width / 2) - (k.x + k.width / 2))).toBeLessThan(1);
        expect(k.x + k.width).toBeLessThan(c.x);
        if (surface === 'picker') { expect(c.width).toBeCloseTo(44, 2); expect(c.height).toBeCloseTo(44, 2); }
        if (!touch) await entry.hover();
        await expect.poll(() => close.evaluate(node => getComputedStyle(node).opacity), { message: `${surface}/${kind} touch=${touch}: Close must be visible` }).toBe('1');
        await close.focus();
        await expect(close).toHaveCSS('pointer-events', 'auto');
        await expect(close).toBeFocused();
        await page.locator(`#${surface} .tabs`).evaluate(node => node.classList.remove('show-tab-index'));
      }
    }
    const modernBar = await page.locator('#modern .topbar').boundingBox();
    const modernTab = page.locator('#modern .tab-entry').first();
    const modernBounds = await modernTab.boundingBox();
    expect(modernBounds.y).toBe(modernBar.y);
    expect(modernBounds.height).toBe(modernBar.height);
    expect(modernBar.height).toBeCloseTo(42, 2);
    await expect(modernTab).toHaveCSS('border-top-left-radius', '0px');
    await page.locator('#modern').evaluate(node => {
      const tabs = document.createElement('div');
      tabs.className = 'detail-tabs';
      tabs.innerHTML = '<div class="detail-tab-entry active"><button class="detail-tab">Run detail</button></div><div class="detail-tab-entry"><button class="detail-tab">Approvals</button></div>';
      node.append(tabs);
    });
    const detailRow = page.locator('#modern .detail-tabs');
    const detailTab = detailRow.locator('.detail-tab-entry').first();
    await expect(detailTab).toHaveCSS('border-top-left-radius', '0px');
    await expect(detailTab.locator('button')).toHaveCSS('border-top-left-radius', '0px');
    expect((await detailTab.boundingBox()).height).toBeCloseTo((await detailRow.boundingBox()).height - 1, 2);
    await page.locator('#modern').evaluate(node => node.classList.remove('modern-tabs'));
    await expect(detailTab).toHaveCSS('border-top-left-radius', '6px');
    await page.locator('#modern').evaluate(node => { node.classList.add('modern-tabs'); node.style.setProperty('--pane-tabbar-height', '36px'); });
    expect((await page.locator('#modern .topbar').boundingBox()).height).toBeCloseTo(32, 2);
    const header = page.locator('#chat-header');
    for (const side of ['top', 'bottom']) await expect(header).toHaveCSS(`padding-${side}`, '8px');
    for (const side of ['left', 'right']) await expect(header).toHaveCSS(`padding-${side}`, '15px');
    await expect(header.locator('h1')).toHaveCSS('font-size', '13.2px');
    await expect(header.locator('.task-header-avatar')).toHaveCSS('width', '30px');
    await expect(header).toHaveCSS('background-color', 'rgb(250, 250, 250)');
    await expect(header).toHaveCSS('backdrop-filter', 'none');
    await context.close();
  }
  console.log('Actual AppSurface CSS: left tab keycaps, independent Close, square Modern tabs and compact chat header padding pass.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
