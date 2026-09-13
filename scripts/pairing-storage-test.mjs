import { chromium, webkit } from '@playwright/test';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import assert from 'node:assert/strict';

const directory = await mkdtemp(join(tmpdir(), 'monitter-pairing-storage-'));
let server, context;
try {
  const entry = join(directory, 'entry.ts'), bundle = join(directory, 'store.js');
  await writeFile(entry, `export * from ${JSON.stringify(resolve('src/lib/controller/pairing-store.ts'))};\nexport * from ${JSON.stringify(resolve('src/lib/controller/secure-session.ts'))};\n`);
  execFileSync('node_modules/.bin/rolldown', [entry, '--platform', 'browser', '--format', 'esm', '--file', bundle, '--logLevel', 'silent']);
  const module = await readFile(bundle);
  server = createServer((request, response) => {
    response.setHeader('Content-Type', request.url === '/store.js' ? 'text/javascript' : 'text/html');
    response.end(request.url === '/store.js' ? module : '<!doctype html><title>Pairing storage test</title>');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  for (const [name, engine] of [['Chromium', chromium], ['WebKit', webkit]]) {
    const profile = join(directory, name);
    context = await engine.launchPersistentContext(profile, {headless:true});
    let page = await context.newPage();
    await page.goto(origin);
    const initial = await page.evaluate(async () => {
      const m = await import('/store.js');
      const desktop = await m.newDesktopPairing('wss://api.monitter.com/relay');
      const phone = await m.newDesktopPairing('wss://api.monitter.com/relay');
      const publicKey = m.bytesToBase64url(await m.exportPairingPublicKey(phone.identity.keyPair.publicKey));
      const desktopPublic = m.bytesToBase64url(await m.exportPairingPublicKey(desktop.identity.keyPair.publicKey));
      const now = 2_000_000_000_000, day = 86_400_000;
      desktop.devices = [{publicKey,name:'Pixel',approvedAt:now,lastAccessAt:now}];
      const indefinite = m.isRememberedController(desktop, publicKey, now + 36500 * day);
      desktop.inactivityDays = 14;
      const valid = m.isRememberedController(desktop, publicKey, now + 13 * day);
      const expired = m.isRememberedController(desktop, publicKey, now + 14 * day);
      desktop.devices[0].lastAccessAt = now + day;
      const renewed = m.isRememberedController(desktop, publicKey, now + 14.5 * day);
      await m.saveDesktopPairing(desktop);
      await m.saveMobilePairing({version:1,identity:{keyPair:phone.identity.keyPair},savedAt:now,invitation:JSON.stringify({version:2,relayUrl:desktop.relayUrl,room:desktop.identity.room,publicKey:desktopPublic})});
      return {publicKey,desktopPublic,indefinite,valid,expired,renewed};
    });
    assert.deepEqual([initial.indefinite, initial.valid, initial.expired, initial.renewed], [true,true,false,true]);
    // Close the browser entirely. This exercises disk persistence, not just an
    // in-memory handle surviving a DOM reload.
    await context.close(); context = null;
    context = await engine.launchPersistentContext(profile, {headless:true});
    page = await context.newPage(); await page.goto(origin);
    const restored = await page.evaluate(async () => {
      const m = await import('/store.js');
      const desktop = await m.loadDesktopPairing(), phone = await m.loadMobilePairing();
      const publicKey = m.bytesToBase64url(await m.exportPairingPublicKey(phone.identity.keyPair.publicKey));
      const desktopPublic = m.bytesToBase64url(await m.exportPairingPublicKey(desktop.identity.keyPair.publicKey));
      const a = await m.deriveEcdhSecret(phone.identity.keyPair.privateKey, await m.exportPairingPublicKey(desktop.identity.keyPair.publicKey));
      const b = await m.deriveEcdhSecret(desktop.identity.keyPair.privateKey, await m.exportPairingPublicKey(phone.identity.keyPair.publicKey));
      let exportBlocked = false;
      try { await crypto.subtle.exportKey('jwk', phone.identity.keyPair.privateKey); } catch { exportBlocked = true; }
      desktop.devices = [];
      await m.saveDesktopPairing(desktop);
      const revoked = !m.isRememberedController(await m.loadDesktopPairing(), publicKey);
      await Promise.all([m.saveMobilePairing(phone), m.clearMobilePairing()]);
      const forgotten = await m.loadMobilePairing() === null;
      await m.clearDesktopPairing();
      return {publicKey,desktopPublic,match:m.bytesToBase64url(a)===m.bytesToBase64url(b),exportBlocked,revoked,forgotten};
    });
    assert.equal(restored.publicKey, initial.publicKey);
    assert.equal(restored.desktopPublic, initial.desktopPublic);
    assert.deepEqual([restored.match,restored.exportBlocked,restored.revoked,restored.forgotten],[true,true,true,true]);
    await context.close(); context = null;
    console.log(`${name}: durable nonextractable keys, cold restart, indefinite/sliding expiry, revocation and save/forget ordering passed.`);
  }
} finally {
  await context?.close();
  await new Promise(resolve => server ? server.close(resolve) : resolve());
  await rm(directory, {recursive:true,force:true});
}
