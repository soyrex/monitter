import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { cp, lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { restorePristineForTest, SUPPORTED_SVELTE_VERSION } from './patch-svelte-compiler.mjs';

const TARGET_FILES = {
  esm: 'src/compiler/phases/2-analyze/css/css-prune.js',
  cjs: 'compiler/index.js'
};
const require = createRequire(import.meta.url);
const patchScript = fileURLToPath(new URL('./patch-svelte-compiler.mjs', import.meta.url));
const fixtureDir = fileURLToPath(new URL('./fixtures/', import.meta.url));
const output = (result) => `${result.stdout ?? ''}${result.stderr ?? ''}`;

function svelteRootFromArgs(args) {
  if (args[0] === '--root' && args[1]) return path.resolve(args[1]);
  if (args[0]?.startsWith('--root=')) return path.resolve(args[0].slice('--root='.length));
  return path.dirname(require.resolve('svelte/package.json'));
}

async function ensureDependencyLink(packageRoot, dependencyRoot) {
  try {
    await lstat(path.join(packageRoot, 'node_modules'));
  } catch {
    await symlink(dependencyRoot, path.join(packageRoot, 'node_modules'), 'dir');
  }
}

async function copyPackage(sourceRoot, destinationRoot, dependencyRoot) {
  await mkdir(path.dirname(destinationRoot), { recursive: true });
  await cp(sourceRoot, destinationRoot, { recursive: true });
  await ensureDependencyLink(destinationRoot, dependencyRoot);
}

async function loadCjsCompiler(packageRoot, testRoot, key) {
  const projectRoot = path.join(testRoot, `cjs-${key}`);
  const alias = path.join(projectRoot, 'node_modules', 'svelte');
  await mkdir(path.dirname(alias), { recursive: true });
  await symlink(packageRoot, alias, 'dir');
  const localRequire = createRequire(path.join(projectRoot, 'runner.cjs'));
  return localRequire('svelte/compiler');
}

function compareResult(baseline, patched) {
  assert.equal(patched.js.code, baseline.js.code, 'compiled JavaScript changed');
  assert.equal(patched.css?.code ?? null, baseline.css?.code ?? null, 'compiled CSS changed');
  assert.equal(JSON.stringify(patched.warnings), JSON.stringify(baseline.warnings), 'compiler warnings changed');
}

const sourceRoot = svelteRootFromArgs(process.argv.slice(2));
const sourcePackage = JSON.parse(await readFile(path.join(sourceRoot, 'package.json'), 'utf8'));
assert.equal(sourcePackage.version, SUPPORTED_SVELTE_VERSION, `test requires Svelte ${SUPPORTED_SVELTE_VERSION}`);

const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'monitter-svelte-ancestor-cache-'));
try {
  const dependencyRoot = path.resolve('node_modules');
  const baselineRoot = path.join(tempRoot, 'baseline', 'node_modules', 'svelte');
  const patchedRoot = path.join(tempRoot, 'patched', 'node_modules', 'svelte');
  await copyPackage(sourceRoot, baselineRoot, dependencyRoot);
  await copyPackage(sourceRoot, patchedRoot, dependencyRoot);

  // The package may already have been patched by postinstall; reconstruct and SHA-check its
  // pristine sources in the test copy so the regression remains runnable after installation.
  for (const packageRoot of [baselineRoot, patchedRoot]) {
    for (const key of ['esm', 'cjs']) {
      const file = path.join(packageRoot, TARGET_FILES[key]);
      const restored = restorePristineForTest(key, await readFile(file, 'utf8'));
      await writeFile(file, restored);
    }
  }

  const patchResult = spawnSync(process.execPath, [patchScript, '--root', patchedRoot], { encoding: 'utf8' });
  assert.equal(patchResult.status, 0, `explicit-root patch failed: ${output(patchResult)}`);
  assert.match(output(patchResult), /Applied guarded Svelte 5\.57\.0 compiler cache/);
  const repeatPatch = spawnSync(process.execPath, [patchScript, `--root=${patchedRoot}`], { encoding: 'utf8' });
  assert.equal(repeatPatch.status, 0, `idempotent patch failed: ${output(repeatPatch)}`);
  assert.match(output(repeatPatch), /already applied/);

  const versionRoot = path.join(tempRoot, 'wrong-version', 'node_modules', 'svelte');
  await copyPackage(baselineRoot, versionRoot, dependencyRoot);
  const versionFile = path.join(versionRoot, 'package.json');
  const versionPackage = JSON.parse(await readFile(versionFile, 'utf8'));
  versionPackage.version = '5.57.1';
  await writeFile(versionFile, `${JSON.stringify(versionPackage, null, 2)}\n`);
  const versionFailure = spawnSync(process.execPath, [patchScript, '--root', versionRoot], { encoding: 'utf8' });
  assert.notEqual(versionFailure.status, 0, 'unsupported version was accepted');
  assert.match(output(versionFailure), /update scripts\/patch-svelte-compiler\.mjs before upgrading/);
  assert.equal(await readFile(path.join(versionRoot, TARGET_FILES.esm), 'utf8'), await readFile(path.join(baselineRoot, TARGET_FILES.esm), 'utf8'));

  const unknownRoot = path.join(tempRoot, 'unknown-cjs', 'node_modules', 'svelte');
  await copyPackage(baselineRoot, unknownRoot, dependencyRoot);
  const unknownCjsFile = path.join(unknownRoot, TARGET_FILES.cjs);
  await writeFile(unknownCjsFile, `${await readFile(unknownCjsFile, 'utf8')}\n/* unknown change */`);
  const unknownFailure = spawnSync(process.execPath, [patchScript, '--root', unknownRoot], { encoding: 'utf8' });
  assert.notEqual(unknownFailure.status, 0, 'unknown compiler content was accepted');
  assert.match(output(unknownFailure), /neither pristine Svelte 5\.57\.0 nor the exact expected patch/);
  assert.equal(await readFile(path.join(unknownRoot, TARGET_FILES.esm), 'utf8'), await readFile(path.join(baselineRoot, TARGET_FILES.esm), 'utf8'), 'unknown CJS content partially patched ESM');

  const esmModules = {
    baseline: await import(pathToFileURL(path.join(baselineRoot, 'src/compiler/index.js'))),
    patched: await import(pathToFileURL(path.join(patchedRoot, 'src/compiler/index.js')))
  };
  const cjsModules = {
    baseline: await loadCjsCompiler(baselineRoot, tempRoot, 'baseline'),
    patched: await loadCjsCompiler(patchedRoot, tempRoot, 'patched')
  };
  const fixtures = await Promise.all([
    'svelte-ancestor-cache-snippets.svelte',
    'svelte-ancestor-cache-conditional.svelte',
    'svelte-ancestor-cache-selectedcontent.svelte'
  ].map((name) => readFile(path.join(fixtureDir, name), 'utf8')));

  let compilerComparisons = 0;
  for (const compilerKind of ['esm', 'cjs']) {
    const compilers = compilerKind === 'esm'
      ? esmModules
      : cjsModules;

    // Repeating the first fixture after the second checks that the CJS WeakMap cannot reuse
    // entries from a prior compile and the ESM cache is reset for each prune pass.
    for (const index of [0, 1, 2, 0]) {
      const filename = `ancestor-cache-${compilerKind}-${index}.svelte`;
      const options = { filename, generate: 'client' };
      const baseline = compilers.baseline.compile(fixtures[index], options);
      const patched = compilers.patched.compile(fixtures[index], options);
      compareResult(baseline, patched);
      compilerComparisons += 1;
    }
  }

  console.log(`Svelte ${SUPPORTED_SVELTE_VERSION} ancestor-cache regression passed: ${compilerComparisons} ESM/CJS compile comparisons plus version, unknown-content atomicity, and idempotence guards.`);
} finally {
  await rm(tempRoot, { recursive: true, force: true });
}
