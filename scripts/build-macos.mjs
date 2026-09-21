import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'darwin') throw new Error('This packaging script is for macOS.');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const profile = process.argv.includes('--debug') ? 'debug' : 'release';
const buildArgs = ['run', 'tauri', '--', 'build', '--bundles', 'app', '--no-sign'];
if (profile === 'debug') buildArgs.push('--debug');
// Explicit recovery option after compilation succeeded but packaging was interrupted.
if (!process.argv.includes('--package-only')) execFileSync('npm', buildArgs, { cwd: root, stdio: 'inherit' });

// Documents may be managed by File Provider, which adds FinderInfo to .app bundles.
// Sign our compiled application outside that directory, preserving security attributes.
const cache = join(homedir(), 'Library/Caches/Monitter/builds');
mkdirSync(cache, { recursive: true });
const staging = mkdtempSync(join(cache, `${profile}-`));
const app = join(staging, 'Monitter.app');
const compiled = join(root, `src-tauri/target/${profile}/bundle/macos/Monitter.app`);
if (!existsSync(compiled)) throw new Error(`Tauri did not produce ${compiled}`);
execFileSync('/usr/bin/ditto', [compiled, app], { stdio: 'inherit' });

function removeSigningMetadata(path) {
  const attrs = execFileSync('/usr/bin/xattr', [path], { encoding: 'utf8' }).split('\n');
  for (const attr of ['com.apple.FinderInfo', 'com.apple.ResourceFork']) {
    if (attrs.includes(attr)) execFileSync('/usr/bin/xattr', ['-d', attr, path]);
  }
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) removeSigningMetadata(child);
    else if (!entry.isSymbolicLink()) {
      const childAttrs = execFileSync('/usr/bin/xattr', [child], { encoding: 'utf8' }).split('\n');
      for (const attr of ['com.apple.FinderInfo', 'com.apple.ResourceFork']) {
        if (childAttrs.includes(attr)) execFileSync('/usr/bin/xattr', ['-d', attr, child]);
      }
    }
  }
}
removeSigningMetadata(app);
// Ad-hoc signing (`-`) hashes the bundle, so every build gets a new identity
// and macOS Keychain re-prompts for Environment & Secrets access on install.
// Prefer the stable local certificate used for dev builds (see
// scripts/dev-codesign-runner.mjs); fall back to ad-hoc where it's absent.
const devIdentity = process.env.MONITTER_DEV_SIGNING_IDENTITY
  ?? 'Apple Development: soyrex@me.com (V53PM3GMXR)';
const availableIdentities = execFileSync('/usr/bin/security', ['find-identity', '-v', '-p', 'codesigning'], {
  encoding: 'utf8',
});
const signIdentity = availableIdentities.includes(devIdentity) ? devIdentity : '-';
execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', signIdentity, app], { stdio: 'inherit' });
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', app], { stdio: 'inherit' });

symlinkSync('/Applications', join(staging, 'Applications'));
const artifacts = join(root, 'artifacts');
mkdirSync(artifacts, { recursive: true });
// Never overwrite a previous disk image: Finder or File Provider may still hold it open.
const filename = `Monitter-${process.arch}-${profile}-${Date.now()}.dmg`;
const packages = join(cache, 'packages');
mkdirSync(packages, { recursive: true });
const cachedDmg = join(packages, filename);
execFileSync('/usr/bin/hdiutil', ['create', '-volname', 'Monitter', '-srcfolder', staging, '-format', 'UDZO', cachedDmg], { stdio: 'inherit' });
execFileSync('/usr/bin/hdiutil', ['verify', cachedDmg], { stdio: 'inherit' });
const dmg = join(artifacts, filename);
copyFileSync(cachedDmg, dmg);
const manifest = { profile, app, dmg, builtAt: new Date().toISOString() };
writeFileSync(join(artifacts, `macos-${profile}.json`), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(JSON.stringify(manifest, null, 2));
