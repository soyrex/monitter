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
    codex: { status: 'ready', active: { label: '5-hour', usedPercent: 98, resetsAt: resetAfter(3, 6, 23) }, weekly: { label: 'Week', usedPercent: 20, resetsAt: resetAfter(7, 0, 0) } },
    claude: { status: 'stale', active: { label: 'Current session', usedPercent: 80, resetsAt: resetAfter(0, 2, 5) } },
    minimax: { status: 'ready', active: { label: 'general 5-hour', usedPercent: 25, resetsAt: resetAfter(0, 4, 30) } },
    'opencode-go': { status: 'error', message: 'Router unavailable.' },
  };
</script>
<main class="sidebar"><UsageRings {usage} expanded={true} /></main>
<style>:global(:root){--ink:#28231c;--muted:#756b5c;--panel:#f5f1e8;--line:#d8d0c2;--soft:#eee8dc;--accent:#3f9d6a;--mono:ui-monospace,monospace}:global(body){margin:24px;background:#fbf8f2}.sidebar{width:252px;height:100vh}</style>`);
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
  const page = await browser.newPage({ viewport: { width: 760, height: 1400 } });
  await page.goto(url);

  const providers = page.locator('.usage-provider');
  const resetButtons = page.locator('button.usage-reset');
  await expect(providers).toHaveCount(4);
  await expect(resetButtons).toHaveCount(4);

  const codex = providers.filter({ hasText: 'Codex' });
  await expect.poll(() => codex.innerText()).toMatch(/98% used\.\s*Resets in\s*3d 6h 23m/);
  await expect(page.locator('.usage-rings')).not.toHaveClass(/rings-only/);
  await expect(codex.locator('.usage-reset strong').first()).toHaveText('3d 6h 23m');
  expect(Number(await codex.locator('.usage-reset strong').first().evaluate(node => getComputedStyle(node).fontWeight))).toBeGreaterThanOrEqual(600);
  await expect(providers.filter({ hasText: 'MiniMax' })).toContainText('general 5-hour');
  await expect(providers.filter({ hasText: 'Claude' })).toHaveClass(/stale/);
  await expect(providers.filter({ hasText: 'OpenCode Go' })).toContainText('Router unavailable.');
  await expect(providers.filter({ hasText: 'OpenCode Go' })).toHaveClass(/error/);
  await expect(resetButtons.first()).toHaveAttribute('aria-label', /Show absolute reset dates and times for all provider windows/);

  const activeStroke = async name => providers.filter({ hasText: name }).locator('.usage-ring').evaluate(node => node.style.getPropertyValue('--ring-active-color'));
  const [redStroke, orangeStroke, greenStroke] = await Promise.all([
    activeStroke('Codex'), activeStroke('Claude'), activeStroke('MiniMax'),
  ]);
  const rgb = stroke => stroke.match(/\d+/g).map(Number);
  const [red, orange, green] = [redStroke, orangeStroke, greenStroke].map(rgb);
  expect(red[0]).toBeGreaterThan(red[1]);
  expect(orange[0]).toBeGreaterThan(orange[1]);
  expect(green[1]).toBeGreaterThan(green[0]);

  const ringValue = codex.locator('.ring-value');
  await expect(ringValue).toHaveAttribute('x', '20');
  await expect(ringValue).toHaveAttribute('y', '20');
  await expect(ringValue).toHaveAttribute('dominant-baseline', 'middle');

  await resetButtons.first().press('Enter');
  await expect(resetButtons.filter({ hasText: 'Resets ' })).toHaveCount(4);
  for (let index = 0; index < 4; index += 1) {
    await expect(resetButtons.nth(index)).toContainText('Resets ');
  }
  await expect(resetButtons.first()).toHaveAttribute('aria-label', /Show relative reset times for all provider windows/);

  await resetButtons.last().click();
  await expect(resetButtons.filter({ hasText: 'Resets in' })).toHaveCount(4);

  await page.setViewportSize({ width: 760, height: 1200 });
  await expect(page.locator('.usage-rings')).not.toHaveClass(/rings-only/);
  await page.setViewportSize({ width: 760, height: 1199 });
  await expect(page.locator('.usage-rings')).toHaveClass(/rings-only/);
  await expect(page.locator('.usage-toggle')).toBeVisible();
  await expect(codex).toHaveText(/98\s*Codex/);
  await expect(codex.locator('.usage-copy')).toBeHidden();
  await expect(page.locator('.usage-ring-label')).toHaveCount(4);
  await expect(page.locator('.usage-ring-label').nth(1)).toHaveText('Claude');
  const ringRow = await page.locator('.usage-provider').evaluateAll(nodes => nodes.map(node => {
    const rect = node.getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  }));
  expect(new Set(ringRow.map(item => Math.round(item.top))).size).toBe(1);
  expect(ringRow[0].left).toBeLessThan(ringRow[1].left);
  expect(ringRow[1].left).toBeLessThan(ringRow[2].left);
  expect(ringRow[2].left).toBeLessThan(ringRow[3].left);
  console.log('Usage switches to a four-ring provider row below 1200px window height while retaining its toggle header.');
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
