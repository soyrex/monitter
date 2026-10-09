import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertExperimentalConfig, assertExperimentalBundle, EXPERIMENTAL_ID } from './experimental-identity.mjs';

if (process.platform !== 'darwin') throw new Error('This installer is for macOS.');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
assertExperimentalConfig(root);
const profile = process.argv.includes('--debug') ? 'debug' : 'release';
const deferRestart = process.argv.includes('--defer-restart');
const manifestPath = join(root, `artifacts/macos-${profile}.json`);
if (!existsSync(manifestPath)) throw new Error(`Build the ${profile} app first with npm run ${profile === 'debug' ? 'build:mac:local' : 'build:mac'}.`);
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
if (manifest.identifier !== EXPERIMENTAL_ID) throw new Error('Refusing an unverified or non-experimental build manifest.');
const source = manifest.app;
const target = '/Applications/Monitter GPUI.app';
if (!existsSync(source)) throw new Error(`Build the ${profile} app first with npm run ${profile === 'debug' ? 'build:mac:local' : 'build:mac'}.`);
assertExperimentalBundle(source);
if (existsSync(target)) assertExperimentalBundle(target);
const processes = execFileSync('/bin/ps', ['-axo', 'comm='], { encoding: 'utf8' });
const running = processes.split('\n').some(line => line.startsWith(target + '/Contents/MacOS/'));
if (running && !deferRestart) {
  throw new Error('Quit Monitter GPUI before installing this build. Your saved agents and conversations will remain.');
}
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', source], { stdio: 'inherit' });
const staging = mkdtempSync('/Applications/.monitter-gpui-install-');
const staged = join(staging, 'Monitter GPUI.app');
execFileSync('/usr/bin/ditto', [source, staged], { stdio: 'inherit' });
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', staged], { stdio: 'inherit' });
if (existsSync(target)) {
  const backups = join(root, 'artifacts/install-backups');
  mkdirSync(backups, { recursive: true });
  const backup = join(backups, `Monitter-GPUI-${Date.now()}.app`);
  renameSync(target, backup);
  console.log(`Previous app preserved at ${backup}`);
}
renameSync(staged, target);
rmdirSync(staging);
console.log(`Installed ${target}`);
if (deferRestart) {
  console.log(running
    ? 'Deferred restart: the existing Monitter GPUI process is still running and will use this build after you restart it.'
    : 'Deferred launch: Monitter GPUI was not opened.');
} else {
  execFileSync('/usr/bin/open', ['-a', target]);
}
