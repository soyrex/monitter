import { execFileSync, spawn } from 'node:child_process';
import { closeSync, mkdirSync, openSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const flags = process.argv.slice(2);
execFileSync(process.execPath, [join(root, 'scripts/build-gpui.mjs'), ...flags], { cwd: root, stdio: 'inherit' });
const profile = flags.includes('--release') ? 'release' : 'debug';
const latest = JSON.parse(readFileSync(join(root, 'artifacts/gpui', profile, 'latest.json'), 'utf8'));
const executable = process.platform === 'linux' ? latest.app : join(latest.app, 'Contents/MacOS/monitter-gpui-desktop');
const logDir = join(root, 'artifacts/gpui', profile);
mkdirSync(logDir, { recursive: true });
const output = openSync(join(logDir, 'preview.stdout.log'), 'a');
const errors = openSync(join(logDir, 'preview.stderr.log'), 'a');
const child = spawn(executable, [], {
  cwd: root, detached: true, stdio: ['ignore', output, errors],
  env: { ...process.env, ...(flags.includes('--visual-export') ? { MONITTER_GPUI_EXPORT_FRAME: '1' } : {}) },
});
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.unref();
closeSync(output);
closeSync(errors);
console.log(`GPUI client launched as PID ${child.pid}. It connects to the persistent backend.`);
