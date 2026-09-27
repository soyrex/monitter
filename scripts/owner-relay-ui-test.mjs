import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { webkit, expect as baseExpect } from '@playwright/test';
import WebSocket from 'ws';

const { createResumableDesktopSession, generatePairingKeyPair, bytesToBase64url, randomBytes } =
  await import(process.env.MONITTER_OWNER_TEST_BRIDGE);

globalThis.WebSocket = WebSocket;
const expect = baseExpect.configure({ timeout: 30_000 });
const children = [];

function start(command, args, env, pattern) {
  const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child);
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error(`Test server startup timed out: ${output.slice(-500)}`)), 60_000);
    child.once('error', reason => { clearTimeout(timer); reject(reason); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Test server exited ${code}: ${output.slice(-500)}`)); });
    const receive = data => {
      output += data.toString();
      const match = output.match(pattern);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', receive);
    child.stderr.on('data', receive);
  });
}

let browser;
let desktop;
try {
  const relayUrl = await start(process.execPath, ['scripts/relay-server.mjs'],
    { MONITTER_RELAY_PORT: '0', MONITTER_RELAY_HOST: '127.0.0.1' }, /listening on (ws:\/\/[^\s]+)/);
  const baseUrl = await start(process.execPath,
    ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '31977', '--strictPort'],
    { NO_COLOR: '1' }, /Local:\s+(http:\/\/[^\s]+)/);

  const controllerTrusted = new Set();
  const ownerTrusted = new Set();
  const ownerCommands = [];
  let limitedSnapshotCalls = 0;
  const snapshot = {
    hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [],
    collaborations: [], queuedMessages: [], approvalRequests: [], settings: { theme: 'dark', accent: '#00A8F0' },
  };
  desktop = await createResumableDesktopSession({
    relayUrl,
    identity: { keyPair: await generatePairingKeyPair(), room: bytesToBase64url(randomBytes(18)) },
    bridge: {
      getSnapshot: async () => { limitedSnapshotCalls++; return snapshot; },
      sendMessage: async () => snapshot, cancelTask: async () => snapshot, resumeTask: async () => snapshot,
      listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }),
    },
    isTrustedController: key => controllerTrusted.has(key),
    onControllerAccess: key => { controllerTrusted.add(key); },
    isTrustedOwnerWorkspace: key => ownerTrusted.has(key),
    onOwnerWorkspaceAccess: () => {},
    onOwnerInvoke: async (command) => {
      ownerCommands.push(command);
      if (command === 'get_owner_appearance') return {
        version: 1, appTheme: { light: 'monitter', dark: 'monitter', accent: null, contrast: 0 },
        surfaceTint: 3, terminalTheme: 'monitter',
      };
      if (command === 'get_command_capabilities') return { protocolVersion: 1, commands: ['get_snapshot'] };
      if (command === 'get_snapshot') return snapshot;
      return null;
    },
  });

  browser = await webkit.launch();
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  // The shell's fixed private origin must enter owner bootstrap even if a
  // WebKit navigation drops the optional ?owner=1 query string.
  await phone.goto(new URL('/mobile', baseUrl).href, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(phone.getByRole('button', { name: 'Scan desktop code' })).toBeVisible();
  await phone.evaluate(invitation => window.dispatchEvent(new CustomEvent('monitter:qr', { detail: invitation })), desktop.invitation);
  await expect.poll(() => desktop.getStatus()).toBe('pending');
  await desktop.approve();
  await expect.poll(() => desktop.getOwnerWorkspaceState()).toBe('requested');
  assert.equal(limitedSnapshotCalls, 0, 'iOS owner bootstrap fetched the limited controller workspace');
  assert.equal(await phone.locator('.view-tabs').count(), 0, 'limited controller UI was rendered during owner approval');
  const phoneKey = desktop.getPeerControllerPublicKey();
  assert.ok(phoneKey, 'phone identity was missing');
  ownerTrusted.add(phoneKey);
  await desktop.approveOwnerWorkspace();
  await phone.waitForURL(url => url.pathname === '/', { timeout: 30_000 });
  await expect(phone.locator('.app-shell')).toHaveCount(1);
  await expect.poll(() => ownerCommands.includes('get_owner_appearance')).toBe(true);
  await expect.poll(() => ownerCommands.includes('get_snapshot')).toBe(true);
  await expect(phone.locator('.app-shell')).not.toHaveAttribute('inert', '');
  assert.equal(limitedSnapshotCalls, 0, 'iOS owner bootstrap fell back to the limited controller after approval');
  console.log('WebKit owner relay: separate approval, root navigation, and full AppSurface handoff passed.');
} finally {
  desktop?.close();
  await browser?.close();
  for (const child of children) child.kill('SIGTERM');
}
