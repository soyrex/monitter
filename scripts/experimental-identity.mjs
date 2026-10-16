import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';

export const EXPERIMENTAL_ID = 'com.monitter.gpui.experimental';
export const EXPERIMENTAL_NAME = 'Monitter GPUI';

export function assertExperimentalConfig(root) {
  const config = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  if (config.identifier !== EXPERIMENTAL_ID || config.productName !== EXPERIMENTAL_NAME) {
    throw new Error('Refusing to package or install without the isolated Monitter GPUI identity.');
  }
}

export function assertExperimentalBundle(app) {
  const identifier = execFileSync('/usr/bin/plutil', [
    '-extract', 'CFBundleIdentifier', 'raw', '-o', '-', join(app, 'Contents/Info.plist'),
  ], { encoding: 'utf8' }).trim();
  if (identifier !== EXPERIMENTAL_ID) {
    throw new Error(`Refusing non-experimental application bundle: ${app}`);
  }
}
