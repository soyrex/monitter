import assert from 'node:assert/strict';
import { hotUiMarkerPlugin } from '../vite.config.js';

let middleware;
hotUiMarkerPlugin().configureServer({
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

const marker = request('/__monitter_dev__?cache-bust=1');
assert.equal(marker.nextCalled, false);
assert.equal(marker.response.statusCode, 200);
assert.equal(marker.headers.get('cache-control'), 'no-store');
assert.deepEqual(JSON.parse(marker.response.body), { app: 'monitter', hotUiProtocol: 1 });

const ordinaryPage = request('/');
assert.equal(ordinaryPage.nextCalled, true);

console.log('Hot UI marker contract passed.');
