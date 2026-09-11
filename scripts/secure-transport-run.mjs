import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const directory = await mkdtemp(join(tmpdir(), 'monitter-secure-'));
try {
  const output = join(directory, 'test.mjs');
  execFileSync(join(root, 'node_modules/.bin/rolldown'), [
    'scripts/secure-transport-test.mjs', '--platform', 'node', '--format', 'esm',
    '--file', output, '--logLevel', 'silent',
  ], { cwd: root, stdio: 'inherit' });
  execFileSync(process.execPath, [output], { cwd: root, stdio: 'inherit' });
} finally {
  await rm(directory, { recursive: true, force: true });
}
