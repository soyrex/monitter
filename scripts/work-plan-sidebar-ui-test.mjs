import { chromium, expect as baseExpect } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
const surface = readFileSync(join(root, 'src/lib/components/AppSurface.svelte'), 'utf8');
expect(surface).toContain('>Task detail</button>');
expect(surface).toContain('workPlans={snapshot.workPlans?.filter(plan=>plan.taskId===selectedTask.id)}');

mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'work-plan-harness-'));
writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'App.svelte'), `<script>
  import RunSummary from ${JSON.stringify(join(root, 'src/lib/components/RunSummary.svelte'))};
  const task={id:'task-one',hostId:'local',cwd:'/project',provider:'codex',status:'running'};
  const gitStatus={repository:true,branch:'feature/task-plan',files:[],truncated:false};
  let plans=$state([
    {id:'current',taskId:'task-one',requestId:'r1',title:'Ship the dashboard',status:'active',createdAt:200,updatedAt:300,closedAt:null,summary:null,items:[
      {id:'one',title:'Map the sidebar',status:'completed',note:'Matched the Git branch card.',updatedAt:1},
      {id:'two',title:'Build the checklist',status:'in_progress',note:'Working in an isolated tree.',updatedAt:2},
      {id:'three',title:'Verify rendering',status:'pending',note:null,updatedAt:3},
      {id:'four',title:'Resolve blocker',status:'blocked',note:'Waiting for a fixture.',updatedAt:4},
      {id:'five',title:'Optional polish',status:'skipped',note:null,updatedAt:5}
    ]},
    {id:'prior',taskId:'task-one',requestId:'r0',title:'Initial investigation',status:'closed',createdAt:100,updatedAt:150,closedAt:150,summary:'Found the data path.',items:[
      {id:'old-one',title:'Inspect snapshot',status:'completed',note:null,updatedAt:100}
    ]}
  ]);
  function advance(){plans=plans.map(plan=>plan.id==='current'?{...plan,updatedAt:400,items:plan.items.map(item=>item.id==='two'?{...item,status:'completed',updatedAt:400}:item)}:plan)}
  function clear(){plans=[]}
</script>
<aside class="sidebar"><RunSummary {task} {gitStatus} workPlans={plans}/></aside>
<button class="advance" onclick={advance}>Advance plan</button>
<button class="clear" onclick={clear}>Clear plan</button>
<style>:global(:root){--accent:#3f9d6a;--accent-ink:#347955;--danger:#c96058;--line:#d9d3c7;--panel:#fff;--paper:#f8f5ef;--sidebar:#f5f1e9;--soft:#eeeae2;--ink:#27231e;--muted:#776f63;--mono:monospace;--interface-font-ratio:1;--sans:system-ui}:global(body){margin:24px;background:#e8e4dc;font-family:system-ui}.sidebar{width:292px;padding:0 12px;background:var(--sidebar);border:1px solid var(--line)}.advance{margin-top:16px}</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte';import App from './App.svelte';mount(App,{target:document.querySelector('#app')});`);
writeFileSync(join(harness, 'vite.config.mjs'), `import { svelte } from '@sveltejs/vite-plugin-svelte';export default{resolve:{alias:{'$lib':${JSON.stringify(join(root, 'src/lib'))}}},plugins:[svelte()]};`);

const child = spawn(process.execPath, [join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '0'], { cwd: harness, env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
let output = '';
let browser;
try {
  const url = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output}`)), 60000);
    const read = chunk => { output += chunk.toString(); const match = output.match(/Local:\s+(http:\/\/[^\s]+)/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    child.stdout.on('data', read); child.stderr.on('data', read); child.once('error', reject);
  });
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
  const page = await browser.newPage({ viewport: { width: 760, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url, { timeout: 60000 });
  const plan = page.getByRole('region', { name: 'Agent work plan' });
  await expect(plan).toContainText('Ship the dashboard');
  await expect(plan).toContainText('1 of 5 done · 1 in progress · 1 blocked · 1 skipped');
  for (const status of ['completed', 'in_progress', 'pending', 'blocked', 'skipped']) {
    await expect(plan.locator(`.plan-items li[data-status="${status}"]`)).toHaveCount(1);
  }
  await expect(plan.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
  const history = plan.locator('.plan-history');
  await expect(history).toContainText('Earlier plans');
  await history.locator('summary').click();
  await expect(history).toContainText('Initial investigation');
  await expect(history).toContainText('Found the data path.');
  const geometry = await page.evaluate(() => {
    const bounds = selector => document.querySelector(selector).getBoundingClientRect();
    const git = bounds('.git'), plan = bounds('.work-plan'), processes = bounds('.processes'), sidebar = bounds('.sidebar');
    return { gitBottom: git.bottom, planTop: plan.top, planBottom: plan.bottom, processesTop: processes.top, planWidth: plan.width, sidebarWidth: sidebar.width, overflowing: document.querySelector('.sidebar').scrollWidth > document.querySelector('.sidebar').clientWidth };
  });
  expect(geometry.planTop).toBeGreaterThanOrEqual(geometry.gitBottom);
  expect(geometry.processesTop).toBeGreaterThanOrEqual(geometry.planBottom);
  expect(geometry.planWidth).toBeLessThan(geometry.sidebarWidth);
  expect(geometry.overflowing).toBe(false);
  await page.screenshot({ path: join(root, 'verification', 'work-plan-sidebar.png') });
  await page.locator('.sidebar').evaluate(element => element.style.width = '240px');
  expect(await page.locator('.sidebar').evaluate(element => element.scrollWidth > element.clientWidth)).toBe(false);
  await page.getByRole('button', { name: 'Advance plan' }).click();
  await expect(plan).toContainText('2 of 5 done');
  await expect(plan.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '2');
  await page.getByRole('button', { name: 'Clear plan' }).click();
  await expect(plan).toContainText('No work plan published for this chat yet.');
  expect(errors).toEqual([]);
  console.log('Work-plan sidebar UI assertions passed.');
} finally {
  await browser?.close();
  child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
