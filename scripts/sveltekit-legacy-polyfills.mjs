import { readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return (await Promise.all(entries.map(entry => {
    const filename = path.join(directory, entry.name);
    return entry.isDirectory() ? files(filename) : [filename];
  }))).flat();
}

// Kit generates its HTML after Vite's transformIndexHtml hooks have run. Await
// plugin-legacy's modern polyfills before importing either client entry, rather
// than emitting an unused chunk or racing a second module script against Kit.
export function withLegacyPolyfills(adapter) {
  return {
    ...adapter,
    async adapt(builder) {
      await adapter.adapt(builder);
      const client = builder.getClientDirectory();
      const chunks = (await files(client)).filter(file => /[/\\]polyfills\.[\w-]+\.js$/.test(file));
      // No chunk means the target browsers already implement every detected API.
      if (!chunks.length) return;
      if (chunks.length !== 1) throw new Error('Expected one plugin-legacy modern polyfill chunk');
      const relative = path.relative(client, chunks[0]).split(path.sep).join('/');
      const source = `${builder.config.kit.paths.base}/${relative}`;
      let bootstraps = 0;
      // This adapter is paired with adapter-static's default build directory.
      for (const filename of (await files('build')).filter(file => file.endsWith('.html'))) {
        const html = await readFile(filename, 'utf8');
        if (!html.includes('kit.start(app, element')) continue;
        let matches = 0;
        const updated = html.replace(/Promise\.all\(\[([\s\S]*?)\]\)\.then\(\(\[kit, app\]\)/g, (_, imports) => {
          matches++;
          return `import(${JSON.stringify(source)}).then(() => Promise.all([${imports}])).then(([kit, app])`;
        });
        if (matches !== 1) throw new Error(`Unrecognised SvelteKit bootstrap in ${filename}; polyfills would not load`);
        await writeFile(filename, updated);
        bootstraps++;
      }
      if (!bootstraps) throw new Error('No SvelteKit HTML bootstrap found for plugin-legacy polyfills');
      builder.log(`Loaded modern browser polyfills before ${bootstraps} client bootstrap(s)`);
    },
  };
}
