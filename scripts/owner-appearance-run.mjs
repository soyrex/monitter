import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const directory = await mkdtemp(join(tmpdir(), 'monitter-owner-appearance-'));
try {
  const output = join(directory, 'test.mjs');
  execFileSync(join(root, 'node_modules/.bin/rolldown'), [
    'scripts/owner-appearance-test.mjs', '--platform', 'node', '--format', 'esm',
    '--file', output, '--no-codeSplitting', '--logLevel', 'silent',
  ], { cwd: root, stdio: 'inherit' });
  execFileSync(process.execPath, [output], { cwd: root, stdio: 'inherit' });
} finally {
  await rm(directory, { recursive: true, force: true });
}
