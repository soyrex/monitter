import assert from 'node:assert/strict';
import { trustedDevAddress, webDevGuard, webDevPlugin, webDevProxy } from './vite-web-dev.mjs';

const origin = 'http://127.0.0.1:18450';
const guard = webDevGuard(origin);
function request({ url = '/api/invoke', method = 'POST', host = '127.0.0.1:18450', from = origin, site, peer = '127.0.0.1' } = {}) {
  let status, next = false;
  const headers = { host };
  if (from !== undefined) headers.origin = from;
  if (site) headers['sec-fetch-site'] = site;
  guard({ url, method, headers, socket: { remoteAddress: peer } }, { writeHead: code => { status = code; }, end() {} }, () => { next = true; });
  return next ? 200 : status;
}
assert.equal(request(), 200);
assert.equal(request({ from: 'https://attacker.invalid' }), 403);
assert.equal(request({ from: 'null' }), 403);
assert.equal(request({ from: null }), 403);
assert.equal(request({ host: 'attacker.invalid:18450' }), 421);
assert.equal(request({ site: 'cross-site' }), 403);
assert.equal(request({ url: '/api/other' }), 404);
assert.equal(request({ url: '/api/access', method: 'GET', from: null }), 200);
assert.equal(request({ url: '/@vite/client', method: 'GET', from: null }), 200);
const proxy = Object.values(webDevProxy())[0];
assert.equal(proxy.target, 'http://127.0.0.1:18436');
assert.equal(proxy.headers.origin, proxy.target);
assert.equal(proxy.changeOrigin, true);
assert.equal(request({ peer: '8.8.8.8' }), 403);
for (const ip of ['192.168.10.110', '100.121.138.9']) {
  const base = `http://${ip}:18450`;
  let reached = false;
  webDevGuard([origin, base])({ url: '/api/invoke', method: 'POST', headers: { host: `${ip}:18450`, origin: base }, socket: { remoteAddress: ip } },
    { writeHead: code => assert.fail(`Allowed private origin rejected: ${code}`), end() {} }, () => { reached = true; });
  assert.equal(reached, true);
}
for (const ip of ['127.0.0.1', '::1', '::ffff:192.168.10.22', '192.168.10.22', '10.0.0.5', '172.16.0.2', '100.121.138.9']) assert.equal(trustedDevAddress(ip), true);
for (const ip of ['8.8.8.8', '172.32.0.1', '100.128.0.1', '100.63.0.1', '192.0.2.1', '', '2001:db8::1']) assert.equal(trustedDevAddress(ip), false);
assert.throws(() => webDevPlugin().configureServer({ config: { server: { host: 'public.example' } } }), /guarded/);
console.log('Dev-web guard: same-origin proxy allowed; foreign origins, rebinding, originless writes and unexpected API paths denied.');
