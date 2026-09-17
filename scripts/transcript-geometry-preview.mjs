// Loopback-only, in-memory fixture for diagnosing transcript geometry. It
// imports the real MessagePane, TranscriptVirtualList, Markdown and buffer;
// it never starts the app, calls native IPC, or writes an output bundle.
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

const root = process.cwd();
const args = process.argv.slice(2);
const buildOnly = args.includes('--build-only');
const portArgument = args.find((value) => value.startsWith('--port='));
const port = portArgument ? Number(portArgument.slice('--port='.length)) : 18434;
if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Expected --port=<1..65535>.');

const lucideStub = fileURLToPath(new URL('./fixtures/transcript-geometry-lucide-stub.js', import.meta.url));
const started = performance.now();
const result = await build({
  configFile: false,
  root,
  plugins: [svelte()],
  resolve: { alias: { '$lib': `${root}/src/lib`, '@lucide/svelte': lucideStub } },
  build: {
    write: false,
    minify: false,
    rollupOptions: { input: 'scripts/fixtures/transcript-geometry-entry.js' },
  },
});
const elapsed = performance.now() - started;
const files = new Map(result.output.map((file) => [
  `/${file.fileName}`,
  { body: file.type === 'asset' ? file.source : file.code, type: file.fileName.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/javascript; charset=utf-8' },
]));
const entry = result.output.find((file) => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('Fixture build did not produce an entry chunk.');
const styles = result.output.filter((file) => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', {
  body: `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Synthetic transcript geometry fixture</title><script>if(!new URLSearchParams(location.search).has('monitter-perf')){const u=new URL(location.href);u.searchParams.set('monitter-perf','1');history.replaceState(null,'',u);}</script>${styles.map((file) => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}</head><body><div id="app"></div><script type="module" src="/${entry.fileName}"></script></body></html>`,
  type: 'text/html; charset=utf-8',
});
console.log(`Synthetic geometry fixture compiled in ${elapsed.toFixed(0)}ms (${result.output.length} in-memory assets).`);

if (buildOnly) process.exit(0);

const server = createServer((request, response) => {
  const path = new URL(request.url ?? '/', 'http://127.0.0.1').pathname;
  const asset = files.get(path);
  if (!asset) { response.writeHead(404).end('Not found'); return; }
  response.writeHead(200, { 'content-type': asset.type, 'cache-control': 'no-store' });
  response.end(asset.body);
});
server.listen(port, '127.0.0.1', () => console.log(`Synthetic geometry fixture: http://127.0.0.1:${port}/`));
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close(() => process.exit(0)));
