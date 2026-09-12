import assert from 'node:assert/strict';
import { mock } from 'node:test';
import {registerPairingCode,redeemPairingCode,revokePairingCode} from '../src/lib/controller/pairing-code.ts';
const original=globalThis.fetch;
const invitation={version:2,relayUrl:'wss://api.monitter.com/relay',room:'r'.repeat(24),publicKey:'B'+'a'.repeat(86)};
let calls=0;
try {
 globalThis.fetch=async(url,options={})=>{
  calls++;
  assert.equal(new URL(url).origin,'https://api.monitter.com');
  if(options.method==='POST'){
   assert.deepEqual(JSON.parse(options.body),{invitation});
   return Response.json({code:'012345678',expiresAt:Date.now()+300000,revocationToken:'private-revocation-token'});
  }
  assert.equal(new URL(url).pathname,'/pairing/012345678');
  if(options.method==='DELETE'){assert.equal(options.headers.Authorization,'Bearer private-revocation-token');return new Response(null,{status:204});}
  assert.equal(options.cache,'no-store');
  return Response.json({invitation,expiresAt:Date.now()+300000});
 };
 const registration=await registerPairingCode(JSON.stringify(invitation));
 assert.equal(registration.code,'012345678');
 assert.deepEqual(JSON.parse(await redeemPairingCode('012 345 678')),invitation);
 await revokePairingCode(invitation.relayUrl,registration);
 assert.equal(calls,3);
 await assert.rejects(()=>redeemPairingCode('123'),/nine-digit/);
 await assert.rejects(()=>registerPairingCode(JSON.stringify({...invitation,secret:'never-send'})),/Update/);
 assert.equal(calls,3);
 globalThis.fetch=async()=>new Response(null,{status:404});
 await assert.rejects(()=>redeemPairingCode('012345678'),/used or expired/);
 globalThis.fetch=async()=>new Response(null,{status:429});
 await assert.rejects(()=>redeemPairingCode('012345678'),/Too many/);
 globalThis.fetch=async()=>Response.json({invitation:{...invitation,secret:'bad'},expiresAt:Date.now()+300000});
 await assert.rejects(()=>redeemPairingCode('012345678'),/Invalid/);
 for (const name of ['AbortError', 'TimeoutError']) {
  globalThis.fetch=async()=>{throw new DOMException('Fetch is aborted',name);};
  await assert.rejects(()=>registerPairingCode(JSON.stringify(invitation)),/pairing service at api.monitter.com timed out/);
  await assert.rejects(()=>redeemPairingCode('012345678'),/pairing service at api.monitter.com timed out/);
  await assert.rejects(()=>revokePairingCode(invitation.relayUrl,registration),/expire automatically/);
 }
 globalThis.fetch=async()=>{throw new TypeError('Failed to fetch');};
 await assert.rejects(()=>redeemPairingCode('012345678'),/Could not reach the pairing service/);
 globalThis.fetch=async()=>new Response('not JSON');
 await assert.rejects(()=>redeemPairingCode('012345678'),/Invalid pairing response/);
 // Body reads share the deadline, and an uncertain one-use claim is never retried.
 mock.timers.enable({apis:['setTimeout']});
 let requests=0;
 globalThis.fetch=async(_url,options)=>{
  requests++;
  return {ok:true,text:()=>new Promise((resolve,reject)=>options.signal.addEventListener('abort',()=>reject(new DOMException('Fetch is aborted','AbortError')),{once:true}))};
 };
 const stalled=assert.rejects(()=>redeemPairingCode('012345678'),/pairing service at api.monitter.com timed out/);
 await Promise.resolve();
 mock.timers.tick(15000);
 await stalled;
 assert.equal(requests,1);
 mock.timers.reset();
 console.log('Nine-digit pairing client validation passed.');
} finally {mock.timers.reset();globalThis.fetch=original;}
