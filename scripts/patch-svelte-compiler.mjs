import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/**
 * Temporary Svelte 5.57.0 compiler workaround for repeated snippet ancestor expansion.
 * The CJS WeakMap uses AST node identities as weak keys, so a later compile cannot reuse
 * entries from an earlier AST and dead compile trees are not retained by the cache.
 * Remove this postinstall hook, this script, and its regression test when a Svelte upgrade
 * contains the upstream fix. To upgrade before then, inspect the new compiler and update
 * the exact-version/hash guards below; never apply the old transformation to unknown files.
 */
export const SUPPORTED_SVELTE_VERSION = '5.57.0';

const TARGETS = [
  {
    key: 'esm',
    relativePath: 'src/compiler/phases/2-analyze/css/css-prune.js',
    pristineSha256: '941bad05a9f0a3638c0fa07db52ff40b9f3b96bdb06f974e87ff96e6889dbcf8',
    patchedSha256: 'ff63771da811086d8aab608f134f57f55a4750da4e1eef1191a71b5bc6889272'
  },
  {
    key: 'cjs',
    relativePath: 'compiler/index.js',
    pristineSha256: '38f88d68baa287815cb6ea6523fac7beccced2403ca2daea6c385d60f42d78a5',
    patchedSha256: '8ef842b8a5a127cb19ebfaf1d5ba6df14a96fef2b1e343572614c67e5c8842fa'
  }
];

const ESM_SEEN_ANCHOR = 'const seen = new Set();';
const ESM_PRUNE_ANCHOR = 'export function prune(stylesheet, elements) {\n\twalk(';
const ESM_CACHE_BLOCK = `const seen = new Set();

/** @type {WeakMap<object, Map<boolean, Array<Compiler.AST.RegularElement | Compiler.AST.SvelteElement>>>} */
let ancestor_cache = new WeakMap();`;
const ESM_PRUNE_PATCHED = 'export function prune(stylesheet, elements) {\n\tancestor_cache = new WeakMap();\n\twalk(';
const ESM_ANCESTOR_DOC = `/**
 * @param {Compiler.AST.RegularElement | Compiler.AST.SvelteElement | Compiler.AST.RenderTag | Compiler.AST.Component | Compiler.AST.SvelteComponent | Compiler.AST.SvelteSelf} node
 * @param {boolean} adjacent_only
 * @param {Set<Compiler.AST.SnippetBlock>} seen
 */
function get_ancestor_elements(node, adjacent_only, seen = new Set()) {`;
const ESM_RECURSIVE_CALL = 'ancestors.push(...get_ancestor_elements(site, adjacent_only, seen));';
const ESM_INNER_HEADER = ESM_ANCESTOR_DOC.replace('function get_ancestor_elements(', 'function get_ancestor_elements_inner(');
const ESM_WRAPPER = `/**
 * @param {Compiler.AST.RegularElement | Compiler.AST.SvelteElement | Compiler.AST.RenderTag | Compiler.AST.Component | Compiler.AST.SvelteComponent | Compiler.AST.SvelteSelf} node
 * @param {boolean} adjacent_only
 */
function get_ancestor_elements(node, adjacent_only) {
	const cached = ancestor_cache.get(node)?.get(adjacent_only);
	if (cached) return cached;

	const ancestors = [...new Set(get_ancestor_elements_inner(node, adjacent_only, new Set()))];
	const values = ancestor_cache.get(node) ?? new Map();
	values.set(adjacent_only, ancestors);
	ancestor_cache.set(node, values);
	return ancestors;
}

`;

const CJS_ANCESTOR_START = 'function vp(e,n,s=new Set){';
const CJS_ANCESTOR_NEXT = 'function gp(';
const CJS_WRAPPER = 'function vp(e,n,s){if(s!==undefined)return __monitterAncestorWalk(e,n,s);let t=__monitterAncestorCache.get(e);if(t&&t.has(n))return t.get(n);const r=[...new Set(__monitterAncestorWalk(e,n,new Set))];return t||(t=new Map,__monitterAncestorCache.set(e,t)),t.set(n,r),r}';
const CJS_CACHE_AND_WRAPPER = `const __monitterAncestorCache=new WeakMap;${CJS_WRAPPER}`;

const sha256 = (content) => createHash('sha256').update(content).digest('hex');

export function buildEsmPatch(content) {
  if (content.split(ESM_SEEN_ANCHOR).length !== 2) throw new Error('ESM cache anchor count changed');
  if (content.split(ESM_PRUNE_ANCHOR).length !== 2) throw new Error('ESM prune anchor count changed');
  if (content.split(ESM_ANCESTOR_DOC).length !== 2) throw new Error('ESM ancestor function anchor count changed');
  if (content.split(ESM_RECURSIVE_CALL).length !== 2) throw new Error('ESM recursive call anchor count changed');

  return content
    .replace(ESM_SEEN_ANCHOR, ESM_CACHE_BLOCK)
    .replace(ESM_PRUNE_ANCHOR, ESM_PRUNE_PATCHED)
    .replace(ESM_ANCESTOR_DOC, `${ESM_WRAPPER}${ESM_INNER_HEADER}`)
    .replace(ESM_RECURSIVE_CALL, 'ancestors.push(...get_ancestor_elements_inner(site, adjacent_only, seen));');
}

export function buildCjsPatch(content) {
  if (content.split(CJS_ANCESTOR_START).length !== 2) throw new Error('CJS ancestor function anchor count changed');
  const start = content.indexOf(CJS_ANCESTOR_START);
  const end = content.indexOf(CJS_ANCESTOR_NEXT, start);
  if (end < 0) throw new Error('CJS ancestor function terminator anchor missing');
  const originalFunction = content.slice(start, end);
  if (!originalFunction.endsWith('}')) throw new Error('CJS ancestor function body changed');
  const innerFunction = originalFunction.replace(CJS_ANCESTOR_START, 'function __monitterAncestorWalk(e,n,s=new Set){');
  return content.slice(0, start) + CJS_CACHE_AND_WRAPPER + innerFunction + content.slice(end);
}

/** Reconstructs the guarded pristine sources for regression tests only; never writes files. */
export function restorePristineForTest(targetKey, content) {
  const target = TARGETS.find(({ key }) => key === targetKey);
  if (!target) throw new Error(`Unknown compiler target: ${targetKey}`);
  const digest = sha256(content);
  if (digest === target.pristineSha256) return content;
  if (digest !== target.patchedSha256) throw new Error(`Cannot reconstruct pristine ${target.relativePath}: unknown file digest ${digest}`);

  let restored;
  if (target.key === 'esm') {
    if (content.split(ESM_CACHE_BLOCK).length !== 2) throw new Error('Patched ESM cache block mismatch');
    if (content.split(ESM_PRUNE_PATCHED).length !== 2) throw new Error('Patched ESM prune block mismatch');
    if (content.split(`${ESM_WRAPPER}${ESM_INNER_HEADER}`).length !== 2) throw new Error('Patched ESM ancestor wrapper mismatch');
    if (content.split('ancestors.push(...get_ancestor_elements_inner(site, adjacent_only, seen));').length !== 2) throw new Error('Patched ESM recursive helper mismatch');
    restored = content
      .replace(ESM_CACHE_BLOCK, ESM_SEEN_ANCHOR)
      .replace(ESM_PRUNE_PATCHED, ESM_PRUNE_ANCHOR)
      .replace(`${ESM_WRAPPER}${ESM_INNER_HEADER}`, ESM_ANCESTOR_DOC)
      .replace('ancestors.push(...get_ancestor_elements_inner(site, adjacent_only, seen));', ESM_RECURSIVE_CALL);
  } else {
    const patchedStart = CJS_CACHE_AND_WRAPPER;
    const start = content.indexOf(patchedStart);
    const helperStart = content.indexOf('function __monitterAncestorWalk(e,n,s=new Set){', start);
    const end = content.indexOf(CJS_ANCESTOR_NEXT, helperStart);
    if (start < 0 || helperStart < 0 || end < 0) throw new Error('Patched CJS ancestor wrapper mismatch');
    const innerFunction = content.slice(helperStart, end).replace('function __monitterAncestorWalk(', 'function vp(');
    restored = content.slice(0, start) + innerFunction + content.slice(end);
  }

  if (sha256(restored) !== target.pristineSha256) throw new Error(`Pristine ${target.relativePath} reconstruction failed its SHA256 guard`);
  return restored;
}

function parseRoot(args) {
  const rootArg = args.find((arg) => arg === '--root' || arg.startsWith('--root='));
  if (!rootArg) {
    const require = createRequire(import.meta.url);
    return path.dirname(require.resolve('svelte/package.json'));
  }
  if (rootArg === '--root') {
    const index = args.indexOf(rootArg);
    if (!args[index + 1] || args[index + 1].startsWith('--')) throw new Error('--root requires a Svelte package directory');
    return path.resolve(args[index + 1]);
  }
  return path.resolve(rootArg.slice('--root='.length));
}

function stateFor(target, current) {
  const hash = sha256(current);
  if (hash === target.patchedSha256) return { state: 'patched', content: current };
  if (hash !== target.pristineSha256) {
    throw new Error(`${target.relativePath} is neither pristine Svelte ${SUPPORTED_SVELTE_VERSION} nor the exact expected patch. Review Svelte's compiler changes and update this workaround for the new version before installing.`);
  }
  const patched = target.key === 'esm' ? buildEsmPatch(current) : buildCjsPatch(current);
  const patchedHash = sha256(patched);
  if (patchedHash !== target.patchedSha256) {
    throw new Error(`Internal ${target.key.toUpperCase()} patch digest mismatch (${patchedHash}); update the guarded digest before applying.`);
  }
  return { state: 'pristine', content: patched };
}

export async function patchSvelteCompiler(root) {
  const packageJson = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
  if (packageJson.version !== SUPPORTED_SVELTE_VERSION) {
    throw new Error(`Expected Svelte ${SUPPORTED_SVELTE_VERSION}, found ${packageJson.version ?? 'unknown'}. Review the compiler diff and update scripts/patch-svelte-compiler.mjs before upgrading Svelte.`);
  }

  // Validate both compiler entry points before writing either one, so unknown content cannot cause a partial patch.
  const plans = await Promise.all(TARGETS.map(async (target) => {
    const file = path.join(root, target.relativePath);
    const current = await readFile(file, 'utf8');
    return { ...target, file, ...stateFor(target, current) };
  }));

  for (const plan of plans) {
    if (plan.state === 'pristine') await writeFile(plan.file, plan.content);
  }
  const patchedCount = plans.filter((plan) => plan.state === 'pristine').length;
  console.log(patchedCount === 0 ? `Svelte ${SUPPORTED_SVELTE_VERSION} compiler cache already applied.` : `Applied guarded Svelte ${SUPPORTED_SVELTE_VERSION} compiler cache to ${patchedCount} file(s).`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    await patchSvelteCompiler(parseRoot(process.argv.slice(2)));
  } catch (error) {
    console.error(`Svelte compiler workaround failed: ${error.message}`);
    process.exitCode = 1;
  }
}
