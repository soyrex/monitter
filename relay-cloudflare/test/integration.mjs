import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { isIP } from 'node:net';
import { build } from 'esbuild';
import WebSocket from 'ws';

const root = new URL('../', import.meta.url).pathname;
const monitter = join(root, '..');
const room = 'A'.repeat(22);
const port = 18000 + Math.floor(Math.random() * 1000);
const externalRelayUrl = process.env.MONITTER_TEST_RELAY_URL;
if (externalRelayUrl && !/^wss:\/\/[^/]+\/relay$/.test(externalRelayUrl)) throw new Error('MONITTER_TEST_RELAY_URL must be a wss:// host ending in /relay.');
const testResolveIp = process.env.MONITTER_TEST_RESOLVE_IP;
if (testResolveIp && (!externalRelayUrl || isIP(testResolveIp) === 0)) throw new Error('MONITTER_TEST_RESOLVE_IP requires MONITTER_TEST_RELAY_URL and a literal IP address.');
const relayUrl = externalRelayUrl ?? `ws://127.0.0.1:${port}/relay`;
const httpOrigin = `http://127.0.0.1:${port}`;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const debug = (...values) => { if (process.env.MONITTER_TEST_DEBUG) console.error(...values); };
const within = (promise, ms, label) => new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error(`Timed out waiting for ${label}.`)), ms);
  promise.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
});
const testIntervals = new Set();
const opens = url => new Promise((resolve, reject) => {
  const socket = new WebSocket(url);
  socket.once('open', () => resolve(socket));
  socket.once('unexpected-response', (_request, response) => reject(new Error(`unexpected response ${response.statusCode}`)));
  socket.once('error', reject);
});
const closes = socket => new Promise(resolve => socket.once('close', (code, reason) => resolve({ code, reason: String(reason) })));
const messages = socket => new Promise(resolve => socket.once('message', value => resolve(JSON.parse(String(value)))));
const inbox = socket => {
  const frames = [];
  const listeners = new Set();
  socket.on('message', value => {
    frames.push(JSON.parse(String(value)));
    for (const listener of listeners) listener();
  });
  const take = predicate => within(new Promise(resolve => {
    const check = () => {
      const index = frames.findIndex(predicate);
      if (index < 0) return;
      listeners.delete(check);
      resolve(frames.splice(index, 1)[0]);
    };
    listeners.add(check);
    check();
  }), 8_000, 'relay frame');
  return {
    next: () => take(() => true),
    nextMatching: predicate => take(predicate),
  };
};
const waitForStatus = (session, expected, ms = 8_000) => new Promise((resolve, reject) => {
  let unsubscribe = () => {};
  let settled = false;
  const finish = callback => { if (settled) return; settled = true; clearTimeout(timer); unsubscribe(); callback(); };
  const timer = setTimeout(() => finish(() => reject(new Error(`Timed out waiting for ${expected}.`))), ms);
  const actualUnsubscribe = session.subscribe(state => {
    if (state.status === expected) { finish(resolve); return; }
    if (['closed', 'error', 'rejected'].includes(state.status)) {
      finish(() => reject(new Error(`Connection ended as ${state.status}${state.error ? `: ${state.error}` : ''}.`)));
    }
  });
  unsubscribe = actualUnsubscribe;
  if (settled) unsubscribe();
});

const workerState = externalRelayUrl ? null : await mkdtemp(join(tmpdir(), 'monitter-cloudflare-state-'));
const worker = externalRelayUrl ? null : spawn(join(root, 'node_modules/.bin/wrangler'), ['dev', '--local', '--port', String(port), '--config', 'wrangler.jsonc', '--persist-to', workerState], {
  cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
});
let workerError = '';
worker?.stderr.on('data', data => { workerError = (workerError + String(data)).slice(-4000); });
worker?.stdout.on('data', data => { workerError = (workerError + String(data)).slice(-4000); });
try {
  if (!externalRelayUrl) {
    for (let attempt = 0; attempt < 480; attempt += 1) {
      try { if ((await fetch(`http://127.0.0.1:${port}/health`)).ok) break; } catch { /* still starting */ }
      await delay(125);
      if (attempt === 479) throw new Error(`local Worker did not start: ${workerError}`);
    }
    assert.equal((await fetch(`http://127.0.0.1:${port}/relay`)).status, 426);
    const sharePreflight = await fetch(`${httpOrigin}/pairing`, { method: 'OPTIONS', headers: { Origin: 'https://share.monitter.com' } });
    assert.equal(sharePreflight.status, 204);
    assert.equal(sharePreflight.headers.get('Access-Control-Allow-Origin'), 'https://share.monitter.com');
    for (const origin of ['https://share.monitter.com.evil.example', 'http://share.monitter.com', 'https://evil.example']) {
      assert.equal((await fetch(`${httpOrigin}/pairing`, { method: 'OPTIONS', headers: { Origin: origin } })).status, 403);
    }
    assert.equal((await fetch(`http://127.0.0.1:${port}/relay?room=bad&role=desktop`)).status, 426);

    const publicKey = Buffer.concat([Buffer.from([4]), Buffer.alloc(64)]).toString('base64url');
    const invitation = room => ({ version: 2, relayUrl, room, publicKey });
    const create = async (body, ip = '198.51.100.1') => {
      const result = await fetch(`${httpOrigin}/pairing`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'CF-Connecting-IP': ip, Origin: 'http://localhost:4173' }, body: JSON.stringify(body) });
      const text = await result.text();
      let parsed = null; try { parsed = JSON.parse(text || 'null'); } catch { /* non-JSON error body */ }
      return { result, body: parsed, text };
    };
    const rejectedOrigin = await fetch(`${httpOrigin}/pairing`, { method: 'POST', headers: { Origin: 'https://example.invalid' } });
    assert.equal(rejectedOrigin.status, 403);
    const malformed = await create({ invitation: { ...invitation('D'.repeat(22)), secret: 'must-not-be-persisted' } });
    assert.equal(malformed.result.status, 400);
    const first = await create({ invitation: invitation('D'.repeat(22)) });
    assert.equal(first.result.status, 201, first.text);
    assert.match(first.body.code, /^\d{9}$/);
    assert.equal(first.body.expiresAt - Date.now() > 4 * 60_000, true);
    assert.equal(first.body.expiresAt - Date.now() <= 5 * 60_000, true);
    assert.equal(first.result.headers.get('Access-Control-Allow-Origin'), 'http://localhost:4173');
    const [claimA, claimB] = await Promise.all([
      fetch(`${httpOrigin}/pairing/${first.body.code}`, { headers: { 'CF-Connecting-IP': '198.51.100.2' } }),
      fetch(`${httpOrigin}/pairing/${first.body.code}`, { headers: { 'CF-Connecting-IP': '198.51.100.3' } }),
    ]);
    assert.deepEqual([claimA.status, claimB.status].sort(), [200, 404]);
    const claimed = await (claimA.status === 200 ? claimA : claimB).json();
    assert.deepEqual(claimed.invitation, invitation('D'.repeat(22)));
    assert.equal(JSON.stringify(claimed).includes('secret'), false);
    const revocable = await create({ invitation: invitation('E'.repeat(22)) }, '198.51.100.4');
    assert.equal(revocable.result.status, 201);
    const revoked = await fetch(`${httpOrigin}/pairing/${revocable.body.code}`, { method: 'DELETE', headers: { Authorization: `Bearer ${revocable.body.revocationToken}`, 'CF-Connecting-IP': '198.51.100.5' } });
    assert.equal(revoked.status, 204);
    assert.equal((await fetch(`${httpOrigin}/pairing/${revocable.body.code}`, { headers: { 'CF-Connecting-IP': '198.51.100.6' } })).status, 404);
    for (let count = 0; count < 5; count += 1) await create({ invitation: invitation(`${count}`.repeat(22)) }, '198.51.100.7');
    assert.equal((await create({ invitation: invitation('Z'.repeat(22)) }, '198.51.100.7')).result.status, 429);

    // Query routing and the later join envelope must agree; a routing ID alone
    // is never accepted as a pairing secret.
    const mismatch = await opens(`${relayUrl}?room=${room}&role=desktop`);
    mismatch.send(JSON.stringify({ type: 'join', room: 'B'.repeat(22), role: 'desktop' }));
    assert.equal((await closes(mismatch)).code, 1008);

    const lone = await opens(`${relayUrl}?room=${room}&role=desktop`);
    await assert.rejects(opens(`${relayUrl}?room=${room}&role=desktop`), /409/);
    lone.close();

    // An unjoined reservation is expired by the Durable Object alarm, rather
    // than retaining a public room indefinitely.
    const timeout = await opens(`${relayUrl}?room=${'C'.repeat(22)}&role=mobile`);
    // Miniflare may expose an alarm-driven server close as 1006 even though
    // the edge runtime emits its policy close code. Closure is the invariant.
    debug('waiting for unjoined timeout');
    assert.ok([1006, 1008].includes((await closes(timeout)).code));
    debug('unjoined timeout closed');

    // The hibernating auto-response is negotiated, exact, and lease-backed.
    // These run together so the real 45-second lease costs one local wait.
    const staleRoom = 'S'.repeat(22);
    const staleDesktop = await opens(`${relayUrl}?room=${staleRoom}&role=desktop`);
    const staleDesktopInbox = inbox(staleDesktop);
    staleDesktop.send(JSON.stringify({ type: 'join', room: staleRoom, role: 'desktop', capabilities: ['relay-heartbeat-v1'] }));
    assert.deepEqual(await staleDesktopInbox.next(), { type: 'joined', capabilities: ['relay-heartbeat-v1'] });
    const staleMobile = await opens(`${relayUrl}?room=${staleRoom}&role=mobile`);
    const staleMobileInbox = inbox(staleMobile);
    const staleMobileClosed = closes(staleMobile);
    let staleMobilePeerLeft = 0;
    const staleMobilePongs = [];
    staleMobile.on('message', value => {
      const frame = JSON.parse(String(value));
      if (frame.type === 'peer_left') staleMobilePeerLeft += 1;
      if (frame.type === 'pong') staleMobilePongs.push(Date.now());
    });
    staleMobile.send(JSON.stringify({ type: 'join', room: staleRoom, role: 'mobile', capabilities: ['relay-heartbeat-v1'] }));
    assert.deepEqual(await staleMobileInbox.next(), { type: 'joined', capabilities: ['relay-heartbeat-v1'] });
    assert.deepEqual(await staleDesktopInbox.next(), { type: 'peer' });
    assert.deepEqual(await staleMobileInbox.next(), { type: 'peer' });
    staleMobile.send('{"type":"ping"}');
    assert.deepEqual(await staleMobileInbox.next(), { type: 'pong' });
    const staleMobileKeepAlive = setInterval(() => staleMobile.send('{"type":"ping"}'), 15_000);
    testIntervals.add(staleMobileKeepAlive);
    const staleClosed = closes(staleDesktop);

    const liveRoom = 'L'.repeat(22);
    const liveDesktop = await opens(`${relayUrl}?room=${liveRoom}&role=desktop`);
    const liveDesktopInbox = inbox(liveDesktop);
    liveDesktop.send(JSON.stringify({ type: 'join', room: liveRoom, role: 'desktop', capabilities: ['relay-heartbeat-v1'] }));
    assert.deepEqual(await liveDesktopInbox.next(), { type: 'joined', capabilities: ['relay-heartbeat-v1'] });
    const liveMobile = await opens(`${relayUrl}?room=${liveRoom}&role=mobile`);
    const liveMobileInbox = inbox(liveMobile);
    liveMobile.send(JSON.stringify({ type: 'join', room: liveRoom, role: 'mobile', capabilities: ['relay-heartbeat-v1'] }));
    assert.deepEqual(await liveMobileInbox.next(), { type: 'joined', capabilities: ['relay-heartbeat-v1'] });
    assert.deepEqual(await liveDesktopInbox.next(), { type: 'peer' });
    assert.deepEqual(await liveMobileInbox.next(), { type: 'peer' });
    liveDesktop.send('{"type":"ping"}');
    assert.deepEqual(await liveDesktopInbox.next(), { type: 'pong' });
    const keepAlive = setInterval(() => liveDesktop.send('{"type":"ping"}'), 15_000);
    testIntervals.add(keepAlive);

    const legacyRoom = 'G'.repeat(22);
    const legacyDesktop = await opens(`${relayUrl}?room=${legacyRoom}&role=desktop`);
    const legacyDesktopInbox = inbox(legacyDesktop);
    legacyDesktop.send(JSON.stringify({ type: 'join', room: legacyRoom, role: 'desktop' }));
    assert.deepEqual(await legacyDesktopInbox.next(), { type: 'joined' });
    const legacyMobile = await opens(`${relayUrl}?room=${legacyRoom}&role=mobile`);
    const legacyMobileInbox = inbox(legacyMobile);
    legacyMobile.send(JSON.stringify({ type: 'join', room: legacyRoom, role: 'mobile' }));
    assert.deepEqual(await legacyMobileInbox.next(), { type: 'joined' });
    assert.deepEqual(await legacyDesktopInbox.next(), { type: 'peer' });
    assert.deepEqual(await legacyMobileInbox.next(), { type: 'peer' });

    // The 15-second production cadence must have received two scheduled pongs
    // before the desktop's initial 45-second lease reaches its alarm.
    await delay(35_000);
    assert.ok(staleMobilePongs.length >= 3, `expected initial plus two 15s pongs, received ${staleMobilePongs.length}: ${staleMobilePongs.join(',')}`);

    // A negotiated stale reservation is removed by its alarm. A live role
    // cannot be displaced, while a fully legacy pair is still long-lived.
    debug('waiting for negotiated stale close');
    assert.ok([1006, 1008].includes((await within(staleClosed, 60_000, 'negotiated stale close')).code));
    debug('negotiated stale socket closed');
    const departure = await within(Promise.race([
      staleMobileInbox.nextMatching(frame => frame.type === 'peer_left').then(frame => ({ kind: 'peer_left', frame })),
      staleMobileClosed.then(close => ({ kind: 'closed', close })),
    ]), 8_000, 'stale peer departure');
    assert.equal(departure.kind, 'peer_left', `heartbeat peer closed instead of receiving peer_left after pongs ${JSON.stringify(staleMobilePongs)}: ${JSON.stringify(departure)}`);
    assert.deepEqual(departure.frame, { type: 'peer_left' });
    // The stale socket's close event and its peer's message are independent
    // WebSocket deliveries. Give the already-received frame one event turn
    // before asserting the observer count below.
    await delay(0);
    assert.equal(staleMobilePeerLeft, 1);
    clearInterval(keepAlive);
    testIntervals.delete(keepAlive);
    const replacement = await opens(`${relayUrl}?room=${staleRoom}&role=desktop`);
    const replacementInbox = inbox(replacement);
    replacement.send(JSON.stringify({ type: 'join', room: staleRoom, role: 'desktop', capabilities: ['relay-heartbeat-v1'] }));
    assert.deepEqual(await replacementInbox.next(), { type: 'joined', capabilities: ['relay-heartbeat-v1'] });
    assert.deepEqual(await staleMobileInbox.nextMatching(frame => frame.type === 'peer'), { type: 'peer' });
    await delay(250);
    assert.equal(staleMobilePeerLeft, 1);
    clearInterval(staleMobileKeepAlive);
    testIntervals.delete(staleMobileKeepAlive);
    replacement.close();
    staleMobile.close();
    await assert.rejects(opens(`${relayUrl}?room=${liveRoom}&role=desktop`), /409/);
    assert.equal(legacyDesktop.readyState, WebSocket.OPEN);
    assert.equal(legacyMobile.readyState, WebSocket.OPEN);
    liveDesktop.close();
    liveMobile.close();
    legacyDesktop.close();
    legacyMobile.close();
  }

  const temporary = await mkdtemp(join(tmpdir(), 'monitter-cloudflare-client-'));
  let desktop;
  let mobile;
  try {
    const bundle = join(temporary, 'remote-client.mjs');
    await build({ entryPoints: [join(monitter, 'src/lib/controller/remote-client.ts')], bundle: true, format: 'esm', platform: 'node', outfile: bundle });
    // This exercises Monitter's real secure session/controller client through
    // the local Worker. The production client normally supplies these query
    // params itself; this adapter keeps the test compatible with both states.
    let nextRole = 'desktop';
    globalThis.WebSocket = class RoutingSocket extends WebSocket {
      constructor(url) {
        const routed = new URL(url);
        if (!routed.searchParams.has('room')) routed.searchParams.set('room', room);
        if (!routed.searchParams.has('role')) routed.searchParams.set('role', nextRole);
        nextRole = 'mobile';
        // This test-only override bypasses a stale local DNS cache. `ws` still
        // connects with the URL hostname, preserving SNI and certificate checks.
        super(routed, testResolveIp ? {
          lookup: (_host, options, callback) => {
            const result = { address: testResolveIp, family: isIP(testResolveIp) };
            if (options.all) callback(null, [result]);
            else callback(null, result.address, result.family);
          },
        } : undefined);
      }
    };
    const { createDesktopSession, createMobileSession } = await import(pathToFileURL(bundle).href);
    const snapshot = { hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], settings: { accent: '#000', theme: 'dark', interfaceScale: 125, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' } };
    let sends = 0;
    const bridge = { getSnapshot: async () => snapshot, sendMessage: async () => { sends += 1; return snapshot; }, cancelTask: async () => snapshot, resumeTask: async () => snapshot, listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }) };
    desktop = await createDesktopSession(relayUrl, bridge);
    const pending = waitForStatus(desktop, 'pending');
    const api = relayUrl.replace(/^ws/, 'http').replace(/\/relay$/, '/pairing');
    const headers = {'Content-Type':'application/json', Origin:'tauri://localhost', ...(!externalRelayUrl ? {'CF-Connecting-IP':'198.51.100.70'} : {})};
    const registered = await fetch(api,{method:'POST',headers,body:JSON.stringify({invitation:JSON.parse(desktop.invitation)})});
    assert.equal(registered.status,201);
    assert.equal(registered.headers.get('Access-Control-Allow-Origin'),'tauri://localhost');
    const {code}=await registered.json();
    assert.match(code,/^\d{9}$/);
    const claimed = await fetch(api+'/'+code,{headers:{Origin:'https://appassets.androidplatform.net',...(!externalRelayUrl?{'CF-Connecting-IP':'198.51.100.71'}:{})}});
    assert.equal(claimed.status,200);
    assert.equal(claimed.headers.get('Cache-Control'),'no-store');
    assert.equal(claimed.headers.get('Access-Control-Allow-Origin'),'https://appassets.androidplatform.net');
    const resolved=await claimed.json();
    assert.equal('secret' in resolved.invitation,false);
    mobile = await createMobileSession(JSON.stringify(resolved.invitation));
    await pending;
    assert.equal(mobile.getVerificationCode(),desktop.getVerificationCode());
    await desktop.approve();
    await waitForStatus(mobile, 'connected');
    await mobile.sendMessage('11111111-1111-4111-8111-111111111111', 'encrypted relay proof');
    assert.equal(sends, 1);
  } finally {
    mobile?.close();
    desktop?.close();
    await rm(temporary, { recursive: true, force: true });
  }
  console.log('Cloudflare Durable Object relay integration assertions passed.');
} finally {
  for (const interval of testIntervals) clearInterval(interval);
  worker?.kill();
  if (worker) await new Promise(resolve => worker.once('exit', resolve));
  if (workerState) await rm(workerState, { recursive: true, force: true });
}
