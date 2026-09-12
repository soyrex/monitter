import { strict as assert } from 'node:assert';
import { createServer } from 'vite';

const base = { id: 'known', title: 'Terminal', autoTitle: null, customTitle: null, hostId: 'local', cwd: '/tmp', status: 'running', exitCode: null };
let terminalList = [base];
let releaseList;
let deferList = false;
globalThis.window = {
  __MONITTER_TEST_BRIDGE__: {
    invoke(command) {
      if (command === 'list_terminals') return deferList ? new Promise(resolve => { releaseList = () => resolve(terminalList); }) : Promise.resolve(terminalList);
      if (command === 'close_terminal') return Promise.resolve();
      throw new Error(`Unexpected terminal bridge command: ${command}`);
    },
    listen: async () => () => {},
  },
};

const vite = await createServer({ server: { middlewareMode: true }, appType: 'custom' });
try {
  const runtime = await vite.ssrLoadModule('/src/lib/terminal-runtime.ts');
  runtime.registerTerminal(base);

  // A native read can expose its snapshot as exited while retaining chunks;
  // the read status remains authoritative until the final drain request.
  runtime.terminalRuntimeTesting.applyRead('known', {
    chunks: [{ seq: 1, data: [111, 107] }], nextSeq: 1, status: 'running', exitCode: null,
    truncated: false, session: { ...base, title: 'Terminal: zsh', autoTitle: 'Terminal: zsh', status: 'exited', exitCode: 0 },
  });
  assert.equal(runtime.terminalStatus('known').status, 'running');
  assert.equal(runtime.terminalStatus('known').title, 'Terminal: zsh');

  // Older LAN peers omit the session projection; their read still advances
  // status without dropping the last known title.
  runtime.terminalRuntimeTesting.applyRead('known', {
    chunks: [], nextSeq: 2, status: 'running', exitCode: null, truncated: false,
  });
  assert.equal(runtime.terminalStatus('known').status, 'running');
  assert.equal(runtime.terminalStatus('known').title, 'Terminal: zsh');

  // A list refresh applies title fields to known sessions only, never imports
  // a different window's terminal.
  terminalList = [
    { ...base, title: 'Terminal: sleep', autoTitle: 'Terminal: sleep' },
    { ...base, id: 'unrelated', title: 'Terminal: private', autoTitle: 'Terminal: private' },
  ];
  await runtime.terminalRuntimeTesting.refresh();
  assert.equal(runtime.terminalStatus('known').title, 'Terminal: sleep');
  assert.equal(runtime.terminalStatus('unrelated'), undefined);

  // A stale in-flight list cannot revive a runtime closed while its request
  // was pending: refresh captures its eligible runtime identity before await.
  deferList = true;
  releaseList = undefined;
  const pending = runtime.terminalRuntimeTesting.refresh();
  while (!releaseList) await new Promise(resolve => setTimeout(resolve, 0));
  runtime.terminalRuntimeTesting.discard('known');
  releaseList();
  deferList = false;
  await pending;
  assert.equal(runtime.terminalStatus('known'), undefined);
  console.log('terminal runtime title regression: drain compatibility and stale-list isolation passed');
} finally {
  await vite.close();
}
