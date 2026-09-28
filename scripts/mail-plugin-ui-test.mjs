// Focused mocked test for Mail settings, OAuth state, and per-account grants.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { createServer } from 'node:http';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium, expect as baseExpect } from '@playwright/test';

const expect = baseExpect.configure({ timeout: 7000 });
const root = process.cwd();
const modules = existsSync(join(root, 'node_modules')) ? join(root, 'node_modules') : resolve(root, '../monitter/node_modules');
const temp = await mkdtemp(join(tmpdir(), 'monitter-mail-ui-'));
let server, browser;
const config = {
  revision: 'r1', clientId: 'client.apps.googleusercontent.com', clientSecretConfigured: true,
  accounts: [{ id: 'gmail-1', provider: 'gmail', email: 'one@gmail.com', label: 'one@gmail.com', agentIds: [], connectedAt: 1700000000000 }],
};

async function bundle() {
  const cfg = join(temp, 'rolldown.config.mjs'), out = join(temp, 'bundle.js');
  await writeFile(cfg, `import { existsSync } from 'node:fs';import { resolve } from 'node:path';import compiler from ${JSON.stringify(join(modules, 'svelte/compiler/index.js'))};const root=${JSON.stringify(root)},fixture=resolve(root,'scripts/fixtures');export default {input:resolve(fixture,'mail-settings-entry.js'),output:{file:${JSON.stringify(out)},format:'es'},plugins:[{name:'svelte-focused',resolveId(id){if(id==='@lucide/svelte')return resolve(fixture,'extensions-settings-lucide-stub.js');if(id.startsWith('$lib/')){const file=resolve(root,'src/lib',id.slice(5));return existsSync(file)?file:file+'.ts';}},transform(code,id){if(id.endsWith('.svelte'))return {code:compiler.compile(code,{filename:id,generate:'client',css:'injected'}).js.code,map:null};if(id.endsWith('/lan.ts'))return {code:code.replaceAll('import.meta.env','({})'),map:null};}}]};`);
  execFileSync(join(modules, '.bin/rolldown'), ['--config', cfg, '--platform', 'browser'], { cwd: root, stdio: 'inherit' });
  return out;
}

try {
  const out = await bundle();
  server = createServer(async (req, res) => {
    if (new URL(req.url, 'http://x').pathname === '/bundle.js') { res.setHeader('content-type', 'text/javascript'); res.end(await readFile(out)); }
    else res.end('<!doctype html><div id="app"></div>');
  });
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 460, height: 900 } });
  await context.addInitScript(({ initial }) => {
    window.mailState = structuredClone(initial);
    window.mailCalls = [];
    window.statusPolls = 0;
    window.connectAttempts = 0;
    window.assignmentFailures = 2;
    window.failConnect = false;
    window.failDisconnect = true;
    window.__MONITTER_TEST_BRIDGE__ = { invoke: async (command, args = {}) => {
      window.mailCalls.push({ command, args });
      if (command === 'mail_get_config') return JSON.parse(JSON.stringify(window.mailState));
      if (command === 'mail_configure_gmail') { window.secretFieldArgs = args; window.mailState = { ...window.mailState, revision: 'r2', clientId: args.clientId }; return JSON.parse(JSON.stringify(window.mailState)); }
      if (command === 'mail_start_connect') {
        if (window.failConnect) throw Error('Mock OAuth launch failure');
        window.connectAttempts += 1; window.statusPolls = 0;
        return { attemptId: `attempt-${window.connectAttempts}`, authorizationUrl: 'https://accounts.google.com/mock', expiresAt: Date.now() + 20000 };
      }
      if (command === 'mail_connect_status') {
        window.statusPolls += 1;
        if (args.attemptId === 'attempt-1') { await new Promise(resolve => setTimeout(resolve, 650)); return { status: 'error', error: 'late stale attempt response', account: null }; }
        const second = { id: 'gmail-2', provider: 'gmail', email: 'two@gmail.com', label: 'two@gmail.com', agentIds: [], connectedAt: Date.now() };
        if (!window.mailState.accounts.some(account => account.id === second.id)) window.mailState = { ...window.mailState, revision: 'r2', accounts: [...window.mailState.accounts, second] };
        return { status: 'connected', error: null, account: second };
      }
      if (command === 'mail_cancel_connect') return undefined;
      if (command === 'mail_set_account_agents') {
        if (window.assignmentFailures > 0) { window.assignmentFailures -= 1; throw Error('Configuration changed; reload and try again.'); }
        window.mailState = { ...window.mailState, revision: 'r3', accounts: window.mailState.accounts.map(account => account.id === args.accountId ? { ...account, agentIds: args.agentIds } : account) };
        return JSON.parse(JSON.stringify(window.mailState));
      }
      if (command === 'mail_disconnect') {
        if (window.failDisconnect) { window.failDisconnect = false; throw Error('Configuration revision is stale.'); }
        window.mailState = { ...window.mailState, revision: 'r4', accounts: window.mailState.accounts.filter(account => account.id !== args.accountId) };
        return JSON.parse(JSON.stringify(window.mailState));
      }
      throw Error(`Unexpected ${command}`);
    }};
  }, { initial: config });
  const errors = [];
  const page = await context.newPage(); page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}`); await page.evaluate(() => import('/bundle.js'));

  await expect(page.getByRole('heading', { name: 'Mail' })).toBeVisible();
  assert.equal(await page.getByRole('checkbox').count(), 3, 'only local Codex, Claude, and generic ACP agents are eligible');
  assert.equal(await page.getByLabel('OAuth client ID').inputValue(), config.clientId, 'existing client ID is hydrated');
  await page.getByRole('button', { name: 'Connect another Gmail account' }).click();
  await expect(page.getByText('Waiting for Google sign-in to finish…')).toBeVisible();
  await page.waitForFunction(() => window.mailCalls.some(call => call.command === 'mail_connect_status' && call.args.attemptId === 'attempt-1'));
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByText('Waiting for Google sign-in to finish…')).toHaveCount(0);
  assert((await page.evaluate(() => window.mailCalls)).some(call => call.command === 'mail_cancel_connect' && call.args.attemptId === 'attempt-1'));

  await page.getByRole('button', { name: 'Connect another Gmail account' }).click();
  await expect(page.getByText('Connected two@gmail.com.')).toBeVisible({ timeout: 6000 });
  await expect(page.getByText('two@gmail.com', { exact: true })).toBeVisible();

  const atlas = page.getByRole('checkbox').nth(0);
  await atlas.check();
  await expect(page.getByRole('button', { name: 'Refresh and retry access' })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.mailCalls.find(call => call.command === 'mail_set_account_agents').args.agentIds), ['codex']);
  await atlas.uncheck();
  await expect(page.getByRole('button', { name: 'Refresh and retry access' })).toBeVisible();
  await expect(atlas).not.toBeChecked();
  await page.getByRole('button', { name: 'Refresh and retry access' }).click();
  await expect(page.getByText('Removing access takes effect immediately.')).toBeVisible();
  await atlas.check();
  await expect(page.getByText('Removing access takes effect immediately.')).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.mailState.accounts.find(account => account.id === 'gmail-1').agentIds), ['codex']);
  await page.screenshot({ path: '/tmp/monitter-mail-settings.png', fullPage: true });
  await page.getByRole('checkbox').nth(0).uncheck();
  await expect(page.getByRole('article').filter({ hasText: 'one@gmail.com' }).getByText('No agents can access this account until you assign one.')).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.mailState.accounts.find(account => account.id === 'gmail-1').agentIds), []);

  await page.getByRole('button', { name: 'Disconnect' }).first().click();
  await expect(page.getByRole('alert')).toContainText('Configuration revision is stale.');
  await expect(page.getByRole('button', { name: 'Refresh settings' })).toBeVisible();
  await page.getByRole('button', { name: 'Refresh settings' }).click();
  await expect(page.getByText('Mail settings refreshed.')).toBeVisible();
  await page.getByRole('button', { name: 'Disconnect' }).first().click();
  await expect(page.getByText('one@gmail.com', { exact: true })).toHaveCount(0);
  await page.evaluate(() => { window.failConnect = true; });
  await page.getByRole('button', { name: 'Connect another Gmail account' }).click();
  await expect(page.getByRole('alert')).toContainText('Mock OAuth launch failure');

  await page.getByText('Google app setup').click();
  await page.getByLabel('OAuth client secret').fill('temporary-secret');
  await page.getByRole('button', { name: 'Save app settings' }).click();
  await expect(page.getByLabel('OAuth client secret')).toHaveValue('');
  assert.equal((await page.evaluate(() => window.secretFieldArgs.clientSecret)), 'temporary-secret');
  assert.deepEqual(await page.evaluate(() => JSON.stringify(localStorage)), '{}');
  assert.deepEqual(errors, [], 'browser page errors');
  await context.close();

  const lan = await browser.newContext();
  await lan.addInitScript(() => { window.mailCalls = []; window.__MONITTER_TEST_BRIDGE__ = { invoke: async command => { window.mailCalls.push(command); throw Error(`LAN must not invoke ${command}`); } }; });
  const lanPage = await lan.newPage(); await lanPage.goto(`http://127.0.0.1:${server.address().port}`);
  await lanPage.evaluate(() => { document.documentElement.setAttribute('data-monitter-lan', ''); return import('/bundle.js'); });
  await expect(lanPage.getByText('Mail connections are managed on this desktop and are never available from LAN or shared browser clients.')).toBeVisible();
  assert.deepEqual(await lanPage.evaluate(() => window.mailCalls), []);
  await lan.close();
  console.log('Mail settings UI: OAuth pending/cancel/completion/error, eligible-agent filtering, grant stale-revision retry/removal, disconnect, secret clearing, and LAN restriction passed.');
} finally {
  await browser?.close();
  await new Promise(ok => server ? server.close(ok) : ok());
  await rm(temp, { recursive: true, force: true });
}
