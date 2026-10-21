import assert from 'node:assert/strict';
import test from 'node:test';
import { safeImageSource } from './media.mjs';

test('rejects files, network requests and inline data', () => {
  for (const source of [
    '/Users/alex/private.png', 'file:///tmp/photo.png', 'asset:/photo.png',
    'https://example.com/photo.png', 'data:image/svg+xml;base64,AA==',
    'data:image/png;base64,AA==',
  ]) assert.equal(safeImageSource(source), null);
});

test('cached media requires a trusted host mapping', () => {
  const id = `media:${'a'.repeat(64)}.jpg`;
  assert.equal(safeImageSource(id), null);
  assert.equal(safeImageSource(id, { [id]: '/approved/photo.jpg' }), '/approved/photo.jpg');
  assert.equal(safeImageSource('media:../../private.jpg', { 'media:../../private.jpg': '/private.jpg' }), null);
});
