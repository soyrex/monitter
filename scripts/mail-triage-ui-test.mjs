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
  async getMailDetail(){reads+=1;return requests ? {mailId:'mail-high',source:'gmail',accountLabel:'Work Gmail',from:'Pat <pat@example.com>',to:['Alex <alex@example.com>'],cc:[],subject:'Decision needed by Friday',receivedAt:1700000000000,bodyText:'Hello Alex,\\n\\nPlease approve the attached proposal by Friday.\\n\\nPat'} : null},
  async requestMailDetail(){requests+=1;return{status:'ready'}}
}}
window.__mailTest={get reads(){return reads},get requests(){return requests}};
`);
writeFileSync(join(harness, 'App.svelte'), `<script>
import MailTriageBatch from ${JSON.stringify(component)};
const batch={id:'batch',messageId:'message',taskId:'task',source:'gmail',accountLabel:'Work Gmail',queryLabel:'Unread since yesterday',createdAt:1700000000000,updatedAt:1700000300000,syncCount:2,lastAdded:1,lastUpdated:1,lastMovedToHistory:1,classifier:{mode:'jev',provider:'TypeSafe',model:'jev-test',latencyMs:22,inputTokens:120,outputTokens:12,costMicrousd:200,fallbackReason:null},items:[
{id:'mail-low',providerMessageId:'provider-low',providerThreadId:null,from:'Digest <digest@example.com>',to:['Alex'],cc:[],subject:'Weekly digest',receivedAt:1699990000000,snippet:'Your weekly newsletter.',importance:'low',importanceScore:20,intent:'newsletter',replyRequired:'no',suggestedOwner:'unclear',suggestedAction:'archive',confidence:90,classificationMetrics:{importanceConfidence:95,intentConfidence:94,replyConfidence:97,ownerConfidence:90,actionConfidence:93},rationale:'low importance',state:'history',firstSeenAt:1699990000000,lastSeenAt:1699990000000,isNew:false},
{id:'mail-high',providerMessageId:'provider-high',providerThreadId:'thread',from:'Pat <pat@example.com>',to:['Alex'],cc:[],subject:'Decision needed by Friday',receivedAt:1700000000000,snippet:'Please approve the proposal.',importance:'high',importanceScore:80,intent:'decision_needed',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'review',confidence:86,inboxFit:{workRelated:91,requiresMyAction:88,excludedBulkOrAutomated:4},classificationMetrics:{importanceConfidence:92,intentConfidence:88,replyConfidence:91,ownerConfidence:89,actionConfidence:86},rationale:'high importance',state:'active',firstSeenAt:1700000300000,lastSeenAt:1700000300000,isNew:true},
{id:'excluded-owner',providerMessageId:'provider-owner',providerThreadId:null,from:'Sam <sam@example.com>',to:['Alex'],cc:[],subject:'Not assigned to me',receivedAt:1699991000000,snippet:'Someone else owns this.',importance:'high',importanceScore:78,intent:'action_request',replyRequired:'yes',suggestedOwner:'team',suggestedAction:'reply',confidence:90,inboxFit:{workRelated:90,requiresMyAction:90,excludedBulkOrAutomated:0},classificationMetrics:{},rationale:'',state:'active',firstSeenAt:1699991000000,lastSeenAt:1699991000000,isNew:true},
{id:'excluded-bulk',providerMessageId:'provider-bulk',providerThreadId:null,from:'Updates <updates@example.com>',to:['Alex'],cc:[],subject:'Automated update',receivedAt:1699992000000,snippet:'Automated notice.',importance:'high',importanceScore:77,intent:'action_request',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'track',confidence:90,inboxFit:{workRelated:90,requiresMyAction:90,excludedBulkOrAutomated:31},classificationMetrics:{},rationale:'',state:'active',firstSeenAt:1699992000000,lastSeenAt:1699992000000,isNew:true},
{id:'excluded-intent',providerMessageId:'provider-intent',providerThreadId:null,from:'Updates <updates@example.com>',to:['Alex'],cc:[],subject:'Transactional notice',receivedAt:1699993000000,snippet:'Your receipt.',importance:'high',importanceScore:76,intent:'transactional',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'reply',confidence:90,inboxFit:{workRelated:90,requiresMyAction:90,excludedBulkOrAutomated:0},classificationMetrics:{},rationale:'',state:'active',firstSeenAt:1699993000000,lastSeenAt:1699993000000,isNew:true},
{id:'excluded-score',providerMessageId:'provider-score',providerThreadId:null,from:'Pat <pat@example.com>',to:['Alex'],cc:[],subject:'Weak action score',receivedAt:1699994000000,snippet:'Maybe review.',importance:'high',importanceScore:75,intent:'decision_needed',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'review',confidence:90,inboxFit:{workRelated:90,requiresMyAction:69,excludedBulkOrAutomated:0},classificationMetrics:{},rationale:'',state:'active',firstSeenAt:1699994000000,lastSeenAt:1699994000000,isNew:true}]};
const pending={...batch,id:'pending-batch',accountLabel:'Pending Gmail',queryLabel:'Latest mail',classifier:{mode:'pending',provider:'TypeSafe',model:'',latencyMs:0,inputTokens:null,outputTokens:null,costMicrousd:null,fallbackReason:null},items:batch.items.map(item=>({...item,confidence:45}))};
const fallback={...batch,id:'fallback-batch',classifier:{...batch.classifier,mode:'fallback',fallbackReason:'fixture fallback'}};
const legacy={...batch,id:'legacy-batch',classifier:{...batch.classifier,mode:'legacy'}};
const fullBatch={...batch,id:'full-batch',items:Array.from({length:12},(_,index)=>({...batch.items.find(item=>item.id==='mail-high'),id:'full-'+index,providerMessageId:'provider-full-'+index,subject:'Actionable message '+(index+1),receivedAt:1700000000000-index*60000,isNew:index<3}))};
</script><main><MailTriageBatch {batch} taskId="task"/><MailTriageBatch batch={pending} taskId="task"/><MailTriageBatch batch={fallback} taskId="task"/><MailTriageBatch batch={legacy} taskId="task"/><div class="screen"><MailTriageBatch batch={fullBatch} taskId="task" fullHeight/></div></main><style>:global(:root){--accent:#3f9d6a;--line:#d9d3c7;--panel:#fff;--paper:#f8f5ef;--soft:#eeeae2;--ink:#27231e;--muted:#776f63;--mono:monospace;--interface-font-ratio:1;--interface-font:system-ui}:global(body){margin:30px;background:var(--paper);font-family:system-ui}main{max-width:850px;margin:auto}.screen{height:520px;margin-top:30px}</style>`);
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
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText('Decision needed by Friday');
  await expect(cards.first()).toContainText('owner me');
  await expect(page.locator('.mail-batch').first()).toContainText('Actionable inbox');
  await expect(page.locator('.mail-batch').first()).toContainText('Needs action');
  await expect(page.locator('.mail-batch').first().locator('.mail-history')).toHaveCount(0);
  await expect(page.locator('.mail-batch').first()).not.toContainText('Not assigned to me');
  await expect(page.locator('.mail-batch').first()).not.toContainText('Automated update');
  await expect(page.locator('.mail-batch').first()).not.toContainText('Transactional notice');
  await expect(page.locator('.mail-batch').first()).not.toContainText('Weak action score');
  await expect(page.locator('.mail-batch').first()).toContainText('1 added · 1 refreshed');
  await expect(page.locator('.mail-batch').first()).not.toContainText('moved to history');
  await expect(cards.first()).toContainText('New');
  const metricsButton = cards.first().getByRole('button', { name: 'Show Jev classifications for Decision needed by Friday' });
  await metricsButton.click();
  const metrics = page.getByRole('dialog', { name: 'Jev classifications for Decision needed by Friday' });
  await expect(metrics).toBeVisible();
  await expect(metrics.locator('.classification-list strong').nth(0)).toHaveText('high');
  await expect(metrics.locator('.classification-list em').nth(0)).toHaveText('92%');
  await expect(metrics.locator('.classification-list strong').nth(1)).toHaveText('decision needed');
  await expect(metrics.locator('.classification-list em').nth(4)).toHaveText('86%');
  await expect(metrics).toContainText('Overall confidence');
  await expect(metrics).toContainText('80/100');
  await expect(metrics).toContainText('Work related');
  await expect(metrics).toContainText('91%');
  await expect(metrics).toContainText('TypeSafe · jev-test');
  expect(await page.evaluate(() => window.__mailTest.requests)).toBe(0);
  await page.waitForTimeout(200);
  await page.screenshot({ path: join(root, 'verification', 'mail-triage-classification.png') });
  await metrics.getByRole('button', { name: 'Close Jev classifications' }).click();
  await expect(page.getByText('Jev', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('Jev pending', { exact: true })).toBeVisible();
  const pendingInbox = page.locator('.mail-batch').nth(1);
  await expect(pendingInbox.getByRole('status').filter({ hasText: 'Waiting for Jev classification' })).toBeVisible();
  await expect(pendingInbox.locator('.mail-card')).toHaveCount(0);
  const fallbackInbox = page.locator('.mail-batch').nth(2);
  await expect(fallbackInbox.getByRole('status').filter({ hasText: 'Jev classification is unavailable' })).toBeVisible();
  await expect(fallbackInbox.locator('.mail-card')).toHaveCount(0);
  const legacyInbox = page.locator('.mail-batch').nth(3);
  await expect(legacyInbox.getByRole('status').filter({ hasText: 'Jev classification is unavailable' })).toBeVisible();
  await expect(legacyInbox.locator('.mail-card')).toHaveCount(0);
  const fullInbox = page.locator('.mail-batch.full-height');
  await expect(fullInbox).toHaveCount(1);
  const fullHeight = await fullInbox.evaluate(element => element.getBoundingClientRect().height);
  expect(Math.round(fullHeight)).toBe(520);
  const scrollOwnsOverflow = await fullInbox.locator('.mail-scroll').evaluate(element => element.scrollHeight > element.clientHeight && getComputedStyle(element).overflowY === 'auto');
  expect(scrollOwnsOverflow).toBe(true);
  await page.screenshot({ path: join(root, 'verification', 'mail-triage-list.png'), fullPage: true });
  await fullInbox.locator('.mail-card-open').first().click();
  const detail = page.getByRole('region', { name: 'Email: Actionable message 1' });
  await expect(detail).toBeVisible();
  await expect(page.locator('.mail-overlay')).toHaveCount(0);
  await expect(detail.locator('pre')).toContainText('Please approve the attached proposal by Friday.');
  await detail.evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  const inboxBox = await fullInbox.boundingBox();
  const detailBox = await detail.boundingBox();
  const contentBox = await fullInbox.locator('.mail-content').boundingBox();
  expect(inboxBox).not.toBeNull();
  expect(detailBox).not.toBeNull();
  expect(contentBox).not.toBeNull();
  expect(Math.abs(detailBox.x - contentBox.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(detailBox.width - contentBox.width)).toBeLessThanOrEqual(1);
  expect(Math.abs(detailBox.y - contentBox.y)).toBeLessThanOrEqual(1);
  expect(detailBox.y + detailBox.height).toBeLessThanOrEqual(inboxBox.y + inboxBox.height + 1);
  await expect(fullInbox.locator('.mail-scroll')).toHaveCount(0);
  await expect(detail.locator('.expanded-row')).toContainText('Actionable message 1');
  expect(await page.evaluate(() => window.__mailTest.requests)).toBe(1);
  expect(await page.evaluate(() => window.__mailTest.reads)).toBe(2);
  await expect(detail).toContainText('untrusted plain text');
  await page.screenshot({ path: join(root, 'verification', 'mail-triage-ui.png') });
  await detail.getByRole('button', { name: 'Back to inbox' }).click();
  await expect(detail).toHaveCount(0);
  await expect(fullInbox.locator('.mail-scroll')).toBeVisible();
  expect(errors).toEqual([]);
  console.log('Mail inbox passed strict Jev inclusion/exclusion, pending/fallback/legacy suppression, Jev attribution, native detail read, plain-text detail, and back checks.');
} finally {
  await browser?.close(); child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
