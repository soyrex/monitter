// Imported by a scratch copy of mock.mjs, never by a live Codex installation.
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
assert.deepEqual(process.argv.slice(2), ['app-server', '--listen', 'stdio://']);
const record = (value) => fs.appendFileSync(path.join(root, 'rpc.jsonl'), `${JSON.stringify(value)}\n`);
record({ pid: process.pid, cwd: process.cwd(), hasToken: Boolean(process.env.MONITTER_TOKEN) });
if (process.env.MONITTER_TOKEN) {
  const endpoint = new URL(process.env.MONITTER_ENDPOINT);
  assert.equal(endpoint.hostname, '127.0.0.1');
  const result = await fetch(endpoint, {
    method: 'POST', headers: { Authorization: `Bearer ${process.env.MONITTER_TOKEN}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ tool: 'list_agents', arguments: {} }), signal: AbortSignal.timeout(3000),
  });
  assert.equal(result.status, 200, 'Reverse tunnel must reach the real task-scoped broker');
  record({ brokerOk: true });
}
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on('line', (line) => {
  const rpc = JSON.parse(line);
  record(rpc);
  if (rpc.method === 'thread/start' || rpc.method === 'thread/resume') {
    assert.equal(fs.realpathSync(rpc.params.cwd), process.cwd());
    if (process.env.MONITTER_TOKEN) {
      const helper = rpc.params.config['mcp_servers.monitter.args'][0];
      assert.match(helper, /^\/tmp\/monitter-mcp\.[a-zA-Z0-9]+\/monitter_mcp\.py$/);
      fs.accessSync(helper);
    }
  }
  if (rpc.method === 'turn/start') assert.equal(fs.realpathSync(rpc.params.cwd), process.cwd());
});
