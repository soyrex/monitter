import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';

const root = process.cwd();
const source = readFileSync(`${root}/src/lib/components/AppSurface.svelte`, 'utf8');
const brandAction = readFileSync(`${root}/src/lib/responsive-brand.ts`, 'utf8');
const brandActionJs = stripTypeScriptTypes(brandAction, { mode: 'strip' });
const fixture = readFileSync(`${root}/scripts/fixtures/sidebar-chrome-isolated.html`, 'utf8');

// Keep this fixture deliberately small, but lift the declarations verbatim from
// AppSurface so this checks the shipped chrome rules rather than a second style.
const ruleNames = ['.brand', '.brand strong', '.brand-logo-button', '.icon', '.brand-actions', '.brand-action', '.sidebar-tabs', '.sidebar-tab-entry', '.sidebar-tab', '.mobile-navigation .brand', '.mobile-navigation .brand-action', '.workspace-context', '.workspace-context .avatar', '.top-actions', '.sidebar-footer', '.mobile-navigation .sidebar-footer > .icon'];
const cssBlocks = ruleNames.flatMap(name => {
  const escaped = name.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\$&');
  const re = new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{[^}]*\\}`, 'g');
  return source.match(re) ?? [];
});
const css = `${cssBlocks.join('\n')}\n.native-mac .brand { height: var(--pane-tabbar-height); padding-left: calc(92px / var(--interface-scale,1)); padding-right: 8px; padding-top: calc(12px / var(--interface-scale,1)); gap: 4px; }\n.native-mac { --pane-tabbar-height: max(36px, calc(68px / var(--interface-scale, 1))); }\n.mobile-navigation .topbar > .top-actions { padding:0; gap:0; } .mobile-navigation .top-actions > .icon { width:44px; height:44px; }\n.sidebar { width: 320px; display:flex; flex-direction:column; }\n.side-scroll { flex:1; min-height:0; }\n.topbar { display:flex; align-items:center; height:52px; }\n.brand-full { display:inline-block; } .brand-short { display:none; }\n.brand[data-compact-wordmark="true"] .brand-full { position:absolute; visibility:hidden; display:inline-block; }\n.brand[data-compact-wordmark="true"] .brand-short { display:inline; }\n`;
const extraCss = ':root { --density-control-size:30px; --density-tabbar-inset:0px; } button { box-sizing:border-box; padding:0; border:0; } .workspace { width:320px; } .embedded .topbar { box-sizing:border-box; padding:.5em .5em 0; } .workspace-context { display:grid; place-items:center; flex:none; width:30px; padding-bottom:.5em; } .workspace-context .avatar { display:block; width:24px; height:24px; } .top-actions { display:flex; align-items:center; gap:14px; flex-shrink:0; padding-bottom:.5em; } .tabs { display:flex; flex:1; min-width:0; } .tab-picker-trigger { display:flex; align-items:center; justify-content:space-between; width:100%; padding:0 8px; border:1px solid #bbb; } .workspace.compact-tabs > .topbar { padding-block:.5em; } .workspace.compact-tabs > .topbar > .workspace-context, .workspace.compact-tabs > .topbar > .top-actions { padding-bottom:0; } .mobile-navigation .workspace > .topbar { height:52px; box-sizing:border-box; align-items:center; padding:4px; gap:4px; } .mobile-navigation .topbar > .tabs { align-self:stretch; } .mobile-navigation .topbar > .top-actions { padding:0; gap:0; } .mobile-navigation .top-actions > .icon { width:44px; height:44px; } .mobile-navigation .workspace.compact-tabs > .topbar { padding-block:4px; }';
const html = fixture.replace('<style id="app-surface-rules"></style>', `<style id="app-surface-rules">${css}${extraCss}</style>`);
const server = createServer((request, response) => {
  const path = (request.url ?? '/').split('?')[0];
  if (path === '/') return response.writeHead(200, { 'content-type': 'text/html' }).end(html);
  if (path === '/responsive-brand.js') return response.writeHead(200, { 'content-type': 'text/javascript' }).end(`${brandActionJs}\nwindow.__BRAND_QA__=responsiveBrand(document.querySelector('.brand'));`);
  return response.writeHead(404).end();
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No isolated server address');
const browser = await webkit.launch({ headless: true });
try {
  for (const profile of [
    { name: 'desktop WebKit', mobile: false, viewport: { width: 1120, height: 680 } },
    { name: 'mobile WebKit', mobile: true, viewport: { width: 390, height: 844 } },
  ]) {
    const context = await browser.newContext({ viewport: profile.viewport, isMobile: profile.mobile, hasTouch: profile.mobile });
    const page = await context.newPage();
    await page.goto(`http://127.0.0.1:${address.port}/`);
    await expect.poll(() => page.evaluate(() => Boolean(window.__BRAND_QA__))).toBe(true);
    if (profile.mobile) await page.locator('.sidebar').evaluate(node => node.parentElement.classList.add('mobile-navigation'));
    const metrics = await page.evaluate(() => {
      const rect = selector => document.querySelector(selector).getBoundingClientRect();
      const style = getComputedStyle(document.querySelector('.brand strong'));
      const brand = rect('.brand'); const label = rect('.brand strong'); const action = rect('.brand-action'); const tab = rect('.sidebar-tab');
      const terminal = rect('.top-actions .icon'); const avatar = rect('.workspace-context .avatar');
      return { action: [action.width, action.height], tab: [tab.width, tab.height], terminal: [terminal.width, terminal.height], avatar: [avatar.width, avatar.height], brandCenter: brand.y + brand.height / 2, brandCenterX: brand.x + brand.width / 2, actionCenter: action.y + action.height / 2, labelCenter: label.y + label.height / 2, labelCenterX: label.x + label.width / 2, fontSize: style.fontSize, lineHeight: style.lineHeight, opacity: style.opacity, labelFits: document.querySelector('.brand strong').scrollWidth <= document.querySelector('.brand strong').clientWidth };
    });
    const expected = profile.mobile ? 44 : 30;
    expect(metrics.action).toEqual([expected, expected]);
    expect(metrics.tab[0]).toBeGreaterThan(50);
    expect(metrics.tab[1]).toBeGreaterThanOrEqual(25);
    expect(metrics.terminal).toEqual([profile.mobile ? 44 : 30, profile.mobile ? 44 : 30]);
    expect(metrics.avatar).toEqual([24, 24]);
    expect(Math.abs(metrics.brandCenter - metrics.actionCenter)).toBeLessThanOrEqual(0.5);
    expect(metrics.fontSize).toBe('13px'); expect(metrics.lineHeight).toBe('13px'); expect(metrics.opacity).toBe('1');
    expect(metrics.labelCenter).toBeGreaterThanOrEqual(metrics.brandCenter - 1);
    expect(Math.abs(metrics.labelCenterX - metrics.brandCenterX)).toBeLessThanOrEqual(0.5);
    const gaps = await page.locator('.workspace.compact-tabs > .topbar').evaluate(node => { const bar = node.getBoundingClientRect(); const trigger = node.querySelector('.tab-picker-trigger').getBoundingClientRect(); return [trigger.top - bar.top, bar.bottom - trigger.bottom]; });
    expect(Math.abs(gaps[0] - gaps[1])).toBeLessThanOrEqual(1);
    await context.close();
    console.log(`${profile.name}: create actions ${expected}x${expected}, centered brand, labeled sidebar tabs`);
  }
  const narrow = await browser.newContext({ viewport: { width: 320, height: 640 } });
  const page = await narrow.newPage(); await page.goto(`http://127.0.0.1:${address.port}/`); await page.locator('.sidebar').evaluate(node => node.classList.add('native-mac'));
  await expect.poll(() => page.evaluate(() => Boolean(window.__BRAND_QA__))).toBe(true);
  await page.evaluate(() => document.documentElement.style.setProperty('--interface-font-ratio', '1.5'));
  await expect.poll(() => page.locator('.brand strong').evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
  await page.locator('.brand').evaluate(node => { node.querySelector('.brand-actions').remove(); window.__BRAND_QA__.update(); });
  await expect(page.locator('.brand')).not.toHaveAttribute('data-compact-wordmark');
  await page.locator('.brand').evaluate(node => { const actions = document.createElement('div'); actions.className = 'brand-actions'; actions.innerHTML = '<button class="icon brand-action">x</button><button class="icon brand-action">x</button>'; node.append(actions); node.style.width = '120px'; window.__BRAND_QA__.update(); });
  await expect.poll(() => page.locator('.brand').getAttribute('data-compact-wordmark')).toBe('true');
  await expect(page.locator('.brand-short')).toBeVisible();
  await expect(page.locator('.brand-full')).toBeHidden();
  await page.locator('.brand').evaluate(node => { node.style.width = '600px'; window.__BRAND_QA__.update(); });
  await expect.poll(() => page.locator('.brand').getAttribute('data-compact-wordmark')).toBe('false');
  const compactGaps = await page.locator('.workspace.compact-tabs > .topbar').evaluate(node => { const bar = node.getBoundingClientRect(); const trigger = node.querySelector('.tab-picker-trigger').getBoundingClientRect(); return [trigger.top - bar.top, bar.bottom - trigger.bottom]; });
  expect(Math.abs(compactGaps[0] - compactGaps[1])).toBeLessThanOrEqual(1);
  await expect(page.locator('.brand-full')).toBeVisible();
  console.log('narrow native chrome: centered brand compacts around window and create actions');
  await narrow.close();
  const footerStart = source.indexOf('<footer class="sidebar-footer"');
  const shareInFooter = footerStart >= 0 && source.indexOf('aria-label="Share workspace"', footerStart) >= footerStart && source.indexOf('aria-label="Share workspace"', footerStart) < source.indexOf('</footer>', footerStart);
  expect(shareInFooter).toBe(true);
  expect(source.includes('<ShareControl')).toBe(false); expect(source.includes('<RemoteControl')).toBe(false);
  expect(brandAction).toContain('ResizeObserver');
  expect(brandAction).toContain('compactWordmark');
  expect(brandAction).toContain('update: observe');
  expect(source).toContain('active={true} activeTooltip="Live run"');
  console.log('source: Share workspace is sidebar-footer-only; no floating ShareControl/RemoteControl markup; responsive brand action observes remounts');
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
