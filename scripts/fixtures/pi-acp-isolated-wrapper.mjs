// Test-only: run the installed ACP adapter against a fake Pi RPC process and
// private session store, never the user's Pi configuration, auth or executable.
import os from 'node:os';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { pathToFileURL } from 'node:url';

const scratch = process.env.MONITTER_PI_SMOKE_DIR;
const fixture = process.env.PI_ACP_PI_COMMAND;
const entry = process.env.MONITTER_PI_ACP_ENTRY;
if (!scratch || !fixture || !entry) throw new Error('Missing isolated Pi smoke configuration.');
os.homedir = () => scratch;
const spawn = childProcess.spawn;
childProcess.spawn = (command, ...args) => {
  if (command !== fixture) throw new Error('Real subprocesses are forbidden in this smoke test.');
  return spawn(command, ...args);
};
// pi-acp's startup banner probes npm for updates. Keep that offline as well.
childProcess.spawnSync = () => ({ status: 1, stdout: '', stderr: '', output: [] });
syncBuiltinESMExports();
globalThis.fetch = () => { throw new Error('Network access is forbidden in this smoke test.'); };
await import(pathToFileURL(entry).href);
