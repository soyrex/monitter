import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import WebSocket from 'ws';
import { createDesktopSession, createMobileSession } from '../src/lib/controller/remote-client.ts';
import { SecureChannel, bytesToBase64url, deriveDirectionalKeys, pairingCommitment, randomBytes } from '../src/lib/controller/secure-session.ts';

globalThis.WebSocket = WebSocket;
const once = (predicate, ms = 5_000) => new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error('Timed out waiting for transport state.')), ms);
  const stop = predicate(value => { clearTimeout(timer); stop(); resolve(value); });
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
  await mobile.sendMessage('11111111-1111-4111-8111-111111111111', 'encrypted request');
  assert.equal(sends, 1);
  const desktopClosed = once(callback => desktop.subscribe(state => { if (state.status === 'closed') callback(state); }));
  mobile.close(); await desktopClosed; desktop.close();

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
