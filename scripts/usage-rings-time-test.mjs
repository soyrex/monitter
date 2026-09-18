import { chromium, webkit, expect as baseExpect } from '@playwright/test';
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
  import '@fontsource/ibm-plex-mono/700.css';
  import '@fontsource/ibm-plex-sans/400.css';
  import UsageRings from '${join(root, 'src/lib/components/UsageRings.svelte')}';
  const base = Date.now();
  const resetAfter = (days, hours, minutes) => base + (((days * 24 + hours) * 60 + minutes) * 60000);
  let compact = $state(false);
  let usage = $state({
    codex: { status: 'ready', active: { label: '5-hour', usedPercent: 98, resetsAt: resetAfter(3, 6, 23) }, weekly: { label: 'Week', usedPercent: 20, resetsAt: resetAfter(7, 0, 0) } },
    claude: { status: 'stale', active: { label: 'Current session', usedPercent: 80, resetsAt: resetAfter(0, 2, 5) } },
    minimax: { status: 'ready', active: { label: 'general 5-hour', usedPercent: 25, resetsAt: resetAfter(0, 4, 30) } },
    'opencode-go': { status: 'error', message: 'Router unavailable.' },
  });
  window.setCodexAccounts = (personalFailed = false) => {
    const accounts = [
      { key: 'personal', label: 'Personal', status: personalFailed ? 'error' : 'ready', message: personalFailed ? 'Personal unavailable' : null, active: personalFailed ? null : { label: '5-hour', usedPercent: 12 }, weekly: { label: 'Week', usedPercent: 20 } },
      { key: 'work', label: 'Work', status: 'ready', active: { label: '5-hour', usedPercent: 67, resetsAt: resetAfter(0, 2, 0) }, weekly: null },
    ];
    usage = { ...usage, codex: { ...accounts[0], accounts } };
  };
</script>
<button id="loading" onclick={() => usage = {...usage, codex: {status: 'loading'}}}>Load</button>
<button id="compact" onclick={() => compact = !compact}>Compact</button>
<main class="sidebar"><UsageRings {usage} {compact} expanded={true} /></main>
<style>:global(:root){--ink:#28231c;--muted:#756b5c;--panel:#f5f1e8;--line:#d8d0c2;--soft:#eee8dc;--accent:#3f9d6a;--mono:"IBM Plex Mono",monospace}:global(body){margin:24px;background:#fbf8f2;font-family:"IBM Plex Sans",sans-serif}.sidebar{width:252px;height:100vh}</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte'; import App from './App.svelte'; mount(App, { target: document.querySelector('#app') });`);
writeFileSync(join(harness, 'vite.config.mjs'), `import { svelte } from '@sveltejs/vite-plugin-svelte'; export default { cacheDir: './.vite', plugins: [svelte()] };`);

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
  browser = await (process.env.BROWSER === 'webkit' ? webkit : chromium).launch();
  const page = await browser.newPage({ viewport: { width: 760, height: 1400 } });
  // Headless WebKit can report a stale outerHeight after a one-pixel resize.
  // Model a window without browser chrome so breakpoint tests are deterministic.
  await page.addInitScript(() => Object.defineProperty(window, 'outerHeight', { get: () => window.innerHeight }));
  page.on('pageerror', error => console.error(error));
  await page.goto(url);
  await page.evaluate(() => document.fonts.ready);

  const providers = page.locator('.usage-provider');
  const resetButtons = page.locator('button.usage-reset');
  const uniqueSvgIds = async () => {
    const ids = await page.locator('.usage-rings svg [id]').evaluateAll(nodes => nodes.map(node => node.id));
    expect(new Set(ids).size).toBe(ids.length);
  };
  await expect(providers).toHaveCount(4);
  await expect(resetButtons).toHaveCount(4);

  const codex = providers.filter({ hasText: 'Codex' });
  await expect.poll(() => codex.innerText()).toMatch(/98% used\.\s*Resets in\s*3d 6h 23m/);
  await expect(page.locator('.usage-rings')).not.toHaveClass(/horizontal/);
  await expect(page.locator('.usage-provider-icon')).toHaveCount(4);
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
  const centered = async (outer, inner) => {
    const [a, b] = await Promise.all([outer.boundingBox(), inner.boundingBox()]);
    expect(Math.abs(a.x + a.width / 2 - b.x - b.width / 2)).toBeLessThan(0.75);
    expect(Math.abs(a.y + a.height / 2 - b.y - b.height / 2)).toBeLessThan(0.75);
  };
  await centered(codex.locator('.usage-ring'), ringValue);
  const logoBeforeName = async label => {
    const icon = await label.locator('.provider-icon').boundingBox();
    const text = await label.locator('strong, .usage-provider-name, .usage-label-text').first().boundingBox();
    expect(icon.x + icon.width).toBeLessThanOrEqual(text.x);
    expect(Math.abs(icon.y + icon.height / 2 - text.y - text.height / 2)).toBeLessThan(1);
  };


  await resetButtons.first().press('Enter');
  await expect(resetButtons.filter({ hasText: 'Resets ' })).toHaveCount(4);
  for (let index = 0; index < 4; index += 1) {
    await expect(resetButtons.nth(index)).toContainText('Resets ');
  }
  await expect(resetButtons.first()).toHaveAttribute('aria-label', /Show relative reset times for all provider windows/);

  await resetButtons.last().click();
  await expect(resetButtons.filter({ hasText: 'Resets in' })).toHaveCount(4);

  await page.setViewportSize({ width: 760, height: 700 });
  await expect(page.locator('.usage-rings')).toHaveClass(/horizontal/);
  await expect(page.locator('.usage-toggle')).toBeVisible();
  await expect(codex).toContainText(/U:\s*98%/);
  await expect(codex.locator('.usage-copy')).toBeVisible();
  await expect(codex.locator('.usage-detail-short')).toBeVisible();
  await expect(codex.locator('.usage-reset-short')).toBeVisible();
  const providerRows = await page.locator('.usage-provider').evaluateAll(nodes => nodes.map(node => {
    const rect = node.getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  }));
  expect(new Set(providerRows.map(item => Math.round(item.top))).size).toBe(4);
  expect(providerRows.every(item => item.left === providerRows[0].left)).toBe(true);
  await page.setViewportSize({ width: 760, height: 1200 });
  await expect(page.locator('.usage-rings')).not.toHaveClass(/horizontal/);
  await expect(codex.locator('.usage-copy')).toBeVisible();
  await logoBeforeName(codex.locator('.usage-heading'));
  await uniqueSvgIds();
  await page.locator('#loading').click();
  const loading = providers.first();
  const active = loading.locator('.ring-active');
  for (const time of [0, 150, 350, 600, 900]) {
    await active.evaluate((node, time) => {
      for (const animation of node.getAnimations()) { animation.pause(); animation.currentTime = time; }
    }, time);
    await centered(loading.locator('.usage-ring'), active);
  }
  await page.locator('#compact').click();
  await expect(page.locator('.usage-toggle')).toBeHidden();
  await expect(providers).toHaveCount(4);
  await centered(loading.locator('.usage-ring'), loading.locator('.ring-value'));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await active.evaluate(node => getComputedStyle(node).animationName)).toBe('none');
  await page.evaluate(() => window.setCodexAccounts());
  const accountSelector = codex.getByLabel('Codex account', { exact: true });
  await expect(accountSelector).toBeVisible();
  await expect(ringValue).toHaveText('12');
  await accountSelector.selectOption('work');
  await expect(ringValue).toHaveText('67');
  await expect(codex).toHaveAttribute('title', /67% used/);
  await expect(codex.locator('.usage-heading')).toContainText('Work');
  await expect(codex.locator('.usage-weekly')).toHaveCount(0);
  await page.evaluate(() => window.setCodexAccounts(true));
  await accountSelector.selectOption('personal');
  await expect(codex).toHaveClass(/error/);
  await expect(codex.locator('.usage-message')).toContainText('Personal unavailable');
  await accountSelector.selectOption('work');
  await expect(ringValue).toHaveText('67');
  await expect(codex).not.toHaveClass(/error/);
  console.log('Usage preserves detailed provider rows, adapts short-window details, and keeps ring geometry and reduced motion intact.');
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
