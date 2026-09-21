#!/usr/bin/env node
// Cargo runner for macOS dev builds (wired via src-tauri/.cargo/config.toml).
//
// The linker's ad-hoc signature hashes the binary itself, so every rebuild
// gets a new identity and macOS Keychain treats each run as a different app,
// re-prompting for Environment & Secrets access. Re-signing with a stable
// local certificate keeps the Keychain ACL valid across rebuilds. Falls back
// to running the linker-signed binary unchanged if that certificate isn't
// present (e.g. CI, other contributors' machines).
import { execFileSync } from 'node:child_process';

const identity = process.env.MONITTER_DEV_SIGNING_IDENTITY
  ?? 'Apple Development: soyrex@me.com (V53PM3GMXR)';
const [binary, ...args] = process.argv.slice(2);

let available = '';
try {
  available = execFileSync('/usr/bin/security', ['find-identity', '-v', '-p', 'codesigning'], {
    encoding: 'utf8',
  });
} catch {
  available = '';
}

if (available.includes(identity)) {
  try {
    execFileSync('/usr/bin/codesign', ['--force', '--sign', identity, binary], { stdio: 'inherit' });
  } catch (error) {
    console.warn(`dev-codesign-runner: failed to sign with "${identity}", running as-is: ${error.message}`);
  }
}

try {
  execFileSync(binary, args, { stdio: 'inherit' });
} catch (error) {
  process.exit(error.status ?? 1);
}
