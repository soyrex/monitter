import { chromium, expect as baseExpect } from '@playwright/test';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'response-metadata-harness-'));
writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'App.svelte'), `<script>import ResponseMetadata from '${join(root, 'src/lib/components/ResponseMetadata.svelte')}'; const metadata={model:'gpt-5.6-terra',inputTokens:1234,outputTokens:321,jevRationale:'The task required repository edits and test execution.',requestedModel:'gpt-5.6-terra',requestedEffort:'high',confidence:0.93,routeApplied:true,applicationError:null};</script><main><div class="bubble">Finished the implementation.</div><ResponseMetadata {metadata}/></main><style>:global(:root){--ink:#28231c;--muted:#756b5c;--panel:#f5f1e8;--line:#ddd4c7;--mono:ui-monospace,monospace;--interface-font:system-ui}:global(body){margin:24px;background:#fbf8f2;color:var(--ink)}.bubble{max-width:420px;padding:12px;border-radius:9px;background:var(--panel)}</style>`);
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
  const systemChrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  browser = await chromium.launch(existsSync(systemChrome) ? { executablePath: systemChrome } : {});
  const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
  await page.goto(url, { timeout: 60000 });
  const summary = page.getByLabel('Response details for gpt-5.6-terra');
  await expect(summary).toHaveText('gpt-5.6-terra');
  await expect(summary.locator('svg')).toHaveCount(1);
  await expect(page.locator('.response-details')).not.toBeVisible();
  await summary.click();
  await expect(page.locator('.response-details')).toBeVisible();
  await expect(page.locator('.response-details')).toContainText('1,555 total');
  await expect(page.locator('.response-details')).toContainText('1,234');
  await expect(page.locator('.response-details')).toContainText('321');
  await expect(page.locator('.response-details')).toContainText('93%');
  await expect(page.locator('.response-details')).toContainText('Jev applied this route.');
  await expect(page.locator('.response-details')).toContainText('The task required repository edits and test execution.');
  console.log('Response metadata stays compact and discloses authoritative token and Jev routing detail.');
} finally {
  await browser?.close(); child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
