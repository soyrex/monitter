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
  const ownerVoiceCalls = [];
  const ownerSentMessages = [];
  let limitedSnapshotCalls = 0;
  const snapshot = {
    hosts: [],
    agents: [{ id: 'test-agent', name: 'Recovery test agent', provider: 'codex', model: '', hostId: 'local', cwd: '/tmp', sandbox: 'workspace-write', color: '#00A8F0', description: '', instructions: '', avatar: null, expertise: [], responsibilities: [], skills: [], collaborationEnabled: false }],
    tasks: [{ id: 'test-task', agentId: 'test-agent', title: 'Recovery reading test', nativeSessionId: null, archived: false, status: 'idle', createdAt: Date.now(), updatedAt: Date.now(), parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp', provider: 'codex', model: '', sandbox: 'workspace-write' }],
    messages: [{ id: 'test-message', taskId: 'test-task', role: 'assistant', text: 'Keep this conversation readable while reconnecting.', createdAt: Date.now() }],
    events: [], channels: [], projects: [],
    collaborations: [], queuedMessages: [], approvalRequests: [], settings: { theme: 'dark', accent: '#00A8F0' },
  };
  const desktopOptions = {
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
    onOwnerInvoke: async (command, args) => {
      ownerCommands.push(command);
      if (command === 'transcribe_voice_message') {
        ownerVoiceCalls.push(args);
        return { text: 'Transcript from iPhone' };
      }
      if (command === 'send_message') ownerSentMessages.push(args);
      if (command === 'get_owner_appearance') return {
        version: 1, appTheme: { light: 'monitter', dark: 'monitter', accent: null, contrast: 0 },
        surfaceTint: 3, terminalTheme: 'monitter',
      };
      if (command === 'get_command_capabilities') return { protocolVersion: 1, commands: ['get_snapshot', 'get_usage_overview', 'list_codex_accounts', 'send_message', 'transcribe_voice_message', 'get_task_git_status', 'get_task_goal', 'list_terminals', 'get_task_model_settings', 'get_task_slash_commands'] };
      if (command === 'get_snapshot' || command === 'send_message') return snapshot;
      if (command === 'list_codex_accounts' || command === 'list_terminals' || command === 'get_task_slash_commands') return [];
      if (command === 'get_usage_overview') return { generatedAt: Date.now(), capturedSince: null, subscriptions: [], providerTotals: [], recentRuns: [] };
      return null;
    },
  };
  desktop = await createResumableDesktopSession(desktopOptions);

  browser = await webkit.launch();
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await phone.addInitScript(() => {
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }) },
    });
    window.MediaRecorder = class {
      state = 'inactive';
      mimeType = 'audio/mp4';
      start() { this.state = 'recording'; }
      stop() {
        this.state = 'inactive';
        this.ondataavailable?.({ data: new Blob([new Uint8Array([1])], { type: this.mimeType }) });
        this.onstop?.();
      }
    };
    window.AudioContext = class {
      async decodeAudioData() {
        return { numberOfChannels: 1, sampleRate: 16_000, length: 16, getChannelData: () => new Float32Array(16) };
      }
      async close() {}
    };
  });
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
  // iOS can report native resume, online and pageshow together. Exercise the
  // real owner route, including a currently-connected socket, and retain the
  // page and saved owner trust while the encrypted session is replaced once.
  await phone.waitForTimeout(500);
  const snapshotsBeforeResume = ownerCommands.filter(command => command === 'get_snapshot').length;
  let resumedConnections = 0;
  const stopResume = desktop.subscribe(state => { if (state.status === 'connected') resumedConnections++; });
  resumedConnections = 0;
  await phone.evaluate(() => {
    window.__networkRecoveryPage = 'same-page';
    for (const type of ['monitter:resume', 'online', 'pageshow']) window.dispatchEvent(new Event(type));
  });
  await expect.poll(() => resumedConnections).toBe(1);
  await expect(phone.locator('.app-shell')).toHaveCount(1);
  await expect.poll(() => ownerCommands.filter(command => command === 'get_snapshot').length).toBeGreaterThan(snapshotsBeforeResume);
  assert.equal(desktop.getPeerControllerPublicKey(), phoneKey);
  assert.equal(desktop.getOwnerWorkspaceState(), 'approved');
  assert.equal(await phone.evaluate(() => window.__networkRecoveryPage), 'same-page', 'resume reloaded the page');
  await phone.waitForTimeout(750);
  assert.equal(resumedConnections, 1, 'lifecycle hints produced duplicate reconnects');
  stopResume();
  // Keep a real chat/draft mounted during a sustained outage. Background
  // snapshot polling must not report the drop before an attempted action.
  await phone.getByRole('button', { name: 'Open chat Recovery reading test', exact: true }).click();
  await expect(phone.getByText('Keep this conversation readable while reconnecting.', { exact: true })).toBeVisible();
  const composer = phone.getByRole('textbox', { name: 'Task message', exact: true });
  const recorder = phone.locator('.composer .voice-recorder');
  await expect(recorder.getByRole('button', { name: 'Record voice message' })).toBeVisible();
  await recorder.getByRole('button', { name: 'Record voice message' }).click();
  await expect(recorder.getByRole('button', { name: 'Stop recording' })).toBeVisible();
  await recorder.getByRole('button', { name: 'Stop recording' }).click();
  await expect(composer).toHaveValue('Transcript from iPhone');
  assert.equal(ownerVoiceCalls.length, 1, 'iPhone audio did not reach desktop transcription');
  assert.equal(typeof ownerVoiceCalls[0]?.audioBase64, 'string');
  await phone.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect.poll(() => ownerSentMessages.length).toBe(1);
  assert.deepEqual(ownerSentMessages[0]?.attachmentIds ?? [], [], 'iPhone voice attached audio to the agent message');
  assert.equal(ownerSentMessages[0]?.text, 'Transcript from iPhone');
  assert.equal(ownerCommands.includes('store_attachment'), false, 'iPhone audio was stored as an attachment');
  await composer.fill('Preserve this unsent draft');
  await phone.evaluate(() => { window.__networkRecoveryShell = document.querySelector('.app-shell'); });
  desktop.close();
  await expect(phone.getByTestId('connection-recovery-spinner')).toBeVisible();
  await phone.waitForTimeout(2_000); // Cross the actual background snapshot poll.
  await expect(phone.locator('.alert.error')).toHaveCount(0);
  await expect(phone.getByText('Keep this conversation readable while reconnecting.', { exact: true })).toBeVisible();
  assert.equal(await phone.evaluate(() => window.__networkRecoveryShell === document.querySelector('.app-shell')), true, 'outage remounted the workspace');
  assert.equal(await composer.evaluate(node => !!node.closest('[inert]')), false, 'outage blocked the composer');
  assert.equal(await phone.locator('.owner-relay-content').evaluate(node => !!node.closest('[inert]')), false, 'outage blocked the workspace');
  assert.equal(await phone.getByTestId('connection-recovery-spinner').evaluate(node => getComputedStyle(node).pointerEvents), 'none');
  await phone.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect(phone.locator('.alert.error')).toContainText('Reconnecting to your desktop. Try again once connected.');
  await expect(composer).toHaveValue('Preserve this unsent draft');
  assert.equal(ownerSentMessages.length, 1, 'offline send was dispatched');
  desktop = await createResumableDesktopSession(desktopOptions);
  await expect(phone.getByTestId('connection-recovery-spinner')).toHaveCount(0);
  await expect.poll(() => desktop.getOwnerWorkspaceState()).toBe('approved');
  assert.equal(desktop.getPeerControllerPublicKey(), phoneKey);
  assert.equal(await phone.evaluate(() => window.__networkRecoveryShell === document.querySelector('.app-shell')), true, 'reconnect remounted the workspace');
  await expect(composer).toHaveValue('Preserve this unsent draft');
  assert.equal(ownerSentMessages.length, 1, 'reconnect replayed an unsent draft');
  console.log('WebKit owner relay: transcript-only voice, approval, foreground recovery, quiet readable outage, and explicit send feedback passed.');
} finally {
  desktop?.close();
  await browser?.close();
  for (const child of children) child.kill('SIGTERM');
}
