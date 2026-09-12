import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { chromium, expect as baseExpect } from '@playwright/test';

const expect = baseExpect.configure({ timeout: 15_000 });
const children = [];

function start(command, args, env, pattern) {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  children.push(child);
  return new Promise((resolve, reject) => {
    let output = '';
    child.testOutput = () => output;
    const timer = setTimeout(() => reject(new Error(`Test server startup timed out: ${output.slice(-1000)}`)), 60_000);
    const receive = data => {
      output += data.toString();
      const match = output.match(pattern);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', receive);
    child.stderr.on('data', receive);
    child.once('error', reason => { clearTimeout(timer); reject(reason); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Test server exited ${code}: ${output.slice(-1000)}`)); });
  });
}

async function stopChildren() {
  for (const child of children) if (child.exitCode === null) child.kill('SIGTERM');
  await Promise.all(children.map(child => child.exitCode === null
    ? new Promise(resolve => { child.once('exit', resolve); setTimeout(() => { if (child.exitCode === null) child.kill('SIGKILL'); }, 2_000); })
    : Promise.resolve()));
}

async function gotoReady(page, url) {
  const deadline = Date.now() + 30_000;
  let lastError;
  while (Date.now() < deadline) {
    try { await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 15_000 }); return; }
    catch (reason) { lastError = reason; await new Promise(resolve => setTimeout(resolve, 250)); }
  }
  throw lastError;
}

async function openRemoteControl(page) {
  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  const settings = page.getByRole('region', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Remote control', exact: true }).click();
  return settings;
}

let browser, desktop, phone, moduleDirectory, staticServer;
const diagnostics = { errors: [], console: [], requests: [] };
try {
  const relayUrl = await start(process.execPath, ['scripts/relay-server.mjs'], {
    MONITTER_RELAY_PORT: '0', MONITTER_RELAY_HOST: '127.0.0.1',
  }, /listening on (ws:\/\/[^\s]+)/);
  const suppliedBaseUrl = process.env.MONITTER_PAIRING_TEST_BASE_URL;
  let baseUrl = suppliedBaseUrl;
  if (!baseUrl) {
    const buildRoot = resolve('build'), indexPath = join(buildRoot, 'index.html');
    try { await readFile(indexPath); }
    catch { throw new Error('Production web candidate is missing build/index.html. Build it first or set MONITTER_PAIRING_TEST_BASE_URL.'); }
    const mime = { '.css': 'text/css', '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.png': 'image/png', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
    staticServer = createServer(async (request, response) => {
      try {
        const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
        const assetPath = resolve(buildRoot, `.${pathname}`);
        const isAsset = assetPath.startsWith(buildRoot + '/') && extname(assetPath) !== '';
        const path = isAsset ? assetPath : indexPath;
        const body = await readFile(path);
        response.writeHead(200, { 'Content-Type': mime[extname(path)] ?? 'application/octet-stream', 'Cache-Control': 'no-store' });
        response.end(body);
      } catch { response.writeHead(404).end('Not found'); }
    });
    await new Promise((resolveListen, reject) => { staticServer.once('error', reject); staticServer.listen(0, '127.0.0.1', resolveListen); });
    baseUrl = `http://127.0.0.1:${staticServer.address().port}/`;
  }

  moduleDirectory = await mkdtemp(join(tmpdir(), 'monitter-pairing-ui-'));
  const moduleEntry = join(moduleDirectory, 'entry.ts'), moduleBundle = join(moduleDirectory, 'pairing.js');
  await writeFile(moduleEntry, `export * from ${JSON.stringify(resolve('src/lib/controller/pairing-store.ts'))};\nexport * from ${JSON.stringify(resolve('src/lib/controller/secure-session.ts'))};\n`);
  execFileSync('node_modules/.bin/rolldown', [moduleEntry, '--platform', 'browser', '--format', 'esm', '--file', moduleBundle, '--logLevel', 'silent']);
  const pairingModule = await readFile(moduleBundle);

  browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.route('**/test-pairing-modules.js', route => route.fulfill({ status: 200, contentType: 'text/javascript', body: pairingModule }));
  await context.addInitScript({ content: readFileSync('scripts/ui-fixture.js', 'utf8') });
  desktop = await context.newPage();
  phone = await context.newPage();
  desktop.on('pageerror', error => diagnostics.errors.push(`desktop: ${error.message}`));
  phone.on('pageerror', error => diagnostics.errors.push(`phone: ${error.message}`));
  for (const [name, page] of [['desktop', desktop], ['phone', phone]]) {
    page.on('console', message => diagnostics.console.push(`${name} ${message.type()}: ${message.text()}`));
    page.on('requestfailed', request => diagnostics.requests.push(`${name}: ${request.url()} - ${request.failure()?.errorText}`));
  }

  // Establish this origin, then seed both nonextractable identities through the
  // shipped persistence module. The UI itself owns every session after reload.
  await gotoReady(desktop, baseUrl);
  await desktop.getByRole('button', { name: 'Preferences', exact: true }).waitFor({ timeout: 90_000 });
  const seeded = await desktop.evaluate(async relayUrl => {
    const store = await import('/test-pairing-modules.js');
    const secure = store;
    await Promise.all([store.clearDesktopPairing(), store.clearMobilePairing()]);
    const desktopRecord = await store.newDesktopPairing(relayUrl);
    desktopRecord.enabled = true;
    desktopRecord.inactivityDays = 14;
    desktopRecord.devices = [];
    const mobileKeyPair = await secure.generatePairingKeyPair();
    const desktopPublicKey = secure.bytesToBase64url(await secure.exportPairingPublicKey(desktopRecord.identity.keyPair.publicKey));
    const mobilePublicKey = secure.bytesToBase64url(await secure.exportPairingPublicKey(mobileKeyPair.publicKey));
    const invitation = JSON.stringify({ version: 2, relayUrl, room: desktopRecord.identity.room, publicKey: desktopPublicKey });
    await store.saveDesktopPairing(desktopRecord);
    await store.saveMobilePairing({ version: 1, invitation, identity: { keyPair: mobileKeyPair }, savedAt: Date.now() });
    return { room: desktopRecord.identity.room, desktopPublicKey, mobilePublicKey };
  }, relayUrl);

  await desktop.reload({ waitUntil: 'commit' });
  const settings = await openRemoteControl(desktop);
  const panel = settings.getByRole('region', { name: 'Remote control' });
  await expect(panel).toBeVisible();
  await expect(desktop.getByRole('dialog', { name: 'Remote control' })).toHaveCount(0);
  await expect(panel).toHaveCount(1);
  await expect(panel.getByRole('status')).toContainText('Ready for a phone');
  await panel.getByRole('button', { name: 'Create pairing code', exact: true }).click();
  const qr = panel.getByAltText('Mobile pairing QR code');
  await expect(qr).toBeVisible();
  assert.equal(await qr.evaluate((image) => {
    const panel = image.closest('.remote-panel');
    const imageWidth = image.getBoundingClientRect().width;
    const panelWidth = panel?.getBoundingClientRect().width ?? 0;
    return imageWidth <= 280.5 && imageWidth <= panelWidth + .5;
  }), true, 'Pairing QR must fit the embedded Settings region.');

  await gotoReady(phone, new URL('mobile', baseUrl).href);
  const policyInput = panel.getByLabel('Inactivity limit (days)');
  await expect(policyInput).toHaveValue('14');
  await expect(panel.getByRole('status')).toContainText('Waiting for phone approval');
  await expect(phone.getByText(/Compare this number with your desktop/)).toBeVisible();
  await policyInput.fill('7');
  await panel.getByRole('button', { name: 'Save policy', exact: true }).click();
  await expect.poll(() => desktop.evaluate(async () => (await import('/test-pairing-modules.js')).loadDesktopPairing().then(record => record.inactivityDays))).toBe(7);
  await expect(panel.getByRole('status')).toContainText('Waiting for phone approval');
  await expect(phone.locator('header small')).toContainText('awaiting approval');
  await panel.getByRole('button', { name: /approve phone/i }).click();

  await expect(phone.getByRole('heading', { name: 'Your workspace', exact: true })).toBeVisible();
  await expect(phone.locator('header small')).toContainText('connected');
  await expect(panel.getByText('Phone 1', { exact: true })).toBeVisible();
  await expect(panel.getByText(/connected · Last access/)).toBeVisible();

  // Moving away from the remote category removes the portal target, not the
  // root-owned listener. Returning restores the same connected controller.
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expect(desktop.locator('.remote-panel')).toBeHidden();
  await expect(phone.locator('header small')).toContainText('connected');
  await settings.getByRole('button', { name: 'Remote control', exact: true }).click();
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('status')).toContainText('Phone connected');
  await expect(phone.locator('header small')).toContainText('connected');

  // Closing the Settings tab removes the target altogether. Reopening it must
  // reattach the existing controller rather than creating a new approval flow.
  await desktop.locator('.settings-tab').hover();
  await desktop.getByRole('button', { name: 'Close Settings tab', exact: true }).click();
  await expect(desktop.getByRole('region', { name: 'Settings' })).toHaveCount(0);
  await expect(desktop.locator('.remote-panel')).toBeHidden();
  await expect(phone.locator('header small')).toContainText('connected');
  const restoredSettings = await openRemoteControl(desktop);
  const reattachedPanel = restoredSettings.getByRole('region', { name: 'Remote control' });
  await expect(reattachedPanel).toBeVisible();
  await expect(reattachedPanel.getByRole('status')).toContainText('Phone connected');
  await expect(reattachedPanel.getByRole('button', { name: /approve phone/i })).toHaveCount(0);
  await policyInput.fill('');
  await panel.getByRole('button', { name: 'Save policy', exact: true }).click();
  await expect.poll(() => desktop.evaluate(async () => (await import('/test-pairing-modules.js')).loadDesktopPairing().then(record => record.inactivityDays))).toBe(null);
  await policyInput.fill('14');
  await panel.getByRole('button', { name: 'Save policy', exact: true }).click();
  await expect.poll(() => desktop.evaluate(async () => (await import('/test-pairing-modules.js')).loadDesktopPairing().then(record => record.inactivityDays))).toBe(14);
  const approved = await desktop.evaluate(async mobilePublicKey => {
    const store = await import('/test-pairing-modules.js');
    const record = await store.loadDesktopPairing();
    return record.devices.find(device => device.publicKey === mobilePublicKey);
  }, seeded.mobilePublicKey);
  assert.ok(approved, 'Approved phone was not persisted.');

  // A valid authenticated refresh slides the 14-day inactivity timestamp.
  await phone.waitForTimeout(20);
  await phone.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect.poll(() => desktop.evaluate(async mobilePublicKey => {
    const store = await import('/test-pairing-modules.js');
    return (await store.loadDesktopPairing()).devices.find(device => device.publicKey === mobilePublicKey)?.lastAccessAt ?? 0;
  }, seeded.mobilePublicKey)).toBeGreaterThan(approved.lastAccessAt);
  const persistedPolicy = await desktop.evaluate(async () => (await import('/test-pairing-modules.js')).loadDesktopPairing());
  assert.equal(persistedPolicy.inactivityDays, 14);

  // Recreating the desktop page restores its saved identity and room. The live
  // phone reconnects without returning to manual approval.
  await desktop.reload({ waitUntil: 'commit' });
  const reloadSettings = await openRemoteControl(desktop);
  const restoredPanel = reloadSettings.getByRole('region', { name: 'Remote control' });
  await expect(phone.getByRole('heading', { name: 'Your workspace', exact: true })).toBeVisible();
  await expect(phone.locator('header small')).toContainText('connected');
  await expect(restoredPanel.getByText('Phone 1', { exact: true })).toBeVisible();
  await expect(restoredPanel.getByRole('status')).toContainText('connected');
  await expect(restoredPanel.getByRole('button', { name: /approve phone/i })).toHaveCount(0);
  const restoredIdentity = await desktop.evaluate(async () => {
    const store = await import('/test-pairing-modules.js');
    const secure = store;
    const record = await store.loadDesktopPairing();
    return { room: record.identity.room, publicKey: secure.bytesToBase64url(await secure.exportPairingPublicKey(record.identity.keyPair.publicKey)) };
  });
  assert.deepEqual(restoredIdentity, { room: seeded.room, publicKey: seeded.desktopPublicKey });

  // A cold phone route reload reads its persisted key and reconnects without a
  // QR, device key, or another approval click.
  await phone.reload({ waitUntil: 'commit' });
  await expect(phone.getByRole('heading', { name: 'Your workspace', exact: true })).toBeVisible();
  await expect(phone.locator('header small')).toContainText('connected');
  await expect(restoredPanel.getByRole('status')).toContainText('connected');
  await expect(restoredPanel.getByRole('button', { name: /approve phone/i })).toHaveCount(0);
  await desktop.screenshot({ path: 'verification/remembered-phones.png', fullPage: true });

  // Individual revocation updates the rendered list and durable record, closes
  // this phone, and removes its saved mobile pairing.
  await restoredPanel.getByRole('button', { name: 'Revoke Phone 1', exact: true }).click();
  await expect(restoredPanel.getByText('No phones are remembered yet.', { exact: true })).toBeVisible();
  await expect(phone.getByRole('alert')).toContainText('revoked');
  await expect(phone.getByRole('button', { name: 'Connect to desktop', exact: true })).toBeVisible();
  const revoked = await desktop.evaluate(async () => {
    const store = await import('/test-pairing-modules.js');
    const [desktopRecord, mobileRecord] = await Promise.all([store.loadDesktopPairing(), store.loadMobilePairing()]);
    return { devices: desktopRecord.devices.length, mobileCleared: mobileRecord === null };
  });
  assert.deepEqual(revoked, { devices: 0, mobileCleared: true });
  if (diagnostics.errors.length) throw new Error(diagnostics.errors.join('\n'));
  console.log('Persistent pairing UI: 14-day sliding policy, remembered phone, desktop/phone cold reload, and individual revocation passed.');
} catch (reason) {
  diagnostics.errors.push(...await Promise.all([['desktop', desktop], ['phone', phone]].map(async ([name, page]) => {
    if (!page) return `${name}: page was not created`;
    try {
      await page.screenshot({ path: `/tmp/monitter-persistent-pairing-${name}.png` });
      return `${name}: url=${page.url()} body=${(await page.locator('body').innerText({ timeout: 2_000 })).slice(0, 4000)}`;
    } catch (captureError) { return `${name}: diagnostic capture failed: ${captureError}`; }
  })));
  console.error(JSON.stringify({
    ...diagnostics,
    children: children.map(child => child.testOutput?.().slice(-5000) ?? ''),
  }, null, 2));
  throw reason;
} finally {
  await browser?.close();
  await stopChildren();
  await new Promise(resolveClose => staticServer ? staticServer.close(resolveClose) : resolveClose());
  if (moduleDirectory) await rm(moduleDirectory, { recursive: true, force: true });
}
