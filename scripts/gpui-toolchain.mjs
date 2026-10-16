import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MIN_RUST = [1, 95, 0];
// All native commands share one regenerable cache. Avoid accidentally creating
// another multi-gigabyte target directory for check/run versus packaging.
function cargoEnvironment() {
  return {
    ...process.env,
    CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR || join(ROOT, '.cargo-target', 'gpui'),
    CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS || '2',
    CARGO_PROFILE_DEV_DEBUG: process.env.CARGO_PROFILE_DEV_DEBUG || '0',
    CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL || '0',
  };
}

function versionOf(command, args, env = process.env) {
  const result = spawnSync(command, args, { encoding: 'utf8', env });
  if (result.error || result.status !== 0) {
    const detail = result.error?.message ?? result.stderr?.trim() ?? `exit ${result.status}`;
    throw new Error(`Could not run ${command} ${args.join(' ')}: ${detail}`);
  }
  return result.stdout.trim();
}

function parseRustVersion(text) {
  const match = text.match(/rustc\s+(\d+)\.(\d+)\.(\d+)/);
  if (!match) throw new Error(`Could not parse Rust version: ${text}`);
  return match.slice(1).map(Number);
}

function meetsMinimum(version) {
  for (let i = 0; i < MIN_RUST.length; i += 1) {
    if (version[i] !== MIN_RUST[i]) return version[i] > MIN_RUST[i];
  }
  return true;
}

function hostTriple() {
  if (process.platform !== 'darwin') return undefined;
  if (process.arch === 'arm64') return 'aarch64-apple-darwin';
  if (process.arch === 'x64') return 'x86_64-apple-darwin';
  return undefined;
}

export function selectToolchain() {
  const triple = hostTriple();
  if (triple) {
    const localBin = join(ROOT, '.toolchain', 'rustup', 'toolchains', `1.98.1-${triple}`, 'bin');
    const rustc = join(localBin, 'rustc');
    const cargo = join(localBin, 'cargo');
    if (existsSync(rustc) && existsSync(cargo)) {
      const env = {
        ...cargoEnvironment(), PATH: `${localBin}:${process.env.PATH ?? ''}`,
        // rust-objcopy in this official macOS toolchain looks for libLLVM via
        // @rpath. Keep its fallback local to compiler children, never global.
        DYLD_FALLBACK_LIBRARY_PATH: [join(localBin, '..', 'lib'), process.env.DYLD_FALLBACK_LIBRARY_PATH].filter(Boolean).join(':'),
      };
      const rustcVersion = versionOf(rustc, ['--version'], env);
      const parsed = parseRustVersion(rustcVersion);
      if (!meetsMinimum(parsed)) {
        throw new Error(`The local GPUI toolchain is too old (${rustcVersion}); Rust 1.95.0 or newer is required.`);
      }
      return { kind: 'local', bin: localBin, cargo, rustcVersion, env };
    }
  }

  const rustcVersion = versionOf('rustc', ['--version']);
  if (!meetsMinimum(parseRustVersion(rustcVersion))) {
    throw new Error(`GPUI requires Rust 1.95.0 or newer; found ${rustcVersion}. Install the fork-local toolchain or select a newer system toolchain.`);
  }
  const cargoVersion = versionOf('cargo', ['--version']);
  return { kind: 'system', cargo: 'cargo', rustcVersion, cargoVersion, env: cargoEnvironment() };
}

export function runCargo(args, options = {}) {
  const selected = selectToolchain();
  const result = spawnSync(selected.cargo, args, {
    cwd: options.cwd ?? ROOT,
    env: selected.env,
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    const selected = selectToolchain();
    if (args.length === 1 && args[0] === '--version') {
      const cargoVersion = versionOf(selected.cargo, ['--version'], selected.env);
      console.log(`GPUI toolchain: ${selected.kind}; ${selected.rustcVersion}; ${cargoVersion}`);
    } else {
      const cargoArgs = args[0] === 'cargo' ? args.slice(1) : args;
      if (cargoArgs.length === 0) throw new Error('Usage: node scripts/gpui-toolchain.mjs [cargo] <cargo arguments> | --version');
      process.exitCode = runSelectedCargo(selected, cargoArgs);
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

function runSelectedCargo(selected, args) {
  const result = spawnSync(selected.cargo, args, { cwd: ROOT, env: selected.env, stdio: 'inherit' });
  if (result.error) throw result.error;
  return result.status ?? 1;
}
