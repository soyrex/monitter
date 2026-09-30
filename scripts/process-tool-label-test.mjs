import assert from 'node:assert/strict';
import { processToolLabel } from '../src/lib/activity-grouping.ts';

const event = (title, detail) => ({ id: 'test', taskId: 'task', kind: 'tool', title, detail, createdAt: 0 });

assert.equal(processToolLabel(event('Run a command', '{"type":"command_execution","command":"git status --short"}')), 'git status --short');
assert.equal(processToolLabel(event('/bin/zsh -lc pwd', '')), '/bin/zsh -lc pwd');
assert.equal(processToolLabel(event('ACP tool activity', JSON.stringify({ update: { sessionUpdate: 'tool_call', kind: 'execute', rawInput: { command: 'rg --files src' } } }))), 'rg --files src');
assert.equal(processToolLabel(event('fileChange', JSON.stringify({ type: 'fileChange', changes: [{ path: '/Users/alex/code/monitter/docs/CONTRACT.md', diff: '' }] }))), 'Edit: docs/CONTRACT.md');
assert.equal(processToolLabel(event('ACP tool activity', JSON.stringify({ update: { sessionUpdate: 'tool_call', kind: 'edit', rawInput: { file_path: '/repo/src/lib/example.ts' } } }))), 'Edit: lib/example.ts');
assert.equal(processToolLabel(event('apply_patch', JSON.stringify({ patch: '*** Begin Patch\n*** Update File: docs/CONTRACT.md\n*** End Patch' }))), 'Edit: docs/CONTRACT.md');

console.log('Turn tree command and edit labels passed.');
