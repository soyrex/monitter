import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { hotUiMarkerPlugin } from '../vite.config.js';

let middleware;
hotUiMarkerPlugin('/monitter-app-ui/__monitter_dev__').configureServer({
  middlewares: {
    use(handler) {
      middleware = handler;
    },
  },
});

assert.equal(typeof middleware, 'function');

function request(url) {
  let nextCalled = false;
  const headers = new Map();
  const response = {
    statusCode: 0,
    setHeader(name, value) {
      headers.set(name.toLowerCase(), value);
    },
    end(value) {
      this.body = value;
    },
  };
  middleware({ url }, response, () => { nextCalled = true; });
  return { response, headers, nextCalled };
}

const marker = request('/monitter-app-ui/__monitter_dev__?cache-bust=1');
assert.equal(marker.nextCalled, false);
assert.equal(marker.response.statusCode, 200);
assert.equal(marker.headers.get('cache-control'), 'no-store');
assert.deepEqual(JSON.parse(marker.response.body), { app: 'monitter', hotUiProtocol: 1 });

const ordinaryPage = request('/');
assert.equal(ordinaryPage.nextCalled, true);

const localCapability = JSON.parse(readFileSync(new URL('../src-tauri/capabilities/default.json', import.meta.url)));
const hotUiCapability = JSON.parse(readFileSync(new URL('../src-tauri/capabilities/hot-ui.json', import.meta.url)));
assert.equal(localCapability.remote, undefined, 'bundled UI capability must not trust remote origins');
assert.equal(hotUiCapability.local, false);
assert.deepEqual(hotUiCapability.remote.urls, ['http://127.0.0.1:18420/monitter-app-ui/*']);
assert.deepEqual(hotUiCapability.permissions, ['allow-use-packaged-ui']);
assert.match(
  readFileSync(new URL('../src-tauri/permissions/default.toml', import.meta.url), 'utf8'),
  /identifier = "allow-use-packaged-ui"[\s\S]*?commands\.allow = \["use_packaged_ui"\]/,
);

console.log('Hot UI marker and remote-IPC boundary contracts passed.');
