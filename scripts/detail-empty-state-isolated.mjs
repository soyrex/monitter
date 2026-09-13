import { webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const stylesheet = source.slice(source.lastIndexOf('<style>') + 7, source.lastIndexOf('</style>'));
const rule = stylesheet.match(/\.detail-empty\s*\{[^}]+\}/)?.[0];
const approvalRule = stylesheet.match(/\.approval-history-panel\s*>\s*\.detail-empty\s*\{[^}]+\}/)?.[0];
if (!rule || !approvalRule) throw new Error('Missing shared detail empty-state CSS');
const html = `<!doctype html><style>:root{--muted:#777;--interface-font-ratio:1.5}${rule}${approvalRule}</style>
  <aside><section><h3>COLLABORATION</h3><p class="detail-empty">No routed agent messages or delegations yet.</p></section>
  <section><h3>DELEGATED TASKS</h3><p class="detail-empty">Delegated tasks will be linked here.</p></section>
  <section class="approval-history-panel"><h3>APPROVALS</h3><p class="detail-empty">No resolved approvals for this chat.</p></section></aside>`;
const server = createServer((_, response) => response.end(html));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No test server address');
const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${address.port}`);
  const states = page.locator('.detail-empty');
  await expect(states).toHaveCount(3);
  for (const state of await states.all()) {
    const style = await state.evaluate(node => ({ size: getComputedStyle(node).fontSize, color: getComputedStyle(node).color, margin: getComputedStyle(node).margin }));
    expect(style).toEqual({ size: '15.75px', color: 'rgb(119, 119, 119)', margin: '0px' });
  }
  console.log('WebKit: collaboration, delegated-task and approval empty states share compact secondary styling.');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
