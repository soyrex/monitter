import assert from 'node:assert/strict';
import {
  copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';

const sourceRoot = process.cwd();
const checker = 'scripts/command-contract.mjs';
const fixture = mkdtempSync(join(tmpdir(), 'monitter-command-contract-'));

function copy(relativePath, recursive = false) {
  const from = join(sourceRoot, relativePath);
  const to = join(fixture, relativePath);
  mkdirSync(dirname(to), { recursive: true });
  if (recursive) cpSync(from, to, { recursive: true });
  else copyFileSync(from, to);
}

function runChecker(cwd = fixture) {
  return spawnSync(process.execPath, [join(cwd, checker)], {
    cwd,
    encoding: 'utf8',
    timeout: 15_000,
  });
}

function replaceOnce(path, before, after) {
  const original = readFileSync(join(fixture, path), 'utf8');
  const index = original.indexOf(before);
  assert.notEqual(index, -1, `fixture source is missing expected anchor in ${path}`);
  assert.equal(original.indexOf(before, index + before.length), -1, `fixture anchor is ambiguous in ${path}`);
  writeFileSync(join(fixture, path), `${original.slice(0, index)}${after}${original.slice(index + before.length)}`);
  return () => writeFileSync(join(fixture, path), original);
}

function rejectsDrift(label, restore, expectedMessage) {
  try {
    const result = runChecker();
    assert.notEqual(result.status, 0, `${label}: checker unexpectedly accepted drift\n${result.stdout}`);
    assert.match(result.stderr, expectedMessage, `${label}: unexpected checker failure\n${result.stderr}`);
    console.log(`rejected ${label}`);
  } finally {
    restore();
  }
}

function drift(label, path, before, after, expectedMessage) {
  const restore = replaceOnce(path, before, after);
  rejectsDrift(label, restore, expectedMessage);
}

try {
  for (const path of [
    'command-contract.json',
    checker,
    'src/lib/generated-command-contract.ts',
    'src/lib/bridge.ts',
    'src/lib/types.ts',
    'src/lib/controller/protocol.ts',
    'src/lib/controller/dispatcher.ts',
    'src/lib/operator-sharing.ts',
    'src-tauri/permissions/default.toml',
  ]) copy(path);
  copy('src-tauri/src', true);

  const baseline = runChecker();
  assert.equal(baseline.status, 0, `fixture baseline failed\n${baseline.stdout}\n${baseline.stderr}`);
  const direct = runChecker(sourceRoot);
  assert.equal(direct.status, 0, `working-tree checker baseline failed\n${direct.stdout}\n${direct.stderr}`);
  console.log(baseline.stdout.trim());

  // These integer widths both map to TypeScript number. The checker must still
  // compare their Rust signatures exactly instead of trusting that projection.
  drift(
    'Rust u64-to-u32 signature drift with unchanged TypeScript number',
    'src-tauri/src/lib.rs',
    'async fn read_terminal(\n    state: State<\'_, AppState>,\n    id: String,\n    after_seq: u64,',
    'async fn read_terminal(\n    state: State<\'_, AppState>,\n    id: String,\n    after_seq: u32,',
    /manifest is stale|read_terminal Rust signature differs from manifest/,
  );

  drift(
    'native ACL loss',
    'src-tauri/permissions/default.toml',
    '  "read_terminal", "close_terminal",',
    '  "close_terminal",',
    /native ACL vs manifest differ/,
  );

  drift(
    'undeclared LAN command exposure',
    'src-tauri/src/lib.rs',
    '            "read_terminal" => {',
    '            "plan_jev_route" => { unreachable!() }\n            "read_terminal" => {',
    /LAN dispatcher vs manifest surface differ/,
  );

  drift(
    'Rust/TypeScript structural member drift',
    'src/lib/types.ts',
    'export interface Host {',
    'export interface Host {\n  contractProbe: string;',
    /Host Rust\/TypeScript members differ/,
  );

  drift(
    'stale generated TypeScript',
    'src/lib/generated-command-contract.ts',
    'export const COMMAND_CONTRACT_PROTOCOL_VERSION =',
    '// stale fixture\nexport const COMMAND_CONTRACT_PROTOCOL_VERSION =',
    /generated TypeScript is stale/,
  );

  drift(
    'controller dispatcher action drift',
    'src/lib/controller/dispatcher.ts',
    "case 'cancelTask':",
    "case 'cancelTaskChanged':",
    /controller protocol actions differ from the manifest/,
  );

  drift(
    'visitor denied-action mapping drift',
    'src/lib/operator-sharing.ts',
    'cancelTask: denied, resumeTask: denied, listTerminals: denied, readTerminal: denied,',
    'cancelTask: async () => undefined, resumeTask: denied, listTerminals: denied, readTerminal: denied,',
    /visitor must keep cancelTask denied/,
  );

  console.log('Command contract negative tests passed (7 drift cases).');
} finally {
  if (existsSync(fixture)) rmSync(fixture, { recursive: true, force: true });
}
