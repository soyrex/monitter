import assert from 'node:assert/strict';
import { mock } from 'node:test';
import { spawn } from 'node:child_process';
import WebSocket, { WebSocketServer } from 'ws';
import { createDesktopSession, createMobileSession } from '../src/lib/controller/remote-client.ts';
import { controllerSnapshot } from '../src/lib/controller/dispatcher.ts';
import { SecureChannel, base64urlToBytes, bytesToBase64url, deriveDirectionalKeys, deriveEcdhSecret, exportPairingPublicKey, generatePairingKeyPair, pairingCommitment, randomBytes } from '../src/lib/controller/secure-session.ts';

globalThis.WebSocket = WebSocket;
const once = (predicate, ms = 5_000) => new Promise((resolve, reject) => {
  const origin = new Error('Timed out waiting for transport state.');
  let stop = () => {};
  const timer = setTimeout(() => { stop(); reject(origin); }, ms);
  stop = predicate(value => { clearTimeout(timer); queueMicrotask(() => stop()); resolve(value); });
});
const relayReady = relay => new Promise((resolve, reject) => {
  let output = '';
  const timer = setTimeout(() => reject(new Error('Relay did not become ready.')), 5_000);
  relay.stdout.setEncoding('utf8');
  relay.stdout.on('data', chunk => {
    output += chunk;
    const match = output.match(/^Monitter opaque relay listening on (ws:\/\/127\.0\.0\.1:\d+)\n$/);
    if (match) { clearTimeout(timer); resolve({ relayUrl: match[1], output }); }
    else if (output.length > 256) { clearTimeout(timer); reject(new Error('Unexpected relay stdout.')); }
  });
  relay.once('error', reject);
  relay.once('exit', code => { if (!output) reject(new Error(`Relay exited before ready (${code}).`)); });
});
const rejectionProbe = async (relayUrl, bridge, mode) => {
  const desktop = await createDesktopSession(relayUrl, bridge);
  const descriptor = JSON.parse(desktop.invitation);
  const rejected = once(callback => desktop.subscribe(state => { if (state.status === 'error') callback(state); }));
  const peer = new WebSocket(relayUrl);
  const ownPublic = randomBytes(65); ownPublic[0] = 4;
  const ownNonce = randomBytes(16);
  const commitment = await pairingCommitment('mobile', ownPublic, ownNonce);
  peer.onopen = () => peer.send(JSON.stringify({ type: 'join', room: descriptor.room, role: 'mobile' }));
  peer.onmessage = event => {
    const message = JSON.parse(event.data.toString());
    if (message.type === 'peer') {
      peer.send(JSON.stringify({ type: 'hello', phase: 'commit', nonce: bytesToBase64url(commitment) }));
      if (mode === 'duplicate') peer.send(JSON.stringify({ type: 'hello', phase: 'commit', nonce: bytesToBase64url(randomBytes(32)) }));
    }
    if (message.type === 'hello' && message.phase === 'reveal' && mode === 'altered-reveal') {
      peer.send(JSON.stringify({ type: 'hello', phase: 'reveal', nonce: bytesToBase64url(randomBytes(16)), publicKey: bytesToBase64url(ownPublic) }));
    }
  };
  await rejected;
  peer.close(); desktop.close();
};

// A native-compatible direct host exercises the exact private-LAN handshake:
// join, mutual key commitment/reveal, pinned ECDH, then an encrypted pair proof.
const directHost = async () => {
  const keyPair = await generatePairingKeyPair();
  const publicKey = await exportPairingPublicKey(keyPair.publicKey);
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await new Promise((resolve, reject) => { server.once('listening', resolve); server.once('error', reject); });
  const port = server.address().port;
  const sockets = new Set();
  let closed = false;
  let connections = 0;
  server.on('connection', socket => {
    sockets.add(socket); socket.once('close', () => sockets.delete(socket));
    connections += 1;
    const nonce = randomBytes(16);
    let peerCommitment = null, channel = null;
    let queue = Promise.resolve();
    const send = value => socket.send(JSON.stringify(value));
    const receive = async raw => {
      const value = JSON.parse(raw.toString());
      if (value.type === 'join') {
        assert.equal(value.role, 'mobile');
        send({ type: 'joined', capabilities: ['relay-heartbeat-v1'] });
        send({ type: 'peer' });
        send({ type: 'hello', phase: 'commit', nonce: bytesToBase64url(await pairingCommitment('desktop', publicKey, nonce)) });
        return;
      }
      if (value.type === 'ping') { send({ type: 'pong' }); return; }
      if (value.type === 'hello' && value.phase === 'commit') {
        peerCommitment = base64urlToBytes(value.nonce);
        send({ type: 'hello', phase: 'reveal', nonce: bytesToBase64url(nonce), publicKey: bytesToBase64url(publicKey) });
        return;
      }
      if (value.type === 'hello' && value.phase === 'reveal') {
        const peerNonce = base64urlToBytes(value.nonce), peerPublicKey = base64urlToBytes(value.publicKey);
        assert.deepEqual(peerCommitment, await pairingCommitment('mobile', peerPublicKey, peerNonce));
        const secret = await deriveEcdhSecret(keyPair.privateKey, peerPublicKey);
        channel = new SecureChannel(await deriveDirectionalKeys(secret, nonce, peerNonce, 'desktop', 2));
        return;
      }
      if (value.type === 'secure') {
        assert.ok(channel, 'direct secure frame follows key agreement');
        assert.equal((await channel.open(value)).type, 'pair-request');
        send(await channel.seal({ type: 'pair-request' }));
        return;
      }
      throw new Error('Unexpected direct test frame.');
    };
    socket.on('message', raw => { queue = queue.then(() => receive(raw)); queue.catch(() => socket.terminate()); });
  });
  return {
    publicKey: bytesToBase64url(publicKey), port,
    connections: () => connections,
    close: () => {
      if (closed) return Promise.resolve();
      closed = true;
      for (const socket of sockets) socket.terminate();
      return new Promise(resolve => server.close(resolve));
    },
  };
};

// Browser security requires literal private IPv4 candidates. Tests route those
// addresses to loopback while retaining and asserting the production URL order.
const routedSockets = [];
class RoutedWebSocket extends WebSocket {
  constructor(url, protocols) {
    const original = String(url); routedSockets.push(original);
    const routed = new URL(original);
    if (/^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(routed.hostname)) routed.hostname = '127.0.0.1';
    super(routed, protocols);
  }
}

// A dead TCP route or a socket that opens without joining must not leave either
// endpoint spinning for the browser's multi-minute connection timeout.
const stalledSockets=[];
class StalledSocket {
  static OPEN=1;
  readyState=0;
  sent=[];
  constructor(){stalledSockets.push(this);}
  send(value){this.sent.push(value);}
  close(){this.readyState=3;this.onclose?.();}
}
globalThis.WebSocket=StalledSocket;
mock.timers.enable({apis:['setTimeout']});
try {
  const desktop=await createDesktopSession('wss://relay.example/relay',{});
  const mobile=await createMobileSession(desktop.invitation);
  const states=[];
  desktop.subscribe(state=>states.push(state));
  const [desktopSocket,mobileSocket]=stalledSockets;
  mobileSocket.readyState=1;mobileSocket.onopen();
  assert.equal(mobileSocket.sent.length,1);
  mock.timers.tick(14999);
  assert.equal(desktop.getStatus(),'connecting');
  mock.timers.tick(1);
  assert.equal(desktop.getStatus(),'error');
  assert.equal(mobile.getStatus(),'error');
  assert.match(states.at(-1).error,/Could not connect to the pairing relay at relay.example/);
  let replay;desktop.subscribe(state=>{replay=state;});
  assert.equal(replay.error,states.at(-1).error);
  // Late events from the timed-out attempt cannot advertise a usable invitation.
  desktopSocket.readyState=1;desktopSocket.onopen();
  desktopSocket.onmessage({data:JSON.stringify({type:'joined'})});
  await Promise.resolve();
  assert.equal(desktopSocket.sent.length,0);
  assert.equal(desktop.getStatus(),'error');
  desktop.close();mobile.close();

  const healthy=await createDesktopSession('wss://relay.example/relay',{});
  const healthySocket=stalledSockets.at(-1);
  healthySocket.readyState=1;healthySocket.onopen();
  healthySocket.onmessage({data:JSON.stringify({type:'joined'})});
  await Promise.resolve();
  assert.equal(healthy.getStatus(),'waiting_for_peer');
  mock.timers.tick(15000);
  assert.equal(healthy.getStatus(),'waiting_for_peer');
  assert.equal(healthySocket.sent.length,1,'legacy relays never receive a new heartbeat frame');
  healthy.close();

  const heartbeat=await createDesktopSession('wss://relay.example/relay',{});
  const heartbeatSocket=stalledSockets.at(-1);
  heartbeatSocket.readyState=1;heartbeatSocket.onopen();
  assert.ok(JSON.parse(heartbeatSocket.sent[0]).capabilities.includes('relay-heartbeat-v1'));
  heartbeatSocket.onmessage({data:JSON.stringify({type:'joined',capabilities:['relay-heartbeat-v1']})});
  await Promise.resolve();
  assert.deepEqual(JSON.parse(heartbeatSocket.sent.at(-1)),{type:'ping'});
  // A transport acknowledgement is not pairing or desktop approval.
  heartbeatSocket.onmessage({data:'{"type":"pong"}'});
  assert.equal(heartbeat.getStatus(),'waiting_for_peer');
  mock.timers.tick(14999);
  assert.equal(heartbeatSocket.sent.length,2);
  mock.timers.tick(1);
  assert.equal(heartbeatSocket.sent.length,3);
  // Model an OPEN socket whose path and close handshake are blackholed.
  heartbeatSocket.close=()=>{heartbeatSocket.readyState=2;};
  mock.timers.tick(9999);
  assert.equal(heartbeat.getStatus(),'waiting_for_peer');
  mock.timers.tick(1);
  assert.equal(heartbeat.getStatus(),'error');
  heartbeatSocket.onmessage({data:'{"type":"pong"}'});
  heartbeatSocket.onmessage({data:'{"type":"joined"}'});
  await Promise.resolve();
  mock.timers.tick(60000);
  assert.equal(heartbeat.getStatus(),'error','late events cannot revive a dead session');
  assert.equal(heartbeatSocket.sent.length,3,'dead sessions stop sending heartbeats');

  const stopped=await createDesktopSession('wss://relay.example/relay',{});
  const stoppedSocket=stalledSockets.at(-1);
  stoppedSocket.readyState=1;stoppedSocket.onopen();
  stoppedSocket.onmessage({data:'{"type":"joined","capabilities":["relay-heartbeat-v1"]}'});
  await Promise.resolve();
  stopped.close();
  mock.timers.tick(60000);
  assert.equal(stopped.getStatus(),'closed','intentional close cancels heartbeat deadlines');
} finally {mock.timers.reset();globalThis.WebSocket=WebSocket;}

// Directional keys must decrypt only in their intended direction and refuse replay.
const secret = randomBytes(32), desktopNonce = randomBytes(16), mobileNonce = randomBytes(16);
const desktopCrypto = new SecureChannel(await deriveDirectionalKeys(secret, desktopNonce, mobileNonce, 'desktop'));
const mobileCrypto = new SecureChannel(await deriveDirectionalKeys(secret, desktopNonce, mobileNonce, 'mobile'));
const envelope = await desktopCrypto.seal({ proof: 'opaque' });
assert.deepEqual(await mobileCrypto.open(envelope), { proof: 'opaque' });
await assert.rejects(() => mobileCrypto.open(envelope), /replayed|out-of-order/);
await assert.rejects(() => desktopCrypto.open(envelope));

const relay = spawn(process.execPath, ['scripts/relay-server.mjs'], { cwd: process.cwd(), env: { ...process.env, MONITTER_RELAY_PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
try {
  const { relayUrl, output } = await relayReady(relay);
  assert.match(output, /^Monitter opaque relay listening on ws:\/\/127\.0\.0\.1:\d+\n$/);
  await assert.rejects(() => createMobileSession('not an invitation'), /Invalid controller invitation/);
  const snapshot = { hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], settings: { accent: '#000', theme: 'dark', interfaceScale: 125, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' } };
  let sends = 0;
  const bridge = { getSnapshot: async () => snapshot, sendMessage: async () => { sends += 1; return snapshot; }, cancelTask: async () => snapshot, resumeTask: async () => snapshot, listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }) };

  const direct = await directHost();
  globalThis.WebSocket = RoutedWebSocket;
  try {
    routedSockets.length = 0;
    const directInvitation = JSON.stringify({
      version: 2, relayUrl, room: bytesToBase64url(randomBytes(18)), publicKey: direct.publicKey,
      directUrls: [`ws://192.168.10.1:${direct.port}/pairing/direct`],
    });
    const directMobile = await createMobileSession(directInvitation);
    await once(callback => directMobile.subscribe(state => { if (state.status === 'awaiting_approval') callback(state); }));
    assert.equal(direct.connections(), 1);
    assert.match(routedSockets[0], /^ws:\/\/192\.168\.10\.1:\d+\/pairing\/direct\?/);
    assert.equal(routedSockets.length, 1, 'an authenticated direct candidate avoids opening the relay');
    directMobile.close();
    await direct.close();

    const fallbackDesktop = await createDesktopSession(relayUrl, bridge);
    const fallbackInvitation = JSON.parse(fallbackDesktop.invitation);
    fallbackInvitation.directUrls = [`ws://192.168.10.1:${direct.port}/pairing/direct`];
    routedSockets.length = 0;
    const fallbackPending = once(callback => fallbackDesktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
    const fallbackMobile = await createMobileSession(JSON.stringify(fallbackInvitation));
    await fallbackPending;
    assert.match(routedSockets[0], /^ws:\/\/192\.168\.10\.1:\d+\/pairing\/direct\?/);
    assert.equal(new URL(routedSockets[1]).origin, new URL(relayUrl).origin, 'relay opens only after the direct candidate fails');
    await fallbackDesktop.approve();
    await once(callback => fallbackMobile.subscribe(state => { if (state.status === 'connected') callback(state); }));
    fallbackMobile.close(); fallbackDesktop.close();
  } finally { await direct.close(); globalThis.WebSocket = WebSocket; }

  const desktop = await createDesktopSession(relayUrl, bridge);
  const descriptor = JSON.parse(desktop.invitation);
  assert.deepEqual(Object.keys(descriptor).sort(), ['publicKey', 'relayUrl', 'room', 'version']);
  assert.equal(descriptor.version, 2);
  const pending = once(callback => desktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
  const mobile = await createMobileSession(desktop.invitation);
  await pending;
  assert.match(desktop.getVerificationCode() ?? '', /^\d{6}$/);
  assert.equal(mobile.getVerificationCode(), desktop.getVerificationCode());
  await assert.rejects(() => mobile.getSnapshot(), /approval and a live encrypted connection/);
  assert.equal(sends, 0);
  await desktop.approve();
  await once(callback => mobile.subscribe(state => { if (state.status === 'connected') callback(state); }));
  assert.equal(mobile.supportsOwnerWorkspace(), false);
  await assert.rejects(() => mobile.requestOwnerWorkspace(), /Update Monitter on the desktop/);
  await mobile.sendMessage('11111111-1111-4111-8111-111111111111', 'encrypted request');
  assert.equal(sends, 1);
  const desktopClosed = once(callback => desktop.subscribe(state => { if (state.status === 'closed') callback(state); }));
  mobile.close(); await desktopClosed; desktop.close();

  // Opening/decrypting is ordered, but a slow desktop Snapshot must not hold
  // later authenticated work or peer_left in the receive queue. The modern
  // mobile capability receives a tiny receipt while the legacy-shaped read is
  // still stalled; no send is replayed to obtain that receipt.
  let snapshotStarted;
  let releaseSnapshot;
  const snapshotStartedPromise = new Promise(resolve => { snapshotStarted = resolve; });
  const delayedSnapshot = new Promise(resolve => { releaseSnapshot = resolve; });
  let delayedCancels = 0, delayedSends = 0;
  const delayedBridge = {
    ...bridge,
    getSnapshot: async () => { snapshotStarted(); await delayedSnapshot; return snapshot; },
    sendMessage: async () => { delayedSends += 1; return { accepted: true }; },
    cancelTask: async () => { delayedCancels += 1; return snapshot; },
  };
  const stalledDesktop = await createDesktopSession(relayUrl, delayedBridge);
  const stalledPending = once(callback => stalledDesktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
  const stalledMobile = await createMobileSession(stalledDesktop.invitation);
  await stalledPending; await stalledDesktop.approve();
  await once(callback => stalledMobile.subscribe(state => { if (state.status === 'connected') callback(state); }));
  const stalledRead = stalledMobile.getSnapshot();
  // Attach the rejection observer before closing so Node does not treat the
  // intentional security-close rejection as an unhandled promise.
  const stalledReadOutcome = stalledRead.then(() => null, reason => reason);
  await snapshotStartedPromise;
  const cancellation = await stalledMobile.cancelTask('11111111-1111-4111-8111-111111111111');
  assert.deepEqual(cancellation, controllerSnapshot(snapshot));
  const receipt = await stalledMobile.sendMessage('11111111-1111-4111-8111-111111111111', 'receipt first');
  assert.deepEqual(receipt, { accepted: true });
  assert.equal(delayedCancels, 1); assert.equal(delayedSends, 1);
  const stalledClosed = once(callback => stalledDesktop.subscribe(state => { if (state.status === 'closed') callback(state); }));
  stalledMobile.close(); await stalledClosed;
  releaseSnapshot();
  assert.match(String(await stalledReadOutcome), /Connection closed/);
  stalledDesktop.close();

  // A collaboration visitor declares a bounded display identity inside the
  // encrypted pairing request; the host sees it only before explicit approval.
  const sharingDesktop = await createDesktopSession(relayUrl, bridge);
  const sharingPending = once(callback => sharingDesktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
  const visitor = await createMobileSession(sharingDesktop.invitation, { name: 'Luke', role: 'visitor' });
  await sharingPending;
  assert.deepEqual(sharingDesktop.getPeer(), { name: 'Luke', role: 'visitor' });
  await sharingDesktop.approve();
  await once(callback => visitor.subscribe(state => { if (state.status === 'connected') callback(state); }));
  visitor.close(); sharingDesktop.close();

  // WebCrypto can complete after a network failure or an explicit close. A
  // dropped approval must reject its caller, and a late decrypted rejection
  // cannot overwrite the state of an already-disconnected session.
  const sealingDesktop = await createDesktopSession(relayUrl, bridge);
  const sealingPending = once(callback => sealingDesktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
  const sealingMobile = await createMobileSession(sealingDesktop.invitation);
  await sealingPending;
  const originalSeal = SecureChannel.prototype.seal;
  let sealStarted, releaseSeal;
  const sealingStarted = new Promise(resolve => { sealStarted = resolve; });
  const sealingGate = new Promise(resolve => { releaseSeal = resolve; });
  SecureChannel.prototype.seal = async function(value) {
    const encrypted = await originalSeal.call(this, value);
    if (value.type === 'approved') { sealStarted(); await sealingGate; }
    return encrypted;
  };
  try {
    const approving = sealingDesktop.approve();
    const outcome = approving.then(() => null, error => error);
    await sealingStarted;
    sealingDesktop.close(); releaseSeal();
    assert.match(String(await outcome), /Connection closed/);
    assert.equal(sealingDesktop.getStatus(), 'closed');
  } finally { releaseSeal(); SecureChannel.prototype.seal = originalSeal; sealingDesktop.close(); sealingMobile.close(); }

  const decryptingDesktop = await createDesktopSession(relayUrl, bridge);
  const decryptingPending = once(callback => decryptingDesktop.subscribe(state => { if (state.status === 'pending') callback(state); }));
  const decryptingMobile = await createMobileSession(decryptingDesktop.invitation);
  await decryptingPending;
  const originalOpen = SecureChannel.prototype.open;
  let decryptStarted, releaseDecrypt;
  const decryptingStarted = new Promise(resolve => { decryptStarted = resolve; });
  const decryptingGate = new Promise(resolve => { releaseDecrypt = resolve; });
  SecureChannel.prototype.open = async function(value) {
    const decrypted = await originalOpen.call(this, value);
    if (decrypted.type === 'rejected') { decryptStarted(); await decryptingGate; }
    return decrypted;
  };
  try {
    await decryptingDesktop.reject(); await decryptingStarted;
    decryptingMobile.close(); releaseDecrypt();
    await new Promise(resolve => setTimeout(resolve, 20));
    assert.equal(decryptingMobile.getStatus(), 'closed', 'late decrypted frames cannot change terminal state');
  } finally { releaseDecrypt(); SecureChannel.prototype.open = originalOpen; decryptingMobile.close(); decryptingDesktop.close(); }

  // v1 descriptors remain accepted for mobile compatibility with existing sessions.
  const legacy = await createMobileSession(JSON.stringify({ version: 1, relayUrl, room: 'abcdefghijklmnopqrstuv', secret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' }));
  legacy.close();

  // A QR whose pinned desktop key has been substituted cannot complete pairing.
  const secureDesktop = await createDesktopSession(relayUrl, bridge);
  const tampered = JSON.parse(secureDesktop.invitation);
  tampered.publicKey = Buffer.from(randomBytes(65)).toString('base64url'); tampered.publicKey = `B${tampered.publicKey.slice(1)}`;
  const badMobile = await createMobileSession(JSON.stringify(tampered));
  await once(callback => badMobile.subscribe(state => { if (state.status === 'error') callback(state); }));
  badMobile.close(); secureDesktop.close();

  await rejectionProbe(relayUrl, bridge, 'altered-reveal');
  await rejectionProbe(relayUrl, bridge, 'duplicate');
  console.log('Secure relay transport assertions passed.');
} finally {
  relay.kill();
}
