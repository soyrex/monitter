import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertExperimentalConfig, EXPERIMENTAL_ID } from './experimental-identity.mjs';
import { publishLanWeb } from './publish-lan-web.mjs';
import { webDevProxy } from './vite-web-dev.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = path => readFileSync(resolve(root, path), 'utf8');

test('experimental identity and installer cannot target the production bundle', () => {
  assertExperimentalConfig(root);
  assert.equal(EXPERIMENTAL_ID, 'com.monitter.gpui.experimental');
  const installer = read('scripts/install-macos.mjs');
  assert.match(installer, /const target = '\/Applications\/Monitter GPUI\.app'/);
  assert.doesNotMatch(installer, /\/Applications\/Monitter\.app/);
  assert.match(installer, /manifest\.identifier !== EXPERIMENTAL_ID/);
  assert.match(installer, /assertExperimentalBundle\(source\)/);
  assert.match(installer, /if \(existsSync\(target\)\) assertExperimentalBundle\(target\)/);
});

test('publisher rejects production profile before reading or writing assets', () => {
  for (const suffix of ['', '/lan-web', '/lan-web/../state.json']) {
    assert.throws(() => publishLanWeb('/nonexistent-build', `${homedir()}/Library/Application Support/com.monitter.desktop${suffix}`), /Refusing to publish/);
  }
});

test('desktop development proxy and runtime ports stay isolated', () => {
  const proxy = Object.values(webDevProxy())[0];
  assert.equal(proxy.target, 'http://127.0.0.1:19436');
  assert.equal(proxy.headers.origin, proxy.target);
  for (const path of ['src-tauri/src/dev_ui.rs', 'src-tauri/src/lan.rs', 'vite.config.js', 'scripts/vite-web-dev.mjs', 'src-tauri/capabilities/hot-ui.json']) {
    assert.doesNotMatch(read(path), /184(?:20|21|36|37|50)|18_43[67]/, path);
  }
  assert.match(read('src-tauri/src/lan.rs'), /pub const PORT: u16 = 19_436/);
  assert.match(read('src-tauri/src/lan.rs'), /pub const TLS_PORT: u16 = 19_437/);
  const capability = JSON.parse(read('src-tauri/capabilities/hot-ui.json'));
  assert.deepEqual(capability.remote.urls, ['http://127.0.0.1:19420/monitter-app-ui/*']);
});

test('private credential stores do not use production keychain services', () => {
  for (const path of ['src-tauri/src/mail.rs', 'src-tauri/src/environment_secrets.rs']) {
    assert.match(read(path), /const KEYCHAIN_SERVICE: &str = "com\.monitter\.gpui\.experimental\./);
    assert.doesNotMatch(read(path), /com\.monitter\.desktop/);
  }
});

test('macOS browser storage has a separate native identity', () => {
  const browser = read('src-tauri/src/native_browser.rs');
  assert.match(browser, /data_store_identifier\(\*b"monitter-gpui-v1"\)/);
  assert.doesNotMatch(browser, /\*b"monitter-browser"/);
});
