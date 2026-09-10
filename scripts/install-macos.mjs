import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'darwin') throw new Error('This installer is for macOS.');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const profile = process.argv.includes('--debug') ? 'debug' : 'release';
const manifestPath = join(root, `artifacts/macos-${profile}.json`);
if (!existsSync(manifestPath)) throw new Error(`Build the ${profile} app first with npm run ${profile === 'debug' ? 'build:mac:local' : 'build:mac'}.`);
const source = JSON.parse(readFileSync(manifestPath, 'utf8')).app;
const target = '/Applications/Monitter.app';
if (!existsSync(source)) throw new Error(`Build the ${profile} app first with npm run ${profile === 'debug' ? 'build:mac:local' : 'build:mac'}.`);
const processes = execFileSync('/bin/ps', ['-axo', 'comm='], { encoding: 'utf8' });
if (processes.split('\n').some(line => line.startsWith(target + '/Contents/MacOS/'))) {
  throw new Error('Quit Monitter before installing this build. Your saved agents and conversations will remain.');
}
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', source], { stdio: 'inherit' });
const staging = mkdtempSync('/Applications/.monitter-install-');
const staged = join(staging, 'Monitter.app');
execFileSync('/usr/bin/ditto', [source, staged], { stdio: 'inherit' });
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', staged], { stdio: 'inherit' });
if (existsSync(target)) {
  const backups = join(root, 'artifacts/install-backups');
  mkdirSync(backups, { recursive: true });
  const backup = join(backups, `Monitter-${Date.now()}.app`);
  renameSync(target, backup);
  console.log(`Previous app preserved at ${backup}`);
}
renameSync(staged, target);
rmdirSync(staging);
console.log(`Installed ${target}`);
execFileSync('/usr/bin/open', ['-a', target]);
