#!/usr/bin/env node
import { createServer } from 'node:http';

// Local-only pages for native history, shared cookies, and password-manager
// autofill checks. The form never submits, so test vault values stay local.
const server = createServer((request, response) => {
  const path = new URL(request.url ?? '/', 'http://127.0.0.1').pathname;
  if (!['/', '/a', '/b', '/login', '/spa'].includes(path)) {
    response.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
    response.end('Not found');
    return;
  }
  const title = path === '/b' ? 'Browser test B' : path === '/login' ? 'Browser test login' : path === '/spa' ? 'Browser test SPA' : 'Browser test A';
  const form = path === '/login'
    ? '<form onsubmit="event.preventDefault()"><label>Username <input name="username" autocomplete="username"></label><label>Password <input name="password" type="password" autocomplete="current-password"></label><button>Sign in</button></form>'
    : path === '/spa'
      ? '<button onclick="history.pushState({}, \'\', \'/spa?step=2\'); document.querySelector(\'#step\').textContent = \'SPA step 2\'">Push SPA route</button><p id="step">SPA step 1</p><script>addEventListener(\'popstate\', () => { document.querySelector(\'#step\').textContent = location.search ? \'SPA step 2\' : \'SPA step 1\'; });</script>'
    : '<p><a href="/a">Page A</a> · <a href="/b">Page B</a> · <a href="/login">Login form</a></p>';
  response.writeHead(200, {
    'Content-Type': 'text/html; charset=utf-8',
    'Cache-Control': 'no-store',
    'Set-Cookie': 'monitter_browser_fixture=shared; HttpOnly; SameSite=Lax; Path=/',
  });
  response.end(`<!doctype html><html><head><title>${title}</title></head><body><h1>${title}</h1>${form}<p>Cookie received: ${request.headers.cookie?.includes('monitter_browser_fixture=shared') ? 'yes' : 'no'}</p></body></html>`);
});

server.listen(0, '127.0.0.1', () => {
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Expected a TCP listener');
  process.stdout.write(`Browser smoke pages: http://127.0.0.1:${address.port}/a and /b; SPA: /spa; autofill form: /login\n`);
});
