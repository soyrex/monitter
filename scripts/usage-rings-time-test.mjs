import { chromium, expect as baseExpect } from '@playwright/test';
import { once } from 'node:events';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawn } from 'node:child_process';

const expect = baseExpect.configure({ timeout: 30_000 });
const root = process.cwd();
mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'usage-rings-harness-'));

writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'App.svelte'), `<script>
  import UsageRings from '${join(root, 'src/lib/components/UsageRings.svelte')}';
  const base = Date.now();
  const resetAfter = (days, hours, minutes) => base + (((days * 24 + hours) * 60 + minutes) * 60000);
  const usage = {
    codex: { status: 'ready', active: { label: '5-hour', usedPercent: 92, resetsAt: resetAfter(3, 6, 23) }, weekly: { label: 'Week', usedPercent: 20, resetsAt: resetAfter(7, 0, 0) } },
    claude: { status: 'stale', active: { label: 'Current session', usedPercent: 10, resetsAt: resetAfter(0, 2, 5) } },
    minimax: { status: 'ready', active: { label: 'general 5-hour', usedPercent: 92, resetsAt: resetAfter(0, 4, 30) } },
    'opencode-go': { status: 'error', message: 'Router unavailable.' },
  };
</script>
<UsageRings {usage} expanded={true} />
<style>:global(:root){--ink:#28231c;--muted:#756b5c;--panel:#f5f1e8;--line:#d8d0c2;--soft:#eee8dc;--accent:#3f9d6a;--mono:ui-monospace,monospace}:global(body){margin:24px;background:#fbf8f2}</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte'; import App from './App.svelte'; mount(App, { target: document.querySelector('#app') });`);
writeFileSync(join(harness, 'vite.config.mjs'), `import { svelte } from '@sveltejs/vite-plugin-svelte'; export default { plugins: [svelte()] };`);

const child = spawn(process.execPath, [join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '0'], {
  cwd: harness,
  env: { ...process.env, NO_COLOR: '1' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '';
const url = await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output}`)), 60_000);
  const read = chunk => {
    output += chunk.toString();
    const match = output.match(/Local:\s+(http:\/\/[^\s]+)/);
    if (match) {
      clearTimeout(timer);
      resolve(match[1]);
    }
  };
  child.stdout.on('data', read);
  child.stderr.on('data', read);
  child.once('error', reject);
});

let browser;
try {
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 760, height: 700 } });
  await page.goto(url);

  const providers = page.locator('.usage-provider');
  const resetButtons = page.locator('button.usage-reset');
  await expect(providers).toHaveCount(4);
  await expect(resetButtons).toHaveCount(4);

  const codex = providers.filter({ hasText: 'Codex' });
  await expect(codex).toContainText('92% used. Resets in 3d 6h 23m');
  await expect(codex.locator('.usage-reset strong').first()).toHaveText('3d 6h 23m');
  expect(Number(await codex.locator('.usage-reset strong').first().evaluate(node => getComputedStyle(node).fontWeight))).toBeGreaterThanOrEqual(600);
  await expect(providers.filter({ hasText: 'MiniMax' })).toContainText('general 5-hour');
  await expect(providers.filter({ hasText: 'Claude' })).toHaveClass(/stale/);
  await expect(providers.filter({ hasText: 'OpenCode Go' })).toContainText('Router unavailable.');
  await expect(providers.filter({ hasText: 'OpenCode Go' })).toHaveClass(/error/);
  await expect(resetButtons.first()).toHaveAttribute('aria-label', /Show absolute reset dates and times for all provider windows/);

  await resetButtons.first().press('Enter');
  await expect(resetButtons.filter({ hasText: 'Resets in' })).toHaveCount(0);
  for (let index = 0; index < 4; index += 1) {
    await expect(resetButtons.nth(index)).toContainText('Resets ');
  }
  await expect(resetButtons.first()).toHaveAttribute('aria-label', /Show relative reset times for all provider windows/);

  await resetButtons.last().click();
  await expect(resetButtons.filter({ hasText: 'Resets in' })).toHaveCount(4);
  console.log('Usage rings show bold relative resets, toggle all times together, retain status states, and label MiniMax as 5-hour.');
} finally {
  await browser?.close();
  if (child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit');
    child.kill('SIGTERM');
    await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 2_000))]);
    if (child.exitCode === null && child.signalCode === null) {
      const killed = once(child, 'exit');
      child.kill('SIGKILL');
      await killed;
    }
  }
  rmSync(harness, { recursive: true, force: true });
}
