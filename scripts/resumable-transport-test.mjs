import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import WebSocket from 'ws';
import { createResumableDesktopSession, createResumableMobileSession } from '../src/lib/controller/resumable-session.ts';
import { generatePairingKeyPair, exportPairingPublicKey, bytesToBase64url, randomBytes } from '../src/lib/controller/secure-session.ts';

globalThis.WebSocket = WebSocket;
const wait = (predicate, ms = 5000, label = 'state') => new Promise((resolve, reject) => {
  let settled = false, stop = () => {};
  const timer = setTimeout(() => { if (!settled) reject(new Error(`timeout ${label}`)); }, ms);
  const done = value => { if (settled) return; settled = true; clearTimeout(timer); queueMicrotask(() => stop()); resolve(value); };
  stop = predicate(done);
  if (settled) stop();
});
const ready = relay => new Promise((resolve, reject) => { relay.stdout.setEncoding('utf8'); relay.stdout.on('data', value => { const match = value.match(/(ws:\/\/127\.0\.0\.1:\d+)/); if (match) resolve(match[1]); }); setTimeout(() => reject(new Error('relay timeout')), 5000); });
const snapshot = { hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], settings: { accent:'#000',theme:'dark',interfaceScale:125,showToolActivity:true,showReasoningSummaries:true,sendWithEnter:false,sidebarView:'standard' } };
const task = '11111111-1111-4111-8111-111111111111';
const relay = spawn(process.execPath, ['scripts/relay-server.mjs'], { cwd: process.cwd(), env: { ...process.env, MONITTER_RELAY_PORT:'0' }, stdio:['ignore','pipe','pipe'] });
try {
  const relayUrl = await ready(relay), room = bytesToBase64url(randomBytes(18));
  const desktopIdentity = { keyPair: await generatePairingKeyPair(), room }, mobileIdentity = { keyPair: await generatePairingKeyPair() };
  const mobileKey = bytesToBase64url(await exportPairingPublicKey(mobileIdentity.keyPair.publicKey));
  const trusted = new Set([mobileKey]); let approvals = 0, sends = 0, accesses = 0, releaseSlowSend = () => {};
  let delayAccess = false, accessStarted = false, releaseDelayedAccess = () => {};
  let delayedAccess = Promise.resolve();
  const onControllerAccess = async () => { accesses++; if (delayAccess) { accessStarted = true; await delayedAccess; } };
  const slowSend = new Promise(resolve => { releaseSlowSend = resolve; });
  const bridge = { getSnapshot: async()=>snapshot, sendMessage: async(_taskId,text)=>{ sends++; if(text==='slow') await slowSend; return { accepted:true }; }, cancelTask:async()=>snapshot,resumeTask:async()=>snapshot,listTerminals:async()=>[],readTerminal:async()=>({chunks:[],nextSeq:0,status:'exited',exitCode:0,truncated:false}) };
  let desktop = await createResumableDesktopSession({ relayUrl, bridge, identity: desktopIdentity, isTrustedController:key=>trusted.has(key), onControllerAccess });
  const invitation = desktop.invitation;
  let mobile = await createResumableMobileSession({ invitation, identity: mobileIdentity });
  await wait(done=>desktop.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'desktop initial');
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile initial');
  assert.equal(mobile.supportsRememberedDevices(), true);
  assert.equal(approvals, 0); assert.equal(desktop.getPeerControllerPublicKey(), mobileKey);
  await mobile.sendMessage(task, 'one'); assert.equal(sends, 1); assert.ok(accesses > 0);
  // A phone process restart uses the same identity; the desktop trusts it without a new approval.
  mobile.close(); mobile = await createResumableMobileSession({ invitation, identity: mobileIdentity });
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile restart');
  await mobile.sendMessage(task, 'two'); assert.equal(sends, 2);
  // Recreating the desktop facade with its persisted identity preserves the
  // invitation and lets the same phone reconnect without another approval.
  desktop.close();
  desktop = await createResumableDesktopSession({ relayUrl, bridge, identity: desktopIdentity, isTrustedController:key=>trusted.has(key), onControllerAccess });
  assert.equal(desktop.invitation, invitation);
  await wait(done=>desktop.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'desktop recreation');
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile after desktop recreation');
  await mobile.sendMessage(task, 'three'); assert.equal(sends, 3);

  // Revocation during the async authenticated-access hook must invalidate the
  // request before the bridge handler runs.
  delayedAccess = new Promise(resolve => { releaseDelayedAccess = resolve; });
  delayAccess = true; accessStarted = false;
  const revokedDuringAccess = mobile.sendMessage(task, 'revoked-in-flight');
  await wait(done => { const timer = setInterval(() => { if (accessStarted) { clearInterval(timer); done(); } }, 5); return () => clearInterval(timer); }, 5000, 'delayed access hook');
  await desktop.reject();
  releaseDelayedAccess();
  await assert.rejects(revokedDuringAccess, /Connection closed|Desktop approval|live encrypted connection/);
  assert.equal(sends, 3, 'revoked in-flight request reached the bridge');
  delayAccess = false;
  await desktop.reconnectNow(); await mobile.reconnectNow();
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'live before policy revocation');

  // Revocation is checked again on every RPC before dispatch. The rejected
  // call cannot renew itself or mutate the desktop.
  const accessesBeforeRevocation = accesses;
  trusted.delete(mobileKey);
  await assert.rejects(mobile.sendMessage(task, 'revoked'), /Connection closed|Desktop approval|live encrypted connection/);
  assert.equal(sends, 3); assert.equal(accesses, accessesBeforeRevocation);
  trusted.add(mobileKey);
  await desktop.reconnectNow(); await mobile.reconnectNow();
  await wait(done=>desktop.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'desktop after trust restore');
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile after trust restore');
  await mobile.sendMessage(task, 'four'); assert.equal(sends, 4);
  // Native resume, visibilitychange and pageshow can request a foreground
  // reconnect together. Concurrent calls coalesce and leave one live session.
  await Promise.all([desktop.reconnectNow(), desktop.reconnectNow(), mobile.reconnectNow(), mobile.reconnectNow()]);
  await wait(done=>desktop.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'desktop coalesced reconnect');
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile coalesced reconnect');
  // In-flight mutations are rejected at close and are never replayed by the new facade.
  const pending = mobile.sendMessage(task, 'slow');
  while (sends < 5) await new Promise(resolve=>setTimeout(resolve,10));
  mobile.close(); await assert.rejects(pending, /Connection closed|Desktop approval/);
  releaseSlowSend();
  mobile = await createResumableMobileSession({ invitation, identity: mobileIdentity });
  await wait(done=>mobile.subscribe(s=>{if(s.status==='connected')done(s)}),5000,'mobile after inflight close');
  await new Promise(resolve=>setTimeout(resolve,350)); assert.equal(sends, 5);
  mobile.close();
  await wait(done=>desktop.subscribe(s=>{if(s.status==='waiting_for_peer')done(s)}),5000,'desktop ready for unknown');
  const unknownIdentity = { keyPair: await generatePairingKeyPair() };
  const unknown = await createResumableMobileSession({ invitation, identity: unknownIdentity });
  await wait(done=>desktop.subscribe(s=>{if(s.status==='pending')done(s)}),5000,'unknown pending');
  assert.equal(desktop.getStatus(), 'pending'); unknown.close();
  await wait(done=>desktop.subscribe(s=>{if(s.status==='waiting_for_peer')done(s)}),5000,'desktop ready after unknown close');
  // Explicit rejection is terminal and does not enter an automatic reconnect loop.
  const rejected = await createResumableMobileSession({ invitation, identity: unknownIdentity });
  await wait(done=>desktop.subscribe(s=>{if(s.status==='pending')done(s)}),5000,'rejected pending'); await desktop.reject();
  await wait(done=>rejected.subscribe(s=>{if(s.status==='rejected')done(s)}),5000,'mobile rejected');
  await new Promise(resolve=>setTimeout(resolve, 350)); assert.equal(rejected.getStatus(), 'rejected');
  rejected.close(); desktop.close();
  console.log('Resumable relay transport assertions passed.');
} finally { relay.kill(); }
