import assert from 'node:assert/strict';
import { mock } from 'node:test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import WebSocket, { WebSocketServer } from 'ws';
import { createResumableDesktopSession, createResumableMobileSession } from '../src/lib/controller/resumable-session.ts';
import { watchReconnectEvents } from '../src/lib/controller/reconnect-events.ts';
import { generatePairingKeyPair, exportPairingPublicKey, bytesToBase64url, randomBytes } from '../src/lib/controller/secure-session.ts';

// Browser foreground + native resume + restored networking is one recovery,
// including cancellation when the owning route has unmounted.
globalThis.window = new EventTarget();
globalThis.document = new EventTarget();
document.visibilityState = 'visible';
mock.timers.enable({ apis: ['setTimeout'] });
try {
  let reconnects = 0;
  const stop = watchReconnectEvents(() => reconnects++);
  for (const name of ['online', 'pageshow', 'monitter:resume']) window.dispatchEvent(new Event(name));
  document.dispatchEvent(new Event('visibilitychange'));
  mock.timers.tick(249); assert.equal(reconnects, 0);
  mock.timers.tick(1); assert.equal(reconnects, 1);
  document.visibilityState = 'hidden';
  window.dispatchEvent(new Event('monitter:resume'));
  mock.timers.tick(500); assert.equal(reconnects, 1);
  document.visibilityState = 'visible';
  document.dispatchEvent(new Event('visibilitychange'));
  stop(); mock.timers.tick(500);
  window.dispatchEvent(new Event('online')); mock.timers.tick(500);
  assert.equal(reconnects, 1, 'unmounted routes do not reconnect');
} finally {
  mock.timers.reset(); delete globalThis.window; delete globalThis.document;
}

globalThis.WebSocket = WebSocket;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const until = async (check, label, timeout = 70_000) => {
  const deadline = Date.now() + timeout;
  while (!check()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${label}`);
    await delay(50);
  }
};
const launchRelay = async (port = '0') => {
  const process = spawn(globalThis.process.execPath, ['scripts/relay-server.mjs'], {
    cwd: globalThis.process.cwd(), env: { ...globalThis.process.env, MONITTER_RELAY_PORT: port },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  try {
    const address = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Relay startup timed out.')), 5_000);
      process.stdout.on('data', data => {
        const match = data.toString().match(/ws:\/\/127\.0\.0\.1:\d+/);
        if (match) { clearTimeout(timer); resolve(match[0]); }
      });
      process.once('error', error => { clearTimeout(timer); reject(error); });
      process.once('exit', code => { clearTimeout(timer); reject(new Error(`Relay exited: ${code}`)); });
    });
    return { process, address };
  } catch (error) { process.kill(); throw error; }
};

let relay, gateway, desktop, mobile;
let releaseSend = () => {};
const connections = [];
try {
  relay = await launchRelay();
  const relayPort = new URL(relay.address).port;
  gateway = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await once(gateway, 'listening');
  gateway.on('connection', (client, request) => {
    const url = new URL(request.url, relay.address);
    const upstream = new WebSocket(relay.address + url.search);
    const connection = { client, upstream, role: url.searchParams.get('role'), dropped: false };
    connections.push(connection);
    const queued = [];
    upstream.on('open', () => { for (const message of queued) upstream.send(message); queued.length = 0; });
    client.on('message', message => {
      if (connection.dropped) return;
      if (upstream.readyState === WebSocket.OPEN) upstream.send(message.toString());
      else if (upstream.readyState === WebSocket.CONNECTING) queued.push(message.toString());
    });
    upstream.on('message', message => {
      if (!connection.dropped && client.readyState === WebSocket.OPEN) client.send(message.toString());
    });
    // The failure leaves both sockets apparently live. Even the close handshake
    // is lost, as on an interface change; only the application lease frees the role.
    client.on('close', () => { if (!connection.dropped) upstream.close(); });
    upstream.on('close', () => { if (!connection.dropped) client.close(); });
    client.on('error', () => { if (!connection.dropped) upstream.close(); });
    upstream.on('error', () => { if (!connection.dropped) client.close(); });
  });
  const relayUrl = `ws://127.0.0.1:${gateway.address().port}`;
  const desktopIdentity = { keyPair: await generatePairingKeyPair(), room: bytesToBase64url(randomBytes(18)) };
  const mobileIdentity = { keyPair: await generatePairingKeyPair() };
  const mobileKey = bytesToBase64url(await exportPairingPublicKey(mobileIdentity.keyPair.publicKey));
  let sends = 0, approvals = 0;
  const snapshot = { hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], settings: {} };
  const bridge = {
    getSnapshot: async () => snapshot,
    sendMessage: async () => { sends++; await new Promise(resolve => { releaseSend = resolve; }); return { accepted: true }; },
    cancelTask: async () => snapshot, resumeTask: async () => snapshot,
    listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }),
  };
  desktop = await createResumableDesktopSession({ relayUrl, identity: desktopIdentity, bridge, isTrustedController: key => key === mobileKey });
  desktop.onPendingPeer(() => { if (desktop.getPeerControllerPublicKey() !== mobileKey) approvals++; });
  const invitation = desktop.invitation;
  mobile = await createResumableMobileSession({ invitation, identity: mobileIdentity });
  const connected = () => desktop.getStatus() === 'connected' && mobile.getStatus() === 'connected';
  await until(connected, 'initial trusted connection', 5_000);
  const initialSnapshot = await mobile.getSnapshot();
  assert.deepEqual(initialSnapshot.tasks, snapshot.tasks);

  // Drop the phone's entire path after a mutation was accepted, while leaving
  // OPEN sockets at the client and relay. Recovery must not repeat that mutation.
  let settled = false, result;
  const pending = mobile.sendMessage('11111111-1111-4111-8111-111111111111', 'accepted once')
    .then(value => { settled = true; result = value; }, error => { settled = true; result = error; });
  await until(() => sends === 1, 'accepted mutation', 5_000);
  const phonePath = connections.findLast(connection => connection.role === 'mobile' && connection.upstream.readyState === WebSocket.OPEN);
  phonePath.dropped = true;
  const phoneStates = [];
  const stopPhoneStates = mobile.subscribe(state => phoneStates.push(state));
  await until(() => phoneStates.some(state => state.status === 'error' && /stopped responding/.test(state.error ?? '')), 'phone heartbeat detects blackhole', 30_000);
  await pending;
  assert.equal(settled, true); assert.ok(result instanceof Error);
  releaseSend();
  await until(connected, 'phone reconnect after stale slot expires');
  assert.equal(desktop.invitation, invitation);
  assert.equal(desktop.getPeerControllerPublicKey(), mobileKey);
  assert.equal(sends, 1, 'ambiguous accepted mutation was not replayed');
  assert.equal(approvals, 0);
  stopPhoneStates();
  console.log('Phone network loss recovered with saved identity and no mutation replay.');

  // Either side can move. Repeat the silent blackhole on the Mac's connection.
  const desktopPath = connections.findLast(connection => connection.role === 'desktop' && connection.upstream.readyState === WebSocket.OPEN);
  desktopPath.dropped = true;
  const desktopStates = [];
  const stopDesktopStates = desktop.subscribe(state => desktopStates.push(state));
  await until(() => desktopStates.some(state => state.status === 'error' && /stopped responding/.test(state.error ?? '')), 'desktop heartbeat detects blackhole', 30_000);
  await until(connected, 'desktop reconnect after stale slot expires');
  assert.deepEqual(await mobile.getSnapshot(), initialSnapshot);
  assert.equal(desktop.invitation, invitation); assert.equal(sends, 1);
  stopDesktopStates();
  console.log('Desktop network loss recovered with the same pairing.');

  const exited = once(relay.process, 'exit'); relay.process.kill(); await exited;
  await until(() => !connected(), 'relay interruption', 5_000);
  relay = await launchRelay(relayPort);
  await until(connected, 'relay restart');
  assert.deepEqual(await mobile.getSnapshot(), initialSnapshot);
  assert.equal(desktop.invitation, invitation); assert.equal(sends, 1);
  console.log('Relay restart recovered without re-pairing. Network recovery assertions passed.');
} finally {
  releaseSend(); mobile?.close(); desktop?.close();
  for (const { client, upstream } of connections) { client.terminate(); upstream.terminate(); }
  gateway?.close(); relay?.process.kill();
}
