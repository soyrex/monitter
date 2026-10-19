import { spawn } from 'node:child_process';
import { once } from 'node:events';
const root = process.cwd();
let server;
let serverExit;
async function run(script, env = {}) {
  const child = spawn(process.execPath, [script], { cwd: root, env: { ...process.env, ...env }, stdio: 'inherit' });
  const [code, signal] = await once(child, 'exit');
  if (code !== 0) throw new Error(`${script} exited ${code ?? signal}`);
}
try {
  await run('scripts/command-contract.mjs');
  for (const script of ['activity-notice-test', 'transcript-notices-test', 'watch-projection-test', 'watch-interjection-test', 'project-ideas-test', 'question-drafts-test']) await run(`scripts/${script}.mjs`);
  let output = '';
  server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', '18449', '--strictPort'], { cwd: root, env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
  serverExit = once(server, 'exit');
  const url = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Vite startup timed out: ${output.slice(-3000)}`)), 60000);
    const read = chunk => { output += chunk; const match = output.match(/Local:\s+(http:\/\/[^\s]+)/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on('data', read); server.stderr.on('data', read);
    server.once('error', error => { clearTimeout(timer); reject(error); });
    server.once('exit', code => { clearTimeout(timer); reject(new Error(`Vite exited ${code}: ${output.slice(-3000)}`)); });
  });
  for (const engine of ['webkit', 'chromium']) await run('scripts/mobile-interface-parity-ui-test.mjs', { MONITTER_TEST_URL: url, MONITTER_TEST_ENGINE: engine });
  await run('scripts/project-ideas-ui-test.mjs');
  await run('scripts/run-activity-process-history-isolated.mjs');
  await run('scripts/task-transcript-notices-isolated.mjs');
  await run('scripts/harness-input-ui-test.mjs');
  console.log('Mobile interface parity verification passed.');
} catch (error) { console.error(error); process.exitCode = 1; }
finally {
  if (server && server.exitCode === null && server.signalCode === null) {
    server.kill('SIGTERM');
    await serverExit;
  }
}
