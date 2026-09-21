import { chromium, expect as baseExpect } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
mkdirSync(join(root, 'verification'), { recursive: true });
const harness = mkdtempSync(join(root, 'verification', 'mail-triage-harness-'));
const component = join(root, 'src/lib/components/MailTriageBatch.svelte');
const lib = join(root, 'src/lib');
writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'bridge.js'), `
let reads=0, requests=0;
export function getBridge(){return{
  async getMailDetail(){reads+=1;return requests && reads >= 3 ? {mailId:'mail-high',source:'gmail',accountLabel:'Work Gmail',from:'Pat <pat@example.com>',to:['Alex <alex@example.com>'],cc:[],subject:'Decision needed by Friday',receivedAt:1700000000000,bodyText:'Hello Alex,\\n\\nPlease approve the attached proposal by Friday.\\n\\nPat'} : null},
  async requestMailDetail(){requests+=1;return{status:'pending'}}
}}
window.__mailTest={get reads(){return reads},get requests(){return requests}};
`);
writeFileSync(join(harness, 'App.svelte'), `<script>
import MailTriageBatch from ${JSON.stringify(component)};
const batch={id:'batch',messageId:'message',taskId:'task',source:'gmail',accountLabel:'Work Gmail',queryLabel:'Unread since yesterday',createdAt:1700000000000,updatedAt:1700000300000,syncCount:2,lastAdded:1,lastUpdated:1,lastMovedToHistory:1,classifier:{mode:'jev',provider:'TypeSafe',model:'jev-test',latencyMs:22,inputTokens:120,outputTokens:12,costMicrousd:200,fallbackReason:null},items:[
{id:'mail-low',providerMessageId:'provider-low',providerThreadId:null,from:'Digest <digest@example.com>',to:['Alex'],cc:[],subject:'Weekly digest',receivedAt:1699990000000,snippet:'Your weekly newsletter.',importance:'low',importanceScore:20,intent:'newsletter',replyRequired:'no',suggestedOwner:'unclear',suggestedAction:'archive',confidence:93,rationale:'low importance',state:'history',firstSeenAt:1699990000000,lastSeenAt:1699990000000,isNew:false},
{id:'mail-high',providerMessageId:'provider-high',providerThreadId:'thread',from:'Pat <pat@example.com>',to:['Alex'],cc:[],subject:'Decision needed by Friday',receivedAt:1700000000000,snippet:'Please approve the proposal.',importance:'high',importanceScore:80,intent:'decision_needed',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'review',confidence:86,rationale:'high importance',state:'active',firstSeenAt:1700000300000,lastSeenAt:1700000300000,isNew:true}]};
const pending={...batch,id:'pending-batch',accountLabel:'Pending Gmail',queryLabel:'Latest mail',classifier:{mode:'pending',provider:'TypeSafe',model:'',latencyMs:0,inputTokens:null,outputTokens:null,costMicrousd:null,fallbackReason:null},items:batch.items.map(item=>({...item,confidence:45}))};
</script><main><MailTriageBatch {batch} taskId="task"/><MailTriageBatch batch={pending} taskId="task"/></main><style>:global(:root){--accent:#3f9d6a;--line:#d9d3c7;--panel:#fff;--paper:#f8f5ef;--soft:#eeeae2;--ink:#27231e;--muted:#776f63;--mono:monospace;--interface-font-ratio:1;--interface-font:system-ui}:global(body){margin:30px;background:var(--paper);font-family:system-ui}main{max-width:850px;margin:auto}</style>`);
writeFileSync(join(harness, 'main.js'), `import { mount } from 'svelte';import App from './App.svelte';mount(App,{target:document.querySelector('#app')});`);
writeFileSync(join(harness, 'vite.config.mjs'), `import { svelte } from '@sveltejs/vite-plugin-svelte';export default{resolve:{alias:[{find:'$lib/bridge',replacement:${JSON.stringify(join(harness, 'bridge.js'))}},{find:'$lib',replacement:${JSON.stringify(lib)}}]},plugins:[svelte()]};`);

const child = spawn(process.execPath, [join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '0'], { cwd: harness, env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
let output = '';
const url = await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output}`)), 60000);
  const read = chunk => { output += chunk.toString(); const match = output.match(/Local:\s+(http:\/\/[^\s]+)/); if (match) { clearTimeout(timer); resolve(match[1]); } };
  child.stdout.on('data', read); child.stderr.on('data', read); child.once('error', reject);
});

let browser;
try {
  const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
    || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  browser = await chromium.launch({ executablePath });
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url, { timeout: 60000 });
  const cards = page.locator('.mail-batch').first().locator('.mail-card');
  await expect(cards).toHaveCount(2);
  await expect(cards.first()).toContainText('Decision needed by Friday');
  await expect(cards.first()).toContainText('owner me');
  await expect(page.locator('.mail-batch').first()).toContainText('Actionable inbox');
  await expect(page.locator('.mail-batch').first()).toContainText('Needs action');
  await expect(page.locator('.mail-batch').first()).toContainText('No longer matching');
  await expect(page.locator('.mail-batch').first()).toContainText('1 added · 1 refreshed');
  await expect(page.locator('.mail-batch').first()).toContainText('1 moved to history');
  await expect(cards.first()).toContainText('New');
  await expect(page.getByText('Jev', { exact: true })).toBeVisible();
  await expect(page.getByText('Jev pending', { exact: true })).toBeVisible();
  await expect(page.getByText('Showing immediate provisional labels while Jev classifies this batch in the background.')).toBeVisible();
  await page.screenshot({ path: join(root, 'verification', 'mail-triage-list.png'), fullPage: true });
  await cards.first().click();
  const dialog = page.getByRole('dialog', { name: 'Email: Decision needed by Friday' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Asking this Codex agent');
  await expect(dialog.locator('pre')).toContainText('Please approve the attached proposal by Friday.');
  expect(await page.evaluate(() => window.__mailTest.requests)).toBe(1);
  expect(await page.evaluate(() => window.__mailTest.reads)).toBeGreaterThanOrEqual(2);
  await expect(dialog).toContainText('untrusted plain text');
  await page.screenshot({ path: join(root, 'verification', 'mail-triage-ui.png') });
  await dialog.getByRole('button', { name: 'Close email' }).click();
  await expect(dialog).toHaveCount(0);
  expect(errors).toEqual([]);
  console.log('Persistent mail inbox passed grouping, new/history state, sync receipt, pending/Jev attribution, click request, plain-text detail, and close checks.');
} finally {
  await browser?.close(); child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
