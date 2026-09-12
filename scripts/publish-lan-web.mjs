import { copyFileSync, existsSync, mkdirSync, readdirSync, renameSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

// Publish assets first, then atomically switch the entry page. Keep previous
// hashed chunks: an already-open browser may still request one lazily.
export function publishLanWeb(source, destination) {
  const entry = join(source, 'index.html');
  if (!existsSync(entry) || !statSync(entry).isFile()) throw new Error('Build has no index.html; live LAN assets unchanged.');
  function copyDirectory(from, to, root = false) {
    mkdirSync(to, { recursive: true });
    for (const item of readdirSync(from, { withFileTypes: true })) {
      if (root && item.name === 'index.html') continue;
      const input = join(from, item.name), output = join(to, item.name);
      if (item.isDirectory()) copyDirectory(input, output);
      else if (item.isFile()) {
        const staged = `${output}.${randomUUID()}.tmp`;
        copyFileSync(input, staged);
        renameSync(staged, output);
      } else throw new Error(`Unsupported build entry: ${input}`);
    }
  }
  copyDirectory(source, destination, true);
  const staged = join(destination, `index.${randomUUID()}.tmp`);
  copyFileSync(entry, staged);
  renameSync(staged, join(destination, 'index.html'));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.platform === 'darwin') {
    const source = resolve(dirname(fileURLToPath(import.meta.url)), '../build');
    const destination = join(homedir(), 'Library/Application Support/com.monitter.desktop/lan-web');
    publishLanWeb(source, destination);
    console.log(`Published live LAN web assets: ${destination}`);
  } else console.log('LAN web auto-publish is macOS-only; web build is available in build/.');
}
