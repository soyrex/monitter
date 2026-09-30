// Prepare a NEW, explicitly test-only frontend directory for a separately
// identified native app. This never changes build/, installs an app, seeds data,
// or replaces the native bridge. All interactions remain real Tauri IPC.
import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = join(root, 'build');
const metricsPath = join(root, 'scripts/native-ipc-metrics.js');
const [output, ...extra] = process.argv.slice(2);
if (!output || extra.length || !isAbsolute(output)) {
  throw new Error('Usage: prepare-native-ipc-ui.mjs <absolute-new-directory-under-worktree-artifacts>');
}
const artifactRoot = realpathSync(join(root, 'artifacts'));
const parent = dirname(output);
if (lstatSync(parent).isSymbolicLink() || !lstatSync(parent).isDirectory()) {
  throw new Error('Output parent must be a real existing directory.');
}
const parentRelative = relative(artifactRoot, realpathSync(parent));
if (parentRelative === '..' || parentRelative.startsWith(`..${sep}`) || isAbsolute(parentRelative)) {
  throw new Error('Output must be under this worktree artifact directory.');
}
if (existsSync(output)) throw new Error('Refusing an existing output directory.');
// lstat also detects dangling links, which existsSync intentionally ignores.
try { lstatSync(output); throw new Error('Refusing an existing output path.'); }
catch (error) { if (error.code !== 'ENOENT') throw error; }

function validateTree(directory) {
  const metadata = lstatSync(directory);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) throw new Error('Build source must be a real directory.');
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`Refusing linked build asset: ${entry.name}`);
    if (entry.isDirectory()) validateTree(path);
    else if (!entry.isFile()) throw new Error(`Refusing non-file build asset: ${entry.name}`);
  }
}
validateTree(source);
const html = readFileSync(join(source, 'index.html'), 'utf8');
const metrics = readFileSync(metricsPath, 'utf8');
if (html.split('<head>').length !== 2 || html.includes('__native-ipc-metrics.js')) {
  throw new Error('Expected one uninstrumented HTML head.');
}
const instrumented = html.replace('<head>', '<head><script src="/__native-ipc-metrics.js"></script>');
mkdirSync(output, { mode: 0o700 });
for (const entry of readdirSync(source)) {
  cpSync(join(source, entry), join(output, entry), { recursive: true, errorOnExist: true, force: false });
}
writeFileSync(join(output, 'index.html'), instrumented);
writeFileSync(join(output, '__native-ipc-metrics.js'), metrics, { flag: 'wx' });
const sha256 = value => createHash('sha256').update(value).digest('hex');
const manifest = {
  purpose: 'Synthetic test-only metrics on the real native bridge. Not production UI.',
  sourceHtmlSha256: sha256(html),
  metricsSha256: sha256(metrics),
  instrumentedHtmlSha256: sha256(instrumented),
};
writeFileSync(join(output, '__native-ipc-ui-manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
console.log(JSON.stringify({ output, ...manifest }, null, 2));
