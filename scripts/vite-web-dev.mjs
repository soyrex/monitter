import { networkInterfaces } from 'node:os';
import { isIP } from 'node:net';
import { realpathSync } from 'node:fs';
import { join } from 'node:path';

const backend = 'http://127.0.0.1:18436';
const apiRoute = /^\/api\/(?:access|invoke)(?:\?|$)/;

export function trustedDevAddress(address = '') {
  const ip = address.replace(/^::ffff:/, '');
  if (ip === '::1') return true;
  if (isIP(ip) !== 4) return false;
  const [a, b] = ip.split('.').map(Number);
  return a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127);
}

/** @param {string | string[]} origins */
export function webDevGuard(origins) {
  const allowed = new Set(typeof origins === 'string' ? [origins] : origins);
  const hosts = new Set([...allowed].map(origin => new URL(origin).host));
  return (/** @type {import('node:http').IncomingMessage} */ req, /** @type {import('node:http').ServerResponse} */ res, /** @type {() => void} */ next) => {
    const reject = (/** @type {number} */ code, /** @type {string} */ error) => {
      res.writeHead(code, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
      res.end(JSON.stringify({ ok: false, error }));
    };
    // Bind broadly for LAN/Tailscale, but reject public peers, DNS rebinding and
    // cross-site requests before rewriting anything for the backend connection.
    if (!trustedDevAddress(req.socket?.remoteAddress)) return reject(403, 'LAN and Tailscale clients only.');
    const host = req.headers.host;
    if (!host || !hosts.has(host)) return reject(421, 'Use a listed dev-server IP address.');
    const origin = `http://${host}`;
    if (req.headers.origin && req.headers.origin !== origin) return reject(403, 'Invalid dev-server origin.');
    if (req.url?.startsWith('/api/')) {
      if (req.headers['sec-fetch-site'] === 'cross-site') return reject(403, 'Cross-site API requests are not allowed.');
      if (!apiRoute.test(req.url)) return reject(404, 'Unknown Monitter API route.');
      if (req.method !== 'GET' && req.headers.origin !== origin) return reject(403, 'API writes require the dev-server origin.');
    }
    next();
  };
}

export function webDevPlugin() {
  return {
    name: 'monitter-private-network-web-dev',
    apply: 'serve',
    configureServer(/** @type {import('vite').ViteDevServer} */ server) {
      const configuredHost = server.config.server.host;
      if (typeof configuredHost !== 'string' || !['127.0.0.1', '0.0.0.0'].includes(configuredHost) || server.config.server.https) {
        throw new Error('Monitter web development requires its guarded private-network HTTP binding.');
      }
      const ips = new Set(['127.0.0.1', ...Object.values(networkInterfaces()).flatMap(items =>
        (items ?? []).filter(item => item.family === 'IPv4' && trustedDevAddress(item.address)).map(item => item.address)
      )]);
      const origins = [...ips].map(ip => `http://${ip}:${server.config.server.port}`);
      // Worktrees may share node_modules; font URLs resolve to its real path.
      server.config.server.fs.allow.push(realpathSync(join(server.config.root, 'node_modules/@fontsource')));
      server.middlewares.use(webDevGuard(origins));
    },
  };
}

export function webDevProxy({ developerBridge = false } = {}) {
  return {
    // The guard checks the original browser Origin before proxying. Rewrite it
    // only for this trusted hop; desktop auth and permission handling stay intact.
    '^/api/(access|invoke)(\\?|$)': {
      target: backend,
      changeOrigin: true,
      headers: { origin: backend, ...(developerBridge ? { 'x-monitter-dev-bridge': '1' } : {}) },
    },
  };
}
