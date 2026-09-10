import { spawn, execFile } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
import { promisify } from 'node:util';

// Explicit opt-in: executes the real authenticated Codex harness in isolated workspaces.
const selected = process.argv[2];
const allowed = ['local', 'mira', 'error', 'cancel-local', 'cancel-mira'];
assert(allowed.includes(selected), `Choose one smoke case: ${allowed.join(', ')}`);
const root = resolve('verification', `native-${selected}-${Date.now()}`);
await mkdir(root, { recursive: true });
const localCwd = resolve('verification/local-workspace');
await mkdir(localCwd, { recursive: true });
const remoteCwd = '/home/alex/.local/share/monitter/smoke-workspace';
const remote = selected.includes('mira');
const cancel = selected.startsWith('cancel-');
const cwd = remote ? remoteCwd : localCwd;
const marker = `MONITTER_${selected.toUpperCase().replaceAll('-', '_')}_${Date.now()}`;
const host = {
  id: remote ? 'mira' : 'local', name: remote ? 'Mira' : 'This Mac',
  kind: remote ? 'ssh' : 'local', address: remote ? 'mira' : '', user: '', port: 0,
  identityFile: '', defaultCwd: cwd,
  codexPath: remote ? '/home/alex/.npm-global/bin/codex' : '/Users/alex/.local/bin/codex',
  opencodePath: '', hermesPath: '',
};
if (selected === 'error') host.codexPath = '/nonexistent/monitter-deliberate-missing-codex';
const prompt = cancel
  ? `Use your shell tool to execute sleep 20 in the current folder, then reply ${marker}. Do not change any files or run any other commands.`
  : `Remember this marker for my next message: ${marker}. Reply with exactly ${marker}. Do not use tools or change files.`;
const args = ['--state-dir', root, '--cwd', cwd, '--host-json', JSON.stringify(host), '--prompt', prompt];
if (cancel) args.push('--cancel', '--expect-status', 'interrupted');
else if (selected === 'error') args.push('--expect-status', 'error');
else args.push('--second-prompt', 'Reply with exactly the marker I asked you to remember in my previous message. Do not use tools or change files.');
const executable = process.env.MONITTER_SMOKE_BINARY || resolve('src-tauri/target/debug/monitter-smoke');
const startedAt = new Date().toISOString();
const child = spawn(executable, args, { stdio: ['ignore', 'pipe', 'pipe'] });
let stdout = '', stderr = '';
child.stdout.on('data', chunk => { stdout += chunk; });
child.stderr.on('data', chunk => { stderr += chunk; process.stderr.write(chunk); });
// The service enforces its own cancellation; this deadline only prevents a hung test.
const timer = setTimeout(() => child.kill('SIGTERM'), 420_000);
let exitCode;
try {
  exitCode = await new Promise((resolveExit, reject) => {
    child.once('error', reject);
    child.once('exit', resolveExit);
  });
} finally { clearTimeout(timer); }
await writeFile(`${root}/stdout.txt`, stdout);
await writeFile(`${root}/stderr.txt`, stderr);
let result;
try { result = JSON.parse(stdout.trim()); }
catch { throw new Error(`Smoke output was not JSON. Inspect ${root}/stdout.txt (exit ${exitCode}).`); }
const evidence = { case: selected, startedAt, finishedAt: new Date().toISOString(), marker, exitCode, result };
if (selected === 'cancel-mira') {
  // Read only processes whose cwd is our isolated test folder; never scan command arguments.
  const script = `import json, pathlib, sys\nfound=[]\nfor p in pathlib.Path('/proc').iterdir():\n if not p.name.isdigit(): continue\n try:\n  if str((p/'cwd').resolve()) == sys.argv[1]:\n   found.append({'pid':int(p.name),'command':(p/'comm').read_text().strip()})\n except (OSError, PermissionError): pass\nprint(json.dumps(found))`;
  const quote = text => `'${text.replaceAll("'", "'\\''")}'`;
  const command = `python3 -c ${quote(script)} ${quote(remoteCwd)}`;
  const check = await promisify(execFile)('/usr/bin/ssh', ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', 'mira', command], { timeout: 20_000 });
  evidence.remainingRemoteProcesses = JSON.parse(check.stdout.trim());
}
await writeFile(`${root}/evidence.json`, `${JSON.stringify(evidence, null, 2)}\n`);
console.log(JSON.stringify({ ...evidence, evidencePath: `${root}/evidence.json` }, null, 2));
assert.equal(exitCode, 0, 'Native smoke process failed');
assert.equal(result.ok, true, 'Native service smoke checks failed');
assert.equal(result.persisted, true, 'Task failed persistence check');
assert.equal(result.finalStatus, cancel ? 'interrupted' : selected === 'error' ? 'error' : 'completed');
if (selected === 'cancel-mira') assert.deepEqual(evidence.remainingRemoteProcesses, [], 'A remote process survived cancellation in the dedicated test workspace');
if (!cancel && selected !== 'error') {
  assert(result.nativeSessionId, 'Native session ID must exist');
  assert(result.outputCount >= 2, 'Both turns must produce assistant output');
  assert((result.lastAssistantText || result.message || '').includes(marker), 'Follow-up did not recall the first-turn marker');
}
