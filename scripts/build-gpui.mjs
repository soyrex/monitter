import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runCargo } from './gpui-toolchain.mjs';
import { checkBuildSpace, defaultTarget, recordDisk } from './gpui-disk.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
if (!['darwin', 'linux'].includes(process.platform)) {
  throw new Error('The experimental GPUI build supports macOS and Linux.');
}

let release = false;
let visualExport = false;
let jobs = 2;
for (let i = 2; i < process.argv.length; i += 1) {
  const arg = process.argv[i];
  if (arg === '--release') {
    release = true;
  } else if (arg === '--visual-export') {
    visualExport = true;
  } else if (arg === '--jobs') {
    const value = process.argv[++i];
    if (!value || !/^\d+$/.test(value) || Number(value) < 1) {
      throw new Error('--jobs requires a positive integer.');
    }
    jobs = Number(value);
  } else {
    throw new Error(`Unknown build:gpui option: ${arg}`);
  }
}
if (release && visualExport) {
  throw new Error('Visual export is a development feature; release measurements must use the normal renderer.');
}

const profile = release ? 'release' : 'debug';
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(root, process.env.CARGO_TARGET_DIR)
  : defaultTarget;
const binary = join(targetDir, profile, 'monitter-gpui-desktop');
const serverBinary = join(targetDir, profile, 'monitter-gpui-server');
const manifest = join(root, 'desktop-gpui', 'Cargo.toml');
const icon = join(root, 'src-tauri', 'icons', 'icon.icns');
const version = '0.1.0';
checkBuildSpace(targetDir, { warmDebug: !release && existsSync(binary) && existsSync(serverBinary) });
process.on('exit', () => {
  try { recordDisk('after-build', targetDir); }
  catch (error) { console.error(`Could not record build disk usage: ${error.message}`); }
});

const cargoArgs = ['build', '--manifest-path', manifest, '--locked', '--bins', '--jobs', String(jobs)];
if (release) cargoArgs.push('--release');
if (visualExport) cargoArgs.push('--features', 'visual-export');
console.log(`Building experimental GPUI desktop (${profile}, ${jobs} jobs).`);
console.log('This is separate from the Tauri build:mac artifact and will not install or launch either app.');
const cargoStatus = runCargo(cargoArgs, { cwd: root });
if (cargoStatus !== 0) throw new Error(`GPUI cargo build failed with exit status ${cargoStatus}; no app bundle was staged.`);

if (!existsSync(binary)) throw new Error(`Cargo succeeded but the GPUI executable is missing: ${binary}`);
if (!existsSync(serverBinary)) throw new Error(`Cargo succeeded but the GPUI server is missing: ${serverBinary}`);
if (process.platform === 'linux') {
  const artifacts = join(root, 'artifacts', 'gpui', profile);
  mkdirSync(artifacts, { recursive: true });
  writeFileSync(join(artifacts, 'latest.json'), `${JSON.stringify({ app: binary, serverApp: serverBinary, profile, platform: 'linux', visualExport, builtAt: new Date().toISOString() }, null, 2)}\n`);
  console.log(`Built Linux GUI: ${binary}`);
  console.log(`Built independent Linux backend: ${serverBinary}`);
  process.exit(0);
}

if (!existsSync(icon)) throw new Error(`The app icon is missing: ${icon}`);

const artifacts = join(root, 'artifacts', 'gpui', profile);
mkdirSync(artifacts, { recursive: true });
const staging = mkdtempSync(join(artifacts, '.Monitter-GPUI-'));
const app = join(staging, 'Monitter GPUI.app');
const contents = join(app, 'Contents');
const executableDir = join(contents, 'MacOS');
const resources = join(contents, 'Resources');
mkdirSync(executableDir, { recursive: true });
mkdirSync(resources, { recursive: true });
copyFileSync(binary, join(executableDir, 'monitter-gpui-desktop'));
copyFileSync(icon, join(resources, 'AppIcon.icns'));
const lanWeb = join(root, 'build');
const lanAssets = existsSync(join(lanWeb, 'index.html'));
if (lanAssets) {
  cpSync(lanWeb, join(resources, 'lan-web'), { recursive: true });
} else {
  console.warn('LAN client assets are absent. Build the web client before testing native LAN access.');
}
// Font bytes are embedded in the executable; redistribute their notices too.
const fontNotices = join(resources, 'licenses', 'ibm-plex');
mkdirSync(fontNotices, { recursive: true });
for (const notice of ['IBM-Plex-Sans-LICENSE.txt', 'IBM-Plex-Mono-LICENSE.txt', 'SOURCE.txt']) {
  copyFileSync(join(root, 'desktop-gpui', 'assets', 'fonts', notice), join(fontNotices, notice));
}
const brandingNotices = join(resources, 'licenses', 'branding');
mkdirSync(brandingNotices, { recursive: true });
copyFileSync(
  join(root, 'desktop-gpui', 'assets', 'branding', 'LICENSE-LOBEHUB.txt'),
  join(brandingNotices, 'LICENSE-LOBEHUB.txt'),
);
writeFileSync(join(contents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>monitter-gpui-desktop</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>com.monitter.gpui.experimental</string>
  <key>CFBundleName</key><string>Monitter</string>
  <key>CFBundleDisplayName</key><string>Monitter</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Record a voice message for local transcription into your editable chat draft.</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
`);

// The server is a separate menu-bar application. GUI updates never replace
// or signal an already running server; each build keeps both signed bundles.
if (!existsSync(serverBinary)) throw new Error(`Backend server executable is missing: ${serverBinary}`);
const serverApp = join(staging, 'Monitter GPUI Server.app');
const serverContents = join(serverApp, 'Contents');
mkdirSync(join(serverContents, 'MacOS'), { recursive: true });
mkdirSync(join(serverContents, 'Resources'), { recursive: true });
copyFileSync(serverBinary, join(serverContents, 'MacOS', 'monitter-gpui-server'));
copyFileSync(icon, join(serverContents, 'Resources', 'AppIcon.icns'));
if (lanAssets) cpSync(lanWeb, join(serverContents, 'Resources', 'lan-web'), { recursive: true });
writeFileSync(join(serverContents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>monitter-gpui-server</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleIdentifier</key><string>com.monitter.gpui.experimental.server</string>
<key>CFBundleName</key><string>Monitter GPUI Server</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${version}</string>
<key>CFBundleVersion</key><string>${version}</string>
<key>LSUIElement</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
`);
execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', serverApp], { stdio: 'inherit' });
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', serverApp], { stdio: 'inherit' });

execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', app], { stdio: 'inherit' });
execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', app], { stdio: 'inherit' });

// A preview may still be running from an earlier bundle. Keep every new build
// separate; launch/restart is an explicit later step, never a packaging side effect.
const buildDir = join(artifacts, `build-${Date.now()}`);
mkdirSync(buildDir, { recursive: true });
const output = join(buildDir, 'Monitter GPUI.app');
renameSync(app, output);
const serverOutput = join(buildDir, 'Monitter GPUI Server.app');
renameSync(serverApp, serverOutput);
rmSync(staging, { recursive: true, force: true });
const latestPath = join(artifacts, 'latest.json');
const latestTemporaryPath = join(artifacts, `.latest-${process.pid}-${Date.now()}.json`);
writeFileSync(latestTemporaryPath, `${JSON.stringify({ app: output, serverApp: serverOutput, profile, visualExport, lanAssets, builtAt: new Date().toISOString() }, null, 2)}\n`, { flag: 'wx' });
renameSync(latestTemporaryPath, latestPath);
console.log(`Packaged experimental GPUI app: ${output}`);
console.log(`Packaged persistent GPUI server: ${serverOutput}`);
