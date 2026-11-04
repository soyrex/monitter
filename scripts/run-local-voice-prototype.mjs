// Build and launch a separately named local preview with its own backend data.
// This never installs over /Applications or connects to the normal profile.
import { execFileSync } from 'node:child_process';
import { accessSync, constants, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
if (process.platform !== 'darwin') throw new Error('This prototype requires macOS.');
const args = process.argv.slice(2);
if (args.some(arg => !['--no-build', '--prepare-only', '--voice-diagnostics'].includes(arg))) throw new Error('Usage: node scripts/run-local-voice-prototype.mjs [--no-build] [--prepare-only] [--voice-diagnostics]');
const models = resolve(process.env.MONITTER_VOICE_MODELS || join(homedir(), 'Library/Caches/Monitter/VoiceModels'));
if (!existsSync(models) || !statSync(models).isDirectory()) throw new Error('Download the pinned voice models first; see desktop-gpui/native-voice/README.md.');
if (!args.includes('--no-build')) execFileSync(process.execPath, [join(root, 'scripts/build-gpui.mjs')], { cwd: root, stdio: 'inherit' });
const latest = JSON.parse(readFileSync(join(root, 'artifacts/gpui/debug/latest.json'), 'utf8'));
const allowed = realpathSync(join(root, 'artifacts/gpui/debug')) + '/';
for (const path of [latest.app, latest.serverApp]) {
  if (typeof path !== 'string' || !realpathSync(path).startsWith(allowed)) throw new Error('Preview bundles must come from this worktree.');
}
const helperOverride = process.env.MONITTER_NATIVE_VOICE_HELPER;
if (helperOverride && !isAbsolute(helperOverride)) throw new Error('MONITTER_NATIVE_VOICE_HELPER must be an absolute path.');
const helper = helperOverride || join(latest.app, 'Contents/MacOS/monitter-native-voice');
if (!existsSync(helper) || !statSync(helper).isFile()) throw new Error(`Native voice helper is missing: ${helper}`);
accessSync(helper, constants.X_OK);
const helperResources = join(dirname(helper), 'FluidAudio_LuxTtsG2pResources.bundle');
if (!existsSync(helperResources) || !statSync(helperResources).isDirectory()) throw new Error(`Native voice resource bundle is missing: ${helperResources}`);
if (!helperOverride && (!realpathSync(helper).startsWith(realpathSync(latest.app) + '/') || !realpathSync(helperResources).startsWith(realpathSync(latest.app) + '/'))) throw new Error('Packaged voice helper and resources must belong to the selected app.');
const base = join(root, 'artifacts/local-voice-preview');
const directory = join(base, `preview-${Date.now()}`);
const profile = join(base, 'profile/com.monitter.gpui.experimental');
mkdirSync(directory, { recursive: true });
mkdirSync(profile, { recursive: true, mode: 0o700 });
const app = join(directory, 'Monitter Voice Prototype.app');
const server = join(directory, 'Monitter GPUI Server.app');
cpSync(latest.app, app, { recursive: true });
cpSync(latest.serverApp, server, { recursive: true });
const executable = join(app, 'Contents/MacOS/monitter-gpui-desktop');
const realExecutable = `${executable}-real`;
const pidFile = join(directory, 'gui.pid');
const bundleExecutable = execFileSync('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleExecutable', join(app, 'Contents/Info.plist')], { encoding: 'utf8' }).trim();
if (bundleExecutable !== 'monitter-gpui-desktop') throw new Error(`Unexpected preview bundle executable: ${bundleExecutable}`);
renameSync(executable, realExecutable);
// Each staged bundle embeds its exact isolated paths. LaunchServices and direct
// invocations therefore pass through the same environment guard.
const config = {
  PREVIEW_STDOUT: join(directory, 'stdout.log'), PREVIEW_STDERR: join(directory, 'stderr.log'),
  PREVIEW_HELPER: join(app, 'Contents/MacOS/monitter-native-voice'), PREVIEW_MODELS: models,
  PREVIEW_PROFILE: profile, PREVIEW_ROOT: root, PREVIEW_PID: pidFile, PREVIEW_EXECUTABLE: realExecutable,
  PREVIEW_DIAGNOSTICS: args.includes('--voice-diagnostics') ? '1' : '0',
};
writeFileSync(join(directory, 'voice-preview-config.h'), Object.entries(config).map(([key, value]) => `#define ${key} ${JSON.stringify(value)}`).join('\n') + '\n', { mode: 0o600 });
execFileSync('/usr/bin/clang', ['-Wall', '-Wextra', '-Werror', '-I', directory, join(root, 'scripts/voice-preview-launcher.c'), '-o', executable], { stdio: 'inherit' });
for (const [bundle, id, name] of [
  [app, 'com.monitter.local-voice.prototype', 'Monitter Voice Prototype'],
  [server, 'com.monitter.local-voice.prototype.server', 'Monitter Voice Prototype Server'],
]) {
  const plist = join(bundle, 'Contents/Info.plist');
  execFileSync('/usr/libexec/PlistBuddy', ['-c', `Set :CFBundleIdentifier ${id}`, plist]);
  execFileSync('/usr/libexec/PlistBuddy', ['-c', `Set :CFBundleName ${name}`, plist]);
  if (bundle === app) {
    execFileSync('/usr/libexec/PlistBuddy', ['-c', `Set :CFBundleDisplayName ${name}`, plist]);
    execFileSync('/usr/libexec/PlistBuddy', ['-c', 'Set :NSMicrophoneUsageDescription Listen for a local voice conversation with the selected agent.', plist]);
    if (helperOverride) {
      copyFileSync(helper, join(bundle, 'Contents/MacOS/monitter-native-voice'));
      for (const destination of ['Contents/Resources', 'Contents/MacOS']) {
        const target = join(bundle, destination, 'FluidAudio_LuxTtsG2pResources.bundle');
        rmSync(target, { recursive: true, force: true });
        mkdirSync(join(bundle, destination), { recursive: true });
        cpSync(helperResources, target, { recursive: true });
      }
    }
  }
  execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', bundle], { stdio: 'inherit' });
  execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', bundle], { stdio: 'inherit' });
}
if (args.includes('--prepare-only')) {
  console.log(`Voice preview prepared and signature verified: ${app}`);
} else {
  execFileSync('/usr/bin/open', ['-n', app], { stdio: 'inherit' });
  const deadline = Date.now() + 30_000;
  let pid;
  while (Date.now() < deadline) {
    if (existsSync(pidFile)) {
      const value = readFileSync(pidFile, 'utf8').trim();
      if (/^[1-9][0-9]*$/.test(value)) {
        try {
          const command = execFileSync('/bin/ps', ['-p', value, '-o', 'command='], { encoding: 'utf8' }).trim();
          if (command === realExecutable || command.startsWith(`${realExecutable} `)) { pid = Number(value); break; }
        } catch { /* The wrapper may not have exec'd yet or may have exited. */ }
      }
    }
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  }
  if (!pid) throw new Error(`LaunchServices did not start the expected isolated GUI; inspect ${join(directory, 'stderr.log')}.`);
  writeFileSync(join(base, 'latest.json'), JSON.stringify({ app, server, profile, pid, sourceBuild: latest, launchedAt: new Date().toISOString() }, null, 2) + '\n', { mode: 0o600 });
  console.log(`Voice preview launched as PID ${pid}; microphone remains off until Start voice. Profile: ${profile}`);
}
