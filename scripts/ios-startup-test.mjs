// Runs the already-built production SPA. Does not compile or contact a real desktop.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { webkit, expect } from '@playwright/test';
import { WebSocketServer } from 'ws';

// Held route modules may also hold font imports. Capture the actual pre-JS
// frame without Playwright waiting for those deliberately blocked fonts.
process.env.PW_TEST_SCREENSHOT_NO_FONTS_READY = '1';

const root = resolve(process.env.MONITTER_STARTUP_BUILD_DIR || 'build');
const artifacts = resolve(process.env.MONITTER_STARTUP_ARTIFACTS || 'artifacts/ios-startup');
await mkdir(artifacts, { recursive: true });
await readFile(resolve(root, 'index.html')); // Fail early rather than silently testing a dev server.
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.webp': 'image/webp', '.png': 'image/png', '.woff2': 'font/woff2', '.json': 'application/json' };
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    let file = resolve(root, '.' + pathname);
    if (file !== root && !file.startsWith(root + sep)) { response.writeHead(403).end(); return; }
    let body;
    try { body = await readFile(file); } catch { file = resolve(root, 'index.html'); body = await readFile(file); }
    response.writeHead(200, { 'content-type': types[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' }).end(body);
  } catch { response.writeHead(500).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const base = `http://127.0.0.1:${server.address().port}`;
// An isolated relay with no desktop present exercises the genuine waiting_for_peer state.
const relay = new WebSocketServer({ host: '127.0.0.1', port: 0 });
await new Promise(done => relay.once('listening', done));
let joined = 0;
relay.on('connection', socket => socket.on('message', raw => {
  if (JSON.parse(String(raw)).type === 'join') { joined++; socket.send(JSON.stringify({ type: 'joined' })); }
}));
const relayUrl = `ws://127.0.0.1:${relay.address().port}`;
const browser = await webkit.launch();
async function nativePage(options = {}, remembered = false) {
  const page = await browser.newPage({ viewport: { width: 375, height: 812 }, isMobile: true, hasTouch: true, ...options });
  await page.addInitScript(({ remembered, relayUrl }) => {
    const encode = bytes => btoa(String.fromCharCode(...new Uint8Array(bytes)));
    let keys;
    window.__pairingLoads = 0;
    window.__startupReady = [];
    window.webkit ??= {};
    window.webkit.messageHandlers ??= {};
    window.webkit.messageHandlers.monitterStartup = { postMessage(value) { window.__startupReady.push({ value, text: document.body.innerText }); } };
    window.webkit.messageHandlers.monitterPairing = { async postMessage(message) {
      if (message.action === 'load') {
        window.__pairingLoads++;
        if (remembered === 'storage-error') throw new Error('Test secure storage unavailable');
        if (!remembered) return null;
        return { invitation: JSON.stringify({ version: 1, relayUrl, room: 'abcdefghijklmnopqrstuv', secret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' }), savedAt: Date.now() };
      }
      if (message.action === 'identity') {
        keys ??= crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']);
        const pair = await keys;
        return { privateKeyPkcs8: encode(await crypto.subtle.exportKey('pkcs8', pair.privateKey)), publicKeyRaw: encode(await crypto.subtle.exportKey('raw', pair.publicKey)) };
      }
      return null;
    } };
    window.__startupFrames = [];
    const sample = () => {
      // Observe real rendered frames, including brief unwanted transition screens.
      const text = document.body?.innerText || '';
      if (text.includes('Your desktop.') || text.includes('Connecting to your desktop') || text.includes('Reconnecting to your desktop')) window.__startupFrames.push(text);
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  }, { remembered, relayUrl });
  return page;
}
try {
  for (const [name, options] of [
    ['portrait-dark', { colorScheme: 'dark' }],
    ['portrait-light', { colorScheme: 'light' }],
    ['landscape-reduced', { viewport: { width: 812, height: 375 }, colorScheme: 'light', reducedMotion: 'reduce' }],
  ]) {
    const page = await nativePage(options);
    let release;
    const held = new Promise(done => { release = done; });
    await page.route('**/*.js', async route => { await held; await route.continue(); });
    await page.goto(`${base}/mobile?owner=1`, { waitUntil: 'commit' });
    const splash = page.locator('#monitter-startup');
    await expect(splash).toBeVisible();
    assert.deepEqual(await page.evaluate(() => window.__startupReady), []);
    await expect(splash.getByText('Preparing workspace…', { exact: true })).toBeVisible();
    await expect.poll(() => splash.locator('img').evaluate(image => image.complete && image.naturalWidth > 0)).toBe(true);
    const layout = await splash.evaluate(element => {
      const image = element.querySelector('img').getBoundingClientRect();
      const label = element.querySelector('p');
      const lockup = element.firstElementChild.getBoundingClientRect();
      return { background: getComputedStyle(element).backgroundColor, color: getComputedStyle(label).color, width: image.width, x: lockup.x + lockup.width / 2, y: lockup.y + lockup.height / 2, viewport: [innerWidth, innerHeight], scroll: document.documentElement.scrollWidth };
    });
    assert.equal(layout.background, 'rgb(23, 25, 27)');
    assert.equal(layout.color, 'rgb(102, 169, 239)');
    assert.ok(Math.abs(layout.width - Math.min(280, layout.viewport[0] * .72)) < 1);
    assert.ok(Math.abs(layout.x - layout.viewport[0] / 2) < 1);
    assert.ok(Math.abs(layout.y - layout.viewport[1] / 2) < 1);
    assert.ok(layout.scroll <= layout.viewport[0]);
    await page.screenshot({ path: `${artifacts}/${name}-bootstrap.png` });
    release();
    await expect(page.getByRole('button', { name: 'Scan desktop QR code', exact: true })).toBeVisible({ timeout: 30000 });
    await expect(splash).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__startupReady.map(event => event.value))).toEqual(['ready']);
    await page.getByText('Use a pairing PIN instead', { exact: true }).click();
    await expect(page.getByLabel('Pairing PIN', { exact: true })).toBeVisible();
    await page.screenshot({ path: `${artifacts}/${name}-fresh-pairing.png` });
    await page.close();
  }
  const web = await browser.newPage();
  await web.route('**/*.js', route => route.abort());
  await web.goto(`${base}/mobile?owner=1`, { waitUntil: 'domcontentloaded' });
  await expect(web.locator('#monitter-startup')).toHaveCount(0);
  await web.close();

  for (const path of ['/mobile?owner=1', '/?monitter-owner-relay=1']) {
    const page = await nativePage({}, true);
    const before = joined;
    await page.goto(base + path, { waitUntil: 'domcontentloaded' });
    await expect.poll(() => joined, { timeout: 30000 }).toBeGreaterThan(before);
    const loading = page.locator('.workspace-loading-screen');
    await expect(loading).toBeVisible();
    await expect(page.locator('#monitter-startup')).toHaveCount(0);
    await page.waitForTimeout(600); // Sample rendered frames while the desktop is unavailable.
    assert.deepEqual(await page.evaluate(() => window.__startupFrames), []);
    assert.deepEqual(await page.evaluate(() => window.__startupReady), [], 'native overlay released while desktop was still unavailable');
    await expect(page.getByText('Preparing workspace…', { exact: true })).toBeVisible();
    await page.screenshot({ path: `${artifacts}/${path.startsWith('/mobile') ? 'remembered-mobile' : 'owner-gate'}-waiting.png` });
    await page.close();
  }
  const storageError = await nativePage({}, 'storage-error');
  await storageError.goto(`${base}/?monitter-owner-relay=1`, { waitUntil: 'domcontentloaded' });
  await expect(storageError.getByRole('alert')).toContainText('Test secure storage unavailable');
  await expect(storageError.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
  await expect.poll(() => storageError.evaluate(() => window.__startupReady.map(event => event.value))).toEqual(['ready']);
  await storageError.close();

  const broken = await nativePage();
  await broken.route('**/*.js', route => route.abort());
  await broken.goto(`${base}/mobile?owner=1`, { waitUntil: 'domcontentloaded' });
  await expect(broken.locator('#monitter-startup').getByRole('button', { name: 'Try again', exact: true })).toBeVisible({ timeout: 25000 });
  await expect.poll(() => broken.evaluate(() => window.__startupReady.map(event => event.value))).toEqual(['ready']);
  await broken.screenshot({ path: `${artifacts}/failed-module-recovery.png` });
  await broken.close();
  console.log(`iOS startup: production WebKit checks passed; screenshots: ${artifacts}`);
} finally {
  await browser.close();
  for (const client of relay.clients) client.terminate();
  await new Promise(done => relay.close(done));
  await new Promise(done => server.close(done));
}
