import { chromium, expect as baseExpect } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const expect = baseExpect.configure({ timeout: 30000 });
const root = process.cwd();
const harness = mkdtempSync(join(root, 'verification', 'native-mail-inbox-harness-'));
const component = join(root, 'src/lib/components/NativeMailInbox.svelte');
const lib = join(root, 'src/lib');
mkdirSync(join(root, 'verification'), { recursive: true });
writeFileSync(join(harness, 'index.html'), '<div id="app"></div><script type="module" src="/main.js"></script>');
writeFileSync(join(harness, 'bridge.js'), `
let requests=0, refreshes=[];
const accounts=[
 {id:'account-b',provider:'gmail',email:'b@example.com',label:'B inbox',agentIds:['agent-a'],connectedAt:1},
 {id:'account-a',provider:'gmail',email:'a@example.com',label:'A inbox',agentIds:['agent-a'],connectedAt:1},
 {id:'account-empty',provider:'gmail',email:'empty@example.com',label:'Empty inbox',agentIds:['agent-a'],connectedAt:1},
 {id:'unassigned',provider:'gmail',email:'other@example.com',label:'Other',agentIds:['agent-b'],connectedAt:1}
];
export function getBridge(){return{
 async getMailConfig(){return{revision:'r1',clientId:'client',clientSecretConfigured:false,accounts}},
 async refreshMailInbox(taskId,accountId,query){refreshes.push({taskId,accountId,query})},
 async getMailDetail(){return requests ? {mailId:'mail-b',source:'gmail',accountLabel:'b@example.com',from:'Pat <pat@example.com>',to:['Alex'],cc:[],subject:'Native Gmail detail',receivedAt:1700000000000,bodyText:'Body retrieved from native Gmail.'} : null},
 async requestMailDetail(){requests+=1;return{status:'ready'}}
}};
window.__nativeMailTest={get refreshes(){return refreshes},get requests(){return requests}};
`);
const nativeBatch = (id, accountId, subject, cardId = id) => ({
  id, messageId:`msg-${id}`,taskId:'task-native',source:'gmail',nativeAccountId:accountId,accountLabel:accountId === 'account-b' ? 'B inbox' : 'A inbox',queryLabel:'in:inbox',createdAt:1700000000000,updatedAt:1700000001000,syncCount:1,lastAdded:1,lastUpdated:0,lastMovedToHistory:0,
  classifier:{mode:'jev',provider:'TypeSafe',model:'test',latencyMs:1,inputTokens:1,outputTokens:1,costMicrousd:1,fallbackReason:null},
  items:[{id:cardId,providerMessageId:`provider-${id}`,providerThreadId:null,from:'Pat <pat@example.com>',to:['Alex'],cc:[],subject,receivedAt:1700000000000,snippet:'A native Gmail message.',importance:'high',importanceScore:80,intent:'decision_needed',replyRequired:'yes',suggestedOwner:'me',suggestedAction:'review',confidence:90,classificationMetrics:null,rationale:'test',state:'active',firstSeenAt:1700000000000,lastSeenAt:1700000000000,isNew:true}]
});
writeFileSync(join(harness, 'App.svelte'), `<script>
import NativeMailInbox from ${JSON.stringify(component)};
const inboxes=[${JSON.stringify(nativeBatch('batch-b','account-b','Selected account native message'))},${JSON.stringify(nativeBatch('batch-a','account-a','Other account native message'))},{...${JSON.stringify(nativeBatch('batch-legacy','account-b','Legacy agent-fed message'))},nativeAccountId:null}];
</script><main><NativeMailInbox taskId="task-native" agentId="agent-a" {inboxes}/><NativeMailInbox taskId="task-empty" agentId="agent-none" inboxes={[]}/></main><style>:global(:root){--accent:#3f9d6a;--line:#d9d3c7;--panel:#fff;--paper:#f8f5ef;--soft:#eeeae2;--ink:#27231e;--muted:#776f63;--mono:monospace;--interface-font-ratio:1;--interface-font:system-ui}:global(body){margin:24px;background:var(--paper);font-family:system-ui}main{max-width:900px;margin:auto;display:grid;gap:24px}</style>`);
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
  const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  browser = await chromium.launch({ executablePath });
  const page = await browser.newPage({ viewport: { width: 1100, height: 850 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url, { timeout: 60000 });
  const mainInbox = page.locator('.native-mail-inbox').first();
  await expect(mainInbox.getByLabel('Gmail account')).toHaveValue('account-b');
  await expect(mainInbox.getByLabel('Gmail account').locator('option')).toHaveCount(3);
  await expect(mainInbox).toContainText('Selected account native message');
  await expect(mainInbox).not.toContainText('Other account native message');
  await expect(mainInbox).not.toContainText('Legacy agent-fed message');
  await expect(page.locator('.native-mail-inbox').nth(1)).toContainText('No Gmail account is assigned to this agent.');

  await mainInbox.getByLabel('Gmail account').selectOption('account-empty');
  await expect(mainInbox).toContainText('No messages loaded for empty@example.com.');
  await mainInbox.getByLabel('Gmail account').selectOption('account-a');
  const query = mainInbox.getByLabel('Gmail search query');
  await query.fill('in:inbox newer_than:7d');
  await mainInbox.getByRole('button', { name: 'Refresh' }).click();
  await expect(mainInbox.locator('.native-mail-receipt')).toContainText('Gmail inbox refreshed.');
  expect(await page.evaluate(() => window.__nativeMailTest.refreshes)).toEqual([{ taskId:'task-native',accountId:'account-a',query:'in:inbox newer_than:7d' }]);

  await mainInbox.getByLabel('Gmail account').selectOption('account-b');
  await mainInbox.getByRole('button', { name: /Open Selected account native message/ }).click();
  const detail = mainInbox.getByRole('region', { name: 'Email: Selected account native message' });
  await expect(detail.locator('pre')).toContainText('Body retrieved from native Gmail.');
  expect(await page.evaluate(() => window.__nativeMailTest.requests)).toBe(1);
  expect(errors).toEqual([]);
  console.log('Native inbox passed assigned-account filtering, account selection, exact native refresh arguments, empty assignment guidance, legacy batch hiding, and direct detail retrieval.');
} finally {
  await browser?.close(); child.kill('SIGTERM');
  rmSync(harness, { recursive: true, force: true });
}
