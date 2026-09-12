import { chromium, expect as baseExpect } from '@playwright/test';
import { readFile, mkdtemp, mkdir, rm } from 'node:fs/promises';
import { execFileSync, spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { resolve, extname } from 'node:path';
import { pathToFileURL } from 'node:url';
import WebSocket from 'ws';

const expect = baseExpect.configure({ timeout: 30000 });

// Exercise the files extracted from the APK, including an actual encrypted
// pairing. The optional source path verifies compatibility with an older host.
await mkdir('verification', { recursive: true });
const directory = await mkdtemp(resolve('verification/android-apk-'));
let browser, relay, desktop;
try {
  execFileSync('unzip', ['-q', process.argv[2] ?? 'mobile-android/app/build/outputs/apk/debug/app-debug.apk', 'assets/*', '-d', directory]);
  const hostModule = resolve(directory, 'desktop-client.mjs');
  execFileSync('node_modules/.bin/rolldown', [process.argv[3] ?? 'src/lib/controller/remote-client.ts', '--platform', 'node', '--format', 'esm', '--file', hostModule, '--logLevel', 'silent']);
  globalThis.WebSocket = WebSocket;
  const { createDesktopSession } = await import(pathToFileURL(hostModule).href);
  relay = spawn(process.execPath, ['scripts/relay-server.mjs'], {
    env: { ...process.env, MONITTER_RELAY_PORT: '0', MONITTER_RELAY_HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const relayUrl = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Error('Test relay startup timed out')), 60000);
    let output = '';
    relay.stdout.on('data', data => {
      output += data;
      const match = output.match(/listening on (ws:\/\/[^\s]+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
    relay.once('error', error => { clearTimeout(timer); reject(error); });
    relay.once('exit', code => { clearTimeout(timer); reject(Error(`Relay exited ${code}`)); });
  });
  const taskId = '11111111-1111-4111-8111-111111111111';
  const snapshot = {
    hosts: [],
    agents: [{ id: 'a', name: 'APK test agent', provider: 'codex', model: '', avatar: null }],
    tasks: [{ id: taskId, agentId: 'a', title: 'APK verification chat', status: 'idle', archived: false, updatedAt: 1, projectId: 'p', parentTaskId: null, channelId: null }],
    projects: [{ id: 'p', name: 'APK test project', icon: 'folder', color: '#709cde', description: '', workspaces: [] }],
    messages: Array.from({length:150}, (_, i) => ({id: `history-${i}`, taskId, text: `Earlier message ${i}`, role:'assistant', createdAt:i})), events: [], channels: [], collaborations: [], queuedMessages: [], approvalRequests: [],
    settings: { accent: '#709cde', theme: 'dark', sidebarView: 'standard' },
  };
  let sends = 0, completedReads = 0, holdSnapshot = false, snapshotWaiting = false, releaseSnapshot, holdSend = false, sendWaiting = false, releaseSend;
  desktop = await createDesktopSession(relayUrl, {
    getSnapshot: async () => { if (holdSnapshot) { snapshotWaiting = true; await new Promise(resolve => releaseSnapshot = resolve); } completedReads++; return snapshot; },
    sendMessage: async (id, text) => { if (holdSend) { sendWaiting = true; await new Promise(resolve => releaseSend = resolve); } sends++; snapshot.messages.push({ id: 'm', taskId: id, text, role: 'user', createdAt: Date.now() }); return snapshot; },
    cancelTask: async () => snapshot, resumeTask: async () => snapshot,
    listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }),
  });
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 412, height: 915 }, isMobile: true, hasTouch: true });
  // The production app uses WSS; this test alone connects to a loopback relay.
  await page.context().grantPermissions(['local-network-access'], {origin:'https://appassets.androidplatform.net'});
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') console.log('Browser:', message.text()); });
  await page.route('https://appassets.androidplatform.net/**', async route => {
    const pathname = new URL(route.request().url()).pathname;
    const file = resolve(directory, 'assets', pathname === '/mobile' ? 'index.html' : '.' + pathname);
    try {
      await route.fulfill({ body: await readFile(file), contentType: ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.json': 'application/json' })[extname(file)] ?? 'application/octet-stream' });
    } catch { await route.fulfill({ status: 404, body: 'Not found' }); }
  });
  await page.goto('https://appassets.androidplatform.net/mobile');
  await expect(page.getByRole('button', { name: 'Connect to desktop', exact: true })).toBeVisible();
  assert.equal(await page.evaluate(() => window.isSecureContext && !!crypto.subtle), true);
  await page.evaluate(invitation => window.dispatchEvent(new CustomEvent('monitter:qr', { detail: invitation })), desktop.invitation);
  try { await expect.poll(() => desktop.getStatus()).toBe('pending'); } catch(error) { console.log(await page.locator('body').innerText()); await page.screenshot({path:'verification/android-pairing-error.png'}); throw error; }
  await expect(page.getByText(desktop.getVerificationCode(), { exact: true })).toBeVisible();
  await desktop.approve();
  await expect(page.getByRole('tab', { name: 'Standard view', exact: true })).toBeVisible();
  holdSnapshot = true;
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect.poll(() => snapshotWaiting).toBe(true);
  await page.getByRole('tab', { name: 'Activity view', exact: true }).click();
  await expect(page.getByRole('button', { name: /APK verification chat/ })).toBeVisible();
  await page.getByRole('tab', { name: 'Projects view', exact: true }).click();
  await expect(page.getByText('APK test project', { exact: true })).toBeVisible();
  const beforeReads = completedReads; holdSnapshot = false; releaseSnapshot();
  await expect.poll(() => completedReads).toBeGreaterThan(beforeReads + 1);
  await expect(page.getByRole('tab', { name: 'Projects view', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.screenshot({ path: 'verification/android-menu.png' });
  await page.getByRole('button', { name: /APK verification chat/ }).click();
  await expect(page.locator('.thread article')).toHaveCount(100);
  await page.getByRole('button', { name: /Load 100 earlier messages/ }).click();
  await expect(page.locator('.thread article')).toHaveCount(150);
  await page.getByLabel('Message', { exact: true }).fill('Message from packaged APK');
  holdSend = true;
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => sendWaiting).toBe(true);
  await page.getByRole('button', { name: 'Back to chats', exact: true }).click();
  await page.getByRole('button', { name: /APK verification chat/ }).click();
  await expect(page.getByLabel('Message', { exact: true })).toHaveValue('Message from packaged APK');
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeDisabled();
  holdSend = false; releaseSend();
  await expect(page.getByText('Message from packaged APK', { exact: true })).toBeVisible();
  assert.equal(sends, 1);
  await page.screenshot({ path: 'verification/android-production.png' });
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Connect to desktop', exact: true })).toBeVisible();
  assert.deepEqual(errors, []);
  console.log('APK assets: secure pairing, sidebar modes, stalled refresh/send navigation, draft retention, bounded history, single send and disconnect passed.');
} finally {
  await browser?.close(); desktop?.close(); relay?.kill('SIGTERM');
  await rm(directory, { recursive: true, force: true });
}
