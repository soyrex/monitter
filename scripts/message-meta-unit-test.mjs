import { chromium, webkit, expect as baseExpect } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'message-meta-harness-'));
writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'App.svelte'), `<script>import MessageMeta from '${join(root, 'src/lib/components/MessageMeta.svelte')}';</script>
<main><section class="desktop"><h2>Desktop / web</h2><div class="message"><MessageMeta name="You" createdAt={1700000000000}><span class="status">Sent</span></MessageMeta><div class="message-body">A compact message with its timestamp beside the name.</div></div></section><section class="android"><h2>Android</h2><div class="message"><MessageMeta name="You" createdAt={1700000000000}/><div class="message-body">A compact message with its timestamp beside the name.</div></div></section><section class="long"><MessageMeta name="A very long agent name that must truncate gracefully without hiding the timestamp" createdAt={1700000000000}>{#snippet avatar()}<span class="avatar">A</span>{/snippet}<span class="status">Sent</span></MessageMeta></section><section class="invalid"><MessageMeta name="Missing timestamp" createdAt={NaN}/></section></main>
<style>
  :global(:root){--ink:#28231c;--muted:#756b5c;--paper:#fbf8f2;--panel:#f5f1e8}
  :global(:root.dark){--ink:#eeece7;--muted:#aaa59b;--paper:#191918;--panel:#272725}
  :global(body){margin:24px;font:14px system-ui;background:var(--paper);color:var(--ink)}
  main{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:24px}
  h2{font-size:14px;font-weight:500;color:var(--muted)}
  .message{padding:12px 14px;border-radius:10px;background:var(--panel)}
  .message-body{font:calc(13px * var(--interface-font-ratio,1))/1.6 system-ui;margin:0}
  .android .message-body{font-size:calc(16px * var(--interface-font-ratio,1))}
  .long{width:280px}.avatar{width:20px;height:20px;flex:none;border-radius:5px;background:#3f9d6a;display:grid;place-items:center}
  .status{margin-left:auto;flex:none;font-size:calc(9px * var(--interface-font-ratio,1));font-weight:400;color:var(--muted)}
</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte'; import App from './App.svelte'; mount(App, { target: document.querySelector('#app') });`);
writeFileSync(join(harness, 'vite.config.mjs'), `import { svelte } from '@sveltejs/vite-plugin-svelte'; export default { plugins: [svelte()] };`);

const child = spawn(process.execPath, [join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '0'], { cwd: harness, env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
let output = '';
const url = await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output}`)), 60000);
  const read = chunk => { output += chunk.toString(); const match = output.match(/Local:\s+(http:\/\/[^\s]+)/); if (match) { clearTimeout(timer); resolve(match[1]); } };
  child.stdout.on('data', read); child.stderr.on('data', read); child.once('error', reject);
});

let browser;
try {
  browser = await (process.argv.includes('--webkit') ? webkit : chromium).launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  page.on('pageerror', error => console.error(`harness page error: ${error.message}`));
  page.on('console', message => { if (message.type() === 'error') console.error(`harness console: ${message.text()}`); });
  await page.goto(url, { timeout: 60000 });
  await expect(page.locator('.message-meta')).toHaveCount(4);
  const metrics = await page.evaluate(() => [...document.querySelectorAll('.message')].map(message => {
    const meta = message.querySelector('.message-meta'), author = meta.querySelector('.message-author'), time = meta.querySelector('time'), body = message.querySelector('.message-body');
    const style = element => getComputedStyle(element), m = meta.getBoundingClientRect(), a = author.getBoundingClientRect(), t = time.getBoundingClientRect(), b = body.getBoundingClientRect();
    return { authorSize: parseFloat(style(author).fontSize), timeSize: parseFloat(style(time).fontSize), bodySize: parseFloat(style(body).fontSize), lineHeight: parseFloat(style(author).lineHeight), gap: b.top - m.bottom, sameLine: Math.abs(a.top - t.top) < 2, datetime: time.getAttribute('datetime'), text: time.textContent.trim(), overflow: author.scrollWidth > author.clientWidth };
  }));
  for (const [label, item] of [['desktop', metrics[0]], ['android', metrics[1]]]) {
    expect(item.authorSize).toBe(11); expect(item.timeSize).toBe(10);
    expect(item.datetime, `${label} ISO timestamp`).toBe('2023-11-14T22:13:20.000Z');
    expect(item.text, `${label} formatted timestamp`).toMatch(/^\d{1,2}:\d{2}/);
    expect(item.timeSize).toBeLessThan(item.authorSize); expect(item.bodySize).toBeGreaterThan(item.authorSize);
    expect(item.lineHeight).toBeCloseTo(item.authorSize * 1.3, 1); expect(item.gap).toBeGreaterThanOrEqual(0); expect(item.gap).toBeLessThanOrEqual(4.5); expect(item.sameLine).toBe(true);
  }
  expect(await page.locator('.long .message-author').evaluate(author => author.scrollWidth > author.clientWidth), 'long author ellipsis').toBe(true);
  await expect(page.locator('.long .message-meta > .avatar')).toHaveCount(1);
  await expect(page.locator('.invalid time')).toHaveCount(0);
  await expect(page.locator('.desktop .message-meta')).toHaveCSS('color', 'rgb(40, 35, 28)');
  await page.evaluate(() => { document.documentElement.style.setProperty('--interface-font-ratio', '1.5'); document.documentElement.classList.add('dark'); });
  await expect(page.locator('.desktop .message-meta')).toHaveCSS('color', 'rgb(238, 236, 231)');
  await expect(page.locator('.desktop time')).toHaveCSS('color', 'rgb(170, 165, 155)');
  const scaled = await page.locator('.desktop .message-meta').evaluate(meta => ({ author: parseFloat(getComputedStyle(meta.querySelector('.message-author')).fontSize), time: parseFloat(getComputedStyle(meta.querySelector('time')).fontSize) }));
  expect(scaled.author).toBeCloseTo(16.5, 1); expect(scaled.time).toBeCloseTo(15, 1);
  await page.screenshot({ path: 'verification/message-meta-unit.png' });
  console.log('MessageMeta unit layout passed desktop/Android, light/dark, timestamps, gap, same-line, ellipsis, and 150% scaling checks.');
} finally {
  await browser?.close(); child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
