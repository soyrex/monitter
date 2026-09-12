import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { publishLanWeb } from './publish-lan-web.mjs';
const root = mkdtempSync(join(tmpdir(), 'monitter-publish-test-'));
try {
  const source = join(root, 'build'), live = join(root, 'live');
  mkdirSync(join(source, '_app/immutable'), { recursive: true });
  writeFileSync(join(source, 'index.html'), 'first');
  writeFileSync(join(source, '_app/immutable/old.js'), 'old');
  publishLanWeb(source, live);
  assert.equal(readFileSync(join(live, 'index.html'), 'utf8'), 'first');
  rmSync(join(source, '_app/immutable/old.js'));
  writeFileSync(join(source, '_app/immutable/new.js'), 'new');
  writeFileSync(join(source, 'index.html'), 'second');
  publishLanWeb(source, live);
  assert.equal(readFileSync(join(live, 'index.html'), 'utf8'), 'second');
  assert.ok(existsSync(join(live, '_app/immutable/old.js')));
  assert.ok(existsSync(join(live, '_app/immutable/new.js')));
  rmSync(join(source, 'index.html'));
  assert.throws(() => publishLanWeb(source, live));
  assert.equal(readFileSync(join(live, 'index.html'), 'utf8'), 'second');
  console.log('Live web publishing, retained chunks and incomplete-build protection passed.');
} finally { rmSync(root, { recursive: true, force: true }); }
