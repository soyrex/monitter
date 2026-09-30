import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
await mkdir(join(root, 'artifacts'), { recursive: true });
const directory = await mkdtemp(join(root, 'artifacts/companion-ui-run-'));
try {
  const output = join(directory, 'test.mjs');
  execFileSync(join(root, 'node_modules/.bin/rolldown'), [
    'scripts/owner-relay-ui-bridge.ts', '--platform', 'node', '--format', 'esm',
    '--file', output, '--logLevel', 'silent',
  ], { cwd: root, stdio: 'inherit' });
  execFileSync(process.execPath, [join(root, 'scripts/companion-ui-test.mjs')], {
    cwd: root, stdio: 'inherit', env: { ...process.env, MONITTER_OWNER_TEST_BRIDGE: output },
  });
} finally {
  await rm(directory, { recursive: true, force: true });
}
