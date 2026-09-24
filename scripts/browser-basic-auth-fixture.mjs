#!/usr/bin/env node
import { createServer } from 'node:http';

// Local-only, disposable credentials for testing the native HTTP auth prompt.
const username = 'monitter-test';
const password = 'browser-test';
const expected = `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`;

const server = createServer((request, response) => {
  if (request.headers.authorization !== expected) {
    response.writeHead(401, {
      'WWW-Authenticate': 'Basic realm="Monitter browser test"',
      'Cache-Control': 'no-store',
      'Content-Type': 'text/plain; charset=utf-8',
    });
    response.end('Authentication required');
    return;
  }
  response.writeHead(200, {
    'Cache-Control': 'no-store',
    'Content-Type': 'text/html; charset=utf-8',
  });
  response.end('<!doctype html><title>Basic Auth passed</title><h1>Basic Auth passed</h1>');
});

server.listen(0, '127.0.0.1', () => {
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Expected a TCP listener');
  process.stdout.write(`Open http://127.0.0.1:${address.port}/ in a native browser tab\n`);
  process.stdout.write(`Fixture credentials: ${username} / ${password}\n`);
});
